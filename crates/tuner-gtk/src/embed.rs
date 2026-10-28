//! Embedded video (cargo feature `embedded`, ADR 0008): libmpv runs
//! in-process and draws into a `gtk::GLArea` through its render API, with
//! tuner's own control bar over it.
//!
//! libmpv's own IPC server is switched on, so playback control, load
//! tracking and error reporting go through the same `tuner_mpv::Mpv` as the
//! external player. The C API is used to create mpv, draw it, and drive the
//! control bar (input commands, observed properties, the track list), which
//! makes this the one module allowed `unsafe`.
#![allow(unsafe_code)]

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::PathBuf;
use std::ptr;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use serde_json::Value;
use tuner_app::Playback;
use tuner_mpv::{LoadProgress, Mpv, MpvOptions, Player, StreamEnd};

use crate::window::Video;

#[repr(C)]
struct MpvHandle {
    _private: [u8; 0],
}

#[repr(C)]
struct RenderContext {
    _private: [u8; 0],
}

#[repr(C)]
struct RenderParam {
    kind: c_int,
    data: *mut c_void,
}

#[repr(C)]
struct GlInitParams {
    get_proc_address: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    get_proc_address_ctx: *mut c_void,
}

#[repr(C)]
struct GlFbo {
    fbo: c_int,
    w: c_int,
    h: c_int,
    internal_format: c_int,
}

#[repr(C)]
struct Event {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

#[repr(C)]
struct EventProperty {
    name: *const c_char,
    format: c_int,
    data: *mut c_void,
}

const PARAM_END: c_int = 0;
const PARAM_API_TYPE: c_int = 1;
const PARAM_OPENGL_INIT: c_int = 2;
const PARAM_OPENGL_FBO: c_int = 3;
const PARAM_FLIP_Y: c_int = 4;
const EVENT_NONE: c_int = 0;
const EVENT_PROPERTY_CHANGE: c_int = 22;
const FORMAT_FLAG: c_int = 3;
const FORMAT_DOUBLE: c_int = 5;
const GL_FRAMEBUFFER_BINDING: u32 = 0x8CA6;
/// glibc's value; tuner is Linux-only.
const LC_NUMERIC: c_int = 1;
/// The control bar hides after this long without pointer movement.
const CONTROLS_HIDE: Duration = Duration::from_millis(2500);

#[link(name = "mpv")]
unsafe extern "C" {
    fn mpv_create() -> *mut MpvHandle;
    fn mpv_set_option_string(h: *mut MpvHandle, name: *const c_char, data: *const c_char) -> c_int;
    fn mpv_initialize(h: *mut MpvHandle) -> c_int;
    fn mpv_set_property_string(
        h: *mut MpvHandle,
        name: *const c_char,
        data: *const c_char,
    ) -> c_int;
    fn mpv_terminate_destroy(h: *mut MpvHandle);
    fn mpv_command_string(h: *mut MpvHandle, args: *const c_char) -> c_int;
    fn mpv_get_property_string(h: *mut MpvHandle, name: *const c_char) -> *mut c_char;
    fn mpv_free(data: *mut c_void);
    fn mpv_observe_property(
        h: *mut MpvHandle,
        reply_userdata: u64,
        name: *const c_char,
        format: c_int,
    ) -> c_int;
    fn mpv_set_wakeup_callback(
        h: *mut MpvHandle,
        callback: Option<unsafe extern "C" fn(*mut c_void)>,
        d: *mut c_void,
    );
    fn mpv_wait_event(h: *mut MpvHandle, timeout: f64) -> *mut Event;
    fn mpv_error_string(error: c_int) -> *const c_char;
    fn mpv_render_context_create(
        res: *mut *mut RenderContext,
        h: *mut MpvHandle,
        params: *mut RenderParam,
    ) -> c_int;
    fn mpv_render_context_set_update_callback(
        ctx: *mut RenderContext,
        callback: Option<unsafe extern "C" fn(*mut c_void)>,
        callback_ctx: *mut c_void,
    );
    fn mpv_render_context_update(ctx: *mut RenderContext) -> u64;
    fn mpv_render_context_render(ctx: *mut RenderContext, params: *mut RenderParam) -> c_int;
    fn mpv_render_context_free(ctx: *mut RenderContext);
}

// shortcut: assumes GTK draws with EGL (its default on Wayland and X11);
// a GLX-only setup needs glXGetProcAddressARB here.
#[link(name = "EGL")]
unsafe extern "C" {
    fn eglGetProcAddress(name: *const c_char) -> *mut c_void;
}

unsafe extern "C" {
    fn setlocale(category: c_int, locale: *const c_char) -> *mut c_char;
}

/// The libmpv instance, shared by the video pane and every player made for
/// it; destroyed with the last of them.
struct Core(*mut MpvHandle);

// SAFETY: the mpv client API is thread-safe; only the render context (kept
// on the GTK thread in `video_area`) is not, and `mpv_wait_event` is only
// called on the GTK thread.
unsafe impl Send for Core {}
unsafe impl Sync for Core {}

impl Core {
    /// Run an mpv input command such as `cycle pause`; errors are ignored
    /// (a control that does nothing is its own feedback).
    fn send(&self, command: &str) {
        if let Ok(command) = CString::new(command) {
            // SAFETY: valid handle and NUL-terminated string.
            unsafe { mpv_command_string(self.0, command.as_ptr()) };
        }
    }

    fn get(&self, property: &CStr) -> Option<String> {
        // SAFETY: valid handle; the result is ours to free with `mpv_free`.
        unsafe {
            let raw = mpv_get_property_string(self.0, property.as_ptr());
            if raw.is_null() {
                return None;
            }
            let value = CStr::from_ptr(raw).to_string_lossy().into_owned();
            mpv_free(raw.cast());
            Some(value)
        }
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        // SAFETY: created by `mpv_create`; the render context, which must go
        // first, is freed on unrealize, before the area drops its `Arc`.
        unsafe { mpv_terminate_destroy(self.0) }
    }
}

fn check(code: c_int, what: &str) -> Result<(), String> {
    if code >= 0 {
        return Ok(());
    }
    // SAFETY: mpv returns a static string for every error code.
    let reason = unsafe { CStr::from_ptr(mpv_error_string(code)) };
    Err(format!("{what}: {}", reason.to_string_lossy()))
}

fn c_string(s: &str) -> Result<CString, String> {
    CString::new(s).map_err(|_| format!("NUL byte in mpv option {s:?}"))
}

/// Start libmpv with the same settings the external player gets (plus the
/// user's `mpv.conf` and mpv args) and return the video pane, hidden until
/// something plays, and a maker of players that draw into it.
pub fn start(
    opts: &MpvOptions,
) -> Result<(Video, impl Fn() -> Box<dyn Playback + Send> + 'static), String> {
    // libmpv refuses to start unless numbers are formatted the C way; GTK
    // has set the user's locale by now.
    // SAFETY: called on the main thread before any other thread uses locale.
    unsafe { setlocale(LC_NUMERIC, c"C".as_ptr()) };
    // SAFETY: plain constructor; null on failure.
    let handle = unsafe { mpv_create() };
    if handle.is_null() {
        return Err("libmpv could not be created".into());
    }
    let core = Arc::new(Core(handle));

    let socket = tuner_mpv::socket_path();
    let ours = [
        ("vo", "libmpv"),
        ("idle", "yes"),
        // Read the user's mpv.conf, as the external player does.
        ("config", "yes"),
        ("terminal", "no"),
        ("network-timeout", "10"),
        ("ytdl", "no"),
        // tuner draws its own controls; mpv's default mouse bindings
        // (double-click for fullscreen, wheel to seek) stay.
        ("osc", "no"),
        ("input-default-bindings", "yes"),
        ("input-ipc-server", &socket.to_string_lossy()),
    ]
    .map(|(k, v)| (k.to_string(), v.to_string()));
    // `--key=value`, `--flag` and `--no-flag`, as on mpv's command line.
    let user = opts.extra_args.iter().map(|arg| {
        let arg = arg.strip_prefix("--").unwrap_or(arg);
        match arg.split_once('=') {
            Some((k, v)) => (k.to_string(), v.to_string()),
            None => match arg.strip_prefix("no-") {
                Some(k) => (k.to_string(), "no".to_string()),
                None => (arg.to_string(), "yes".to_string()),
            },
        }
    });
    for (name, value) in ours.into_iter().chain(user) {
        let (k, v) = (c_string(&name)?, c_string(&value)?);
        // SAFETY: valid handle and NUL-terminated strings.
        check(
            unsafe { mpv_set_option_string(core.0, k.as_ptr(), v.as_ptr()) },
            &format!("mpv option {name}={value}"),
        )?;
    }
    // SAFETY: valid, not yet initialized handle.
    check(unsafe { mpv_initialize(core.0) }, "starting libmpv")?;
    // mpv.conf is read during initialize and overrides the options above. A
    // `vo=` line there would make libmpv open its own window inside this
    // process (it crashes GTK), and `input-ipc-server=` would move the
    // socket the players connect to, so set these again. Nothing has been
    // drawn yet, so changing them now is safe.
    for (name, value) in [
        ("vo", "libmpv"),
        ("idle", "yes"),
        ("osc", "no"),
        ("input-ipc-server", &*socket.to_string_lossy()),
    ] {
        let (k, v) = (c_string(name)?, c_string(value)?);
        // SAFETY: valid handle and NUL-terminated strings.
        check(
            unsafe { mpv_set_property_string(core.0, k.as_ptr(), v.as_ptr()) },
            &format!("mpv option {name}={value}"),
        )?;
    }

    let video = video_pane(&core);
    let detached = video.detached.clone();
    let detach_next = video.detach_next.clone();
    let opts = opts.clone();
    let make = move || -> Box<dyn Playback + Send> {
        Box::new(EmbeddedPlayer {
            core: core.clone(),
            socket: socket.clone(),
            mpv: None,
            external: Player::new(opts.clone()),
            detached: detached.clone(),
            detach_next: detach_next.clone(),
        })
    };
    Ok((video, make))
}

/// Plays in the embedded mpv over its IPC socket (connecting on first use),
/// or in mpv's own window after "Open in mpv".
struct EmbeddedPlayer {
    core: Arc<Core>,
    socket: PathBuf,
    /// Dropping it stops the stream (see `Mpv`'s `Drop`).
    mpv: Option<Mpv>,
    external: Player,
    /// The stream plays in mpv's own window; read by the GUI.
    detached: Arc<AtomicBool>,
    /// Set by the GUI: play the next stream in mpv's own window.
    detach_next: Arc<AtomicBool>,
}

impl EmbeddedPlayer {
    fn detached(&self) -> bool {
        self.detached.load(Ordering::Relaxed)
    }
}

impl Playback for EmbeddedPlayer {
    /// One stream at a time: the other player is stopped before this one
    /// starts, so moving the stream never needs a second connection.
    fn play(&mut self, url: &str) -> tuner_mpv::Result<()> {
        if self.detach_next.swap(false, Ordering::Relaxed) {
            self.detached.store(true, Ordering::Relaxed);
        }
        if self.detached() {
            self.core.send("set fullscreen no");
            // The stream keeps its tracks, volume and delays, and a movie
            // continues where it was; it is fetched once more, as the new
            // mpv cannot take over this one's connection.
            let handoff = self.mpv.as_mut().map(Mpv::handoff).unwrap_or_default();
            self.mpv = None;
            return self.external.play_with(url, &handoff);
        }
        self.external.shutdown();
        if let Some(mpv) = &mut self.mpv {
            match mpv.load(url) {
                Err(e) if e.is_disconnect() => self.mpv = None,
                other => return other,
            }
        }
        match Mpv::connect(self.socket.clone()) {
            Ok(mpv) => self.mpv.insert(mpv).load(url),
            // Something told libmpv to quit (MPRIS Quit, a script); it cannot
            // be restarted, so play in mpv's own window from now on.
            Err(e) => {
                eprintln!("tuner-gtk: embedded mpv is gone ({e}); using an mpv window");
                self.detached.store(true, Ordering::Relaxed);
                self.external.play(url)
            }
        }
    }
    fn toggle_pause(&mut self) -> tuner_mpv::Result<()> {
        if self.detached() {
            return self.external.toggle_pause();
        }
        self.mpv.as_mut().map_or(Ok(()), Mpv::toggle_pause)
    }
    /// The embedded view has no window to close; closing mpv's own window
    /// ends the detached stream and brings the next one back in.
    fn is_running(&mut self) -> bool {
        if self.detached() && !self.external.is_running() {
            self.detached.store(false, Ordering::Relaxed);
            return false;
        }
        true
    }
    fn shutdown(&mut self) {
        self.mpv = None;
        self.external.shutdown();
        self.detached.store(false, Ordering::Relaxed);
    }
    fn is_paused(&mut self) -> Option<bool> {
        if self.detached() {
            return self.external.is_paused();
        }
        self.mpv.as_mut()?.is_paused().ok()
    }
    fn poll_end(&mut self) -> Option<StreamEnd> {
        if self.detached() {
            return self.external.poll();
        }
        self.mpv.as_mut()?.poll()
    }
    fn progress(&self) -> Option<LoadProgress> {
        if self.detached() {
            return self.external.progress();
        }
        self.mpv.as_ref()?.progress()
    }
}

thread_local! {
    /// Drains the core's events on the GTK thread; set by `video_pane`.
    static ON_EVENTS: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
}

/// Called by mpv on any thread when events are queued, possibly inside an
/// mpv call on the GTK thread while mpv holds its lock: always defer (glib's
/// `invoke` would run at once there and deadlock in `mpv_wait_event`).
unsafe extern "C" fn on_wakeup(_: *mut c_void) {
    glib::idle_add_once(|| {
        ON_EVENTS.with(|f| {
            if let Some(f) = f.borrow().as_ref() {
                f();
            }
        });
    });
}

/// The control bar's widgets that follow mpv's state.
struct Controls {
    pane: gtk::Overlay,
    pause: gtk::Button,
    mute: gtk::Button,
    volume: gtk::Scale,
    seek: gtk::Scale,
    time: gtk::Label,
    fullscreen: gtk::Button,
    cinema: gtk::ToggleButton,
    /// Resolution, frame rate and bitrate badge, top left of the video.
    info: gtk::Label,
    width: Cell<Option<f64>>,
    height: Cell<Option<f64>>,
    fps: Cell<Option<f64>>,
    bitrate: Cell<Option<f64>>,
    /// mpv's `fullscreen` property.
    full: Cell<bool>,
    /// Widgets hidden for cinema mode or fullscreen, shown again after.
    hidden: RefCell<Vec<gtk::Widget>>,
    duration: Cell<Option<f64>>,
}

fn video_pane(core: &Arc<Core>) -> Video {
    let area = video_area(core.clone());
    forward_pointer(&area, core);

    let button = |icon: &str, tip: &str, command: &'static str| {
        let b = gtk::Button::from_icon_name(icon);
        b.set_tooltip_text(Some(tip));
        b.update_property(&[gtk::accessible::Property::Label(tip)]);
        let core = core.clone();
        b.connect_clicked(move |_| core.send(command));
        b
    };
    let pause = button("media-playback-pause-symbolic", "Pause", "cycle pause");
    let back = button("media-seek-backward-symbolic", "Back 10 s", "seek -10");
    let forward = button("media-seek-forward-symbolic", "Forward 10 s", "seek 10");
    let mute = button("audio-volume-high-symbolic", "Mute", "cycle mute");
    let fullscreen = button(
        "view-fullscreen-symbolic",
        "Fullscreen (F11)",
        "cycle fullscreen",
    );
    let cinema = gtk::ToggleButton::builder()
        .icon_name("video-single-display-symbolic")
        .tooltip_text("Cinema mode")
        .build();
    cinema.update_property(&[gtk::accessible::Property::Label("Cinema mode")]);
    let detach = gtk::Button::from_icon_name("window-new-symbolic");
    detach.set_tooltip_text(Some("Open in mpv window"));
    detach.update_property(&[gtk::accessible::Property::Label("Open in mpv window")]);

    let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 130.0, 1.0);
    volume.set_width_request(80);
    volume.set_draw_value(false);
    volume.set_tooltip_text(Some("Volume"));
    volume.connect_change_value({
        let core = core.clone();
        move |_, _, value| {
            core.send(&format!("set volume {}", value.clamp(0.0, 130.0)));
            glib::Propagation::Proceed
        }
    });
    let seek = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 1.0);
    seek.set_draw_value(false);
    seek.set_hexpand(true);
    seek.set_visible(false);
    seek.add_css_class("video-seek");
    seek.connect_change_value({
        let core = core.clone();
        move |_, _, value| {
            core.send(&format!("seek {value} absolute"));
            glib::Propagation::Proceed
        }
    });
    let time = gtk::Label::new(None);
    time.add_css_class("video-time");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);

    let tracks = track_menu(core);
    // mpv's own statistics overlay (stats.lua), drawn into the video.
    let info = gtk::Label::builder()
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .visible(false)
        .build();
    info.add_css_class("video-info");
    let show_info = gtk::ToggleButton::builder()
        .icon_name("view-reveal-symbolic")
        .tooltip_text("Stream info: resolution, frame rate, bitrate")
        .build();
    show_info.update_property(&[gtk::accessible::Property::Label("Stream info")]);
    show_info
        .bind_property("active", &info, "visible")
        .sync_create()
        .build();
    let stats = gtk::ToggleButton::builder()
        .icon_name("network-cellular-signal-excellent-symbolic")
        .tooltip_text("Playback statistics")
        .build();
    stats.update_property(&[gtk::accessible::Property::Label("Playback statistics")]);
    stats.connect_toggled({
        let core = core.clone();
        move |_| core.send("script-binding stats/display-stats-toggle")
    });

    // Seek bar across the top; below it playback, volume and time on the
    // left, everything about the view on the right.
    pause.add_css_class("video-play");
    volume.add_css_class("video-volume");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    for w in [
        pause.upcast_ref::<gtk::Widget>(),
        back.upcast_ref(),
        forward.upcast_ref(),
        mute.upcast_ref(),
        volume.upcast_ref(),
        time.upcast_ref(),
        spacer.upcast_ref(),
        tracks.upcast_ref(),
        show_info.upcast_ref(),
        stats.upcast_ref(),
        detach.upcast_ref(),
        cinema.upcast_ref(),
        fullscreen.upcast_ref(),
    ] {
        row.append(w);
    }
    let bar = gtk::Box::new(gtk::Orientation::Vertical, 0);
    bar.add_css_class("video-controls");
    bar.append(&seek);
    bar.append(&row);
    let revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::Crossfade)
        .valign(gtk::Align::End)
        .child(&bar)
        .build();

    let pane = gtk::Overlay::builder().child(&area).visible(false).build();
    pane.add_css_class("video");
    pane.add_overlay(&revealer);
    pane.add_overlay(&info);
    autohide(&pane, &revealer, tracks);

    let controls = Rc::new(Controls {
        pane: pane.clone(),
        pause,
        mute,
        volume,
        seek,
        time,
        fullscreen,
        cinema,
        info,
        width: Cell::new(None),
        height: Cell::new(None),
        fps: Cell::new(None),
        bitrate: Cell::new(None),
        full: Cell::new(false),
        hidden: RefCell::default(),
        duration: Cell::new(None),
    });
    let weak = Rc::downgrade(&controls);
    controls.cinema.connect_toggled(move |_| {
        if let Some(c) = weak.upgrade() {
            apply_view(&c);
        }
    });
    // Nothing left to watch (stopped, or moved to mpv's window): leave
    // cinema mode and fullscreen, which would otherwise show an empty window.
    let weak = Rc::downgrade(&controls);
    let quit_core = core.clone();
    pane.connect_visible_notify(move |pane| {
        let Some(c) = weak.upgrade() else {
            return;
        };
        if !pane.is_visible() {
            c.cinema.set_active(false);
            if c.full.get() {
                quit_core.send("set fullscreen no");
            }
        }
    });
    for (name, format) in [
        (c"pause", FORMAT_FLAG),
        (c"mute", FORMAT_FLAG),
        (c"fullscreen", FORMAT_FLAG),
        (c"volume", FORMAT_DOUBLE),
        (c"duration", FORMAT_DOUBLE),
        (c"time-pos", FORMAT_DOUBLE),
        // What mpv's stats overlay shows on its first page.
        (c"video-params/w", FORMAT_DOUBLE),
        (c"video-params/h", FORMAT_DOUBLE),
        (c"container-fps", FORMAT_DOUBLE),
        (c"video-bitrate", FORMAT_DOUBLE),
    ] {
        // SAFETY: valid handle and NUL-terminated name.
        unsafe { mpv_observe_property(core.0, 0, name.as_ptr(), format) };
    }
    let weak = Arc::downgrade(core);
    let state = controls.clone();
    ON_EVENTS.with(|f| *f.borrow_mut() = Some(Box::new(move || drain(&weak, &state))));
    // SAFETY: valid handle; `on_wakeup` only schedules work on GTK's thread.
    unsafe { mpv_set_wakeup_callback(core.0, Some(on_wakeup), ptr::null_mut()) };
    window_keys(&pane, core, &controls);

    Video {
        widget: pane.upcast(),
        detach,
        detached: Arc::default(),
        detach_next: Arc::default(),
    }
}

/// Apply mpv's queued property changes to the control bar.
fn drain(core: &Weak<Core>, c: &Controls) {
    let Some(core) = core.upgrade() else {
        return;
    };
    loop {
        // SAFETY: valid handle; only this (GTK) thread waits for events.
        let event = unsafe { &*mpv_wait_event(core.0, 0.0) };
        match event.event_id {
            EVENT_NONE => break,
            EVENT_PROPERTY_CHANGE => {
                // SAFETY: property-change events carry an `mpv_event_property`
                // whose data has the observed format (or format 0, no data).
                let p = unsafe { &*(event.data as *const EventProperty) };
                let name = unsafe { CStr::from_ptr(p.name) }.to_bytes();
                let flag =
                    (p.format == FORMAT_FLAG).then(|| unsafe { *p.data.cast::<c_int>() } != 0);
                let num = (p.format == FORMAT_DOUBLE).then(|| unsafe { *p.data.cast::<f64>() });
                on_property(c, name, flag, num);
            }
            _ => {}
        }
    }
}

fn on_property(c: &Controls, name: &[u8], flag: Option<bool>, num: Option<f64>) {
    match name {
        b"pause" => {
            let paused = flag == Some(true);
            let (icon, tip) = if paused {
                ("media-playback-start-symbolic", "Play")
            } else {
                ("media-playback-pause-symbolic", "Pause")
            };
            c.pause.set_icon_name(icon);
            c.pause.set_tooltip_text(Some(tip));
        }
        b"mute" => c.mute.set_icon_name(if flag == Some(true) {
            "audio-volume-muted-symbolic"
        } else {
            "audio-volume-high-symbolic"
        }),
        b"volume" => {
            if let Some(v) = num {
                c.volume.set_value(v);
            }
        }
        b"fullscreen" => {
            c.full.set(flag == Some(true));
            apply_view(c);
        }
        // Live streams have no duration: no seek bar, just the time played.
        b"duration" => {
            let duration = num.filter(|d| d.is_finite() && *d > 0.0);
            c.duration.set(duration);
            c.seek.set_visible(duration.is_some());
            if let Some(d) = duration {
                c.seek.set_range(0.0, d);
            }
        }
        b"video-params/w" | b"video-params/h" | b"container-fps" | b"video-bitrate" => {
            let value = num.filter(|v| v.is_finite() && *v > 0.0);
            match name {
                b"video-params/w" => c.width.set(value),
                b"video-params/h" => c.height.set(value),
                b"container-fps" => c.fps.set(value),
                _ => c.bitrate.set(value),
            }
            c.info.set_text(&stream_info(
                c.width.get().zip(c.height.get()),
                c.fps.get(),
                c.bitrate.get(),
            ));
        }
        b"time-pos" => {
            let Some(pos) = num else {
                c.time.set_text("");
                return;
            };
            c.seek.set_value(pos);
            c.time.set_text(&match c.duration.get() {
                Some(d) => format!("{} / {}", clock(pos), clock(d)),
                None => clock(pos),
            });
        }
        _ => {}
    }
}

/// `1920×1080 · 25 fps · 4.2 Mb/s`, leaving out what mpv doesn't know yet
/// (bitrate takes a second to measure; audio-only streams have no size).
fn stream_info(size: Option<(f64, f64)>, fps: Option<f64>, bitrate: Option<f64>) -> String {
    let mut parts = Vec::new();
    if let Some((w, h)) = size {
        parts.push(format!("{w:.0}×{h:.0}"));
    }
    if let Some(fps) = fps {
        // 25 fps, 29.97 fps
        let fps = format!("{fps:.2}");
        parts.push(format!(
            "{} fps",
            fps.trim_end_matches('0').trim_end_matches('.')
        ));
    }
    if let Some(bps) = bitrate {
        parts.push(if bps >= 1e6 {
            format!("{:.1} Mb/s", bps / 1e6)
        } else {
            format!("{:.0} kb/s", bps / 1e3)
        });
    }
    if parts.is_empty() {
        "No stream info yet".into()
    } else {
        parts.join(" · ")
    }
}

/// `1:02:03` or `2:03`.
fn clock(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Show the window as cinema mode and fullscreen say. Both hide every
/// sibling on the way from the pane up to the window (lists, panels, bars),
/// so the video fills it; cinema mode keeps the header and the window as it
/// is, fullscreen hides the header too and fullscreens the window.
fn apply_view(c: &Controls) {
    let Some(window) = c.pane.root().and_downcast::<gtk::Window>() else {
        return;
    };
    let (full, cinema) = (c.full.get(), c.cinema.is_active());
    let (icon, tip) = if full {
        ("view-restore-symbolic", "Leave fullscreen (Esc)")
    } else {
        ("view-fullscreen-symbolic", "Fullscreen (F11)")
    };
    c.fullscreen.set_icon_name(icon);
    c.fullscreen.set_tooltip_text(Some(tip));

    let mut hidden = c.hidden.borrow_mut();
    for w in hidden.drain(..) {
        w.set_visible(true);
    }
    if full || cinema {
        let header = window.titlebar();
        let mut path: gtk::Widget = c.pane.clone().upcast();
        while let Some(parent) = path.parent() {
            let mut child = parent.first_child();
            while let Some(w) = child {
                let keep = w == path || (!full && Some(&w) == header.as_ref());
                if !keep && w.is_visible() {
                    w.set_visible(false);
                    hidden.push(w.clone());
                }
                child = w.next_sibling();
            }
            path = parent;
        }
    }
    if full != window.is_fullscreen() {
        if full {
            window.fullscreen();
        } else {
            window.unfullscreen();
        }
    }
}

/// F11 toggles fullscreen while the video shows; in fullscreen, Esc leaves
/// it and Space pauses.
/// In cinema mode Esc leaves it. A window fullscreen ended by the
/// compositor also ends mpv's.
fn window_keys(pane: &gtk::Overlay, core: &Arc<Core>, c: &Rc<Controls>) {
    let done = Cell::new(false);
    let core = core.clone();
    let c = Rc::downgrade(c);
    pane.connect_realize(move |pane| {
        let Some(window) = pane.root().and_downcast::<gtk::Window>() else {
            return;
        };
        if done.replace(true) {
            return;
        }
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed({
            let (core, c) = (core.clone(), c.clone());
            move |_, key, _, _| {
                let Some(c) = c.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                let full = c.full.get();
                let command = match key {
                    // Nothing to show fullscreen while no video plays.
                    gtk::gdk::Key::F11 if full || c.pane.is_visible() => "cycle fullscreen",
                    gtk::gdk::Key::Escape if full => "set fullscreen no",
                    gtk::gdk::Key::space if full => "cycle pause",
                    gtk::gdk::Key::Escape if c.cinema.is_active() => {
                        c.cinema.set_active(false);
                        return glib::Propagation::Stop;
                    }
                    _ => return glib::Propagation::Proceed,
                };
                core.send(command);
                glib::Propagation::Stop
            }
        });
        window.add_controller(keys);
        let core = core.clone();
        let c = c.clone();
        window.connect_fullscreened_notify(move |w| {
            let ours = c.upgrade().is_some_and(|c| c.full.get());
            if ours && !w.is_fullscreen() {
                core.send("set fullscreen no");
            }
        });
    });
}

/// Show the control bar while the pointer moves over the video and for a
/// moment after; keep it while the track menu is open.
fn autohide(pane: &gtk::Overlay, bar: &gtk::Revealer, menu: gtk::MenuButton) {
    let generation = Rc::new(Cell::new(0u64));
    let motion = gtk::EventControllerMotion::new();
    let bar = bar.clone();
    motion.connect_motion(move |_, _, _| {
        bar.set_reveal_child(true);
        let current = generation.get() + 1;
        generation.set(current);
        let (bar, generation, menu) = (bar.clone(), generation.clone(), menu.clone());
        glib::timeout_add_local_once(CONTROLS_HIDE, move || {
            if generation.get() == current && !menu.is_active() {
                bar.set_reveal_child(false);
            }
        });
    });
    pane.add_controller(motion);
}

/// "Audio & subtitles": the stream's audio and subtitle tracks side by
/// side, read from mpv's `track-list` each time the menu opens.
fn track_menu(core: &Arc<Core>) -> gtk::MenuButton {
    let columns = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    let popover = gtk::Popover::builder().child(&columns).build();
    let menu = gtk::MenuButton::builder()
        .icon_name("media-view-subtitles-symbolic")
        .tooltip_text("Audio & subtitles")
        .popover(&popover)
        .direction(gtk::ArrowType::Up)
        // Right edges aligned, so the wide menu opens leftwards, over the
        // video, instead of past the window's edge.
        .halign(gtk::Align::End)
        .build();
    menu.add_css_class("video-tracks");
    let core = core.clone();
    popover.connect_show(move |popover| {
        while let Some(child) = columns.first_child() {
            columns.remove(&child);
        }
        let tracks: Vec<Value> = core
            .get(c"track-list")
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        for (kind, heading) in [("audio", "Audio"), ("sub", "Subtitles")] {
            let column = track_column(&core, popover, &tracks, kind, heading);
            columns.append(&column);
        }
    });
    menu
}

/// One column of the track menu: a heading and a row per track, the
/// selected one checked. Subtitles start with "Off".
fn track_column(
    core: &Arc<Core>,
    popover: &gtk::Popover,
    tracks: &[Value],
    kind: &str,
    heading: &str,
) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let title = gtk::Label::builder().label(heading).xalign(0.0).build();
    title.add_css_class("section-label");
    column.append(&title);
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.add_css_class("track-list");
    column.append(&list);

    let property = if kind == "sub" { "sid" } else { "aid" };
    let ours: Vec<&Value> = tracks.iter().filter(|t| t["type"] == kind).collect();
    let mut rows: Vec<(String, String, String, bool)> = Vec::new();
    if kind == "sub" {
        let off = !ours.iter().any(|t| t["selected"] == true);
        rows.push(("Off".into(), String::new(), "no".into(), off));
    }
    for t in &ours {
        let id = t["id"].as_i64().unwrap_or_default();
        let (name, detail) = track_label(t, id);
        rows.push((name, detail, id.to_string(), t["selected"] == true));
    }
    if ours.is_empty() && kind == "audio" {
        let none = gtk::Label::new(Some("No audio"));
        none.add_css_class("dim-label");
        column.append(&none);
    }

    let mut values = Vec::new();
    for (name, detail, value, selected) in rows {
        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_hexpand(true);
        let name = gtk::Label::builder().label(&name).xalign(0.0).build();
        name.add_css_class("track-name");
        text.append(&name);
        if !detail.is_empty() {
            let detail = gtk::Label::builder().label(&detail).xalign(0.0).build();
            detail.add_css_class("dim-label");
            detail.add_css_class("track-detail");
            text.append(&detail);
        }
        let check = gtk::Image::from_icon_name("object-select-symbolic");
        check.set_opacity(if selected { 1.0 } else { 0.0 });
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        line.append(&text);
        line.append(&check);
        let row = gtk::ListBoxRow::builder().child(&line).build();
        if selected {
            row.add_css_class("current");
        }
        list.append(&row);
        values.push(value);
    }
    let (core, popover) = (core.clone(), popover.clone());
    list.connect_row_activated(move |_, row| {
        if let Some(value) = usize::try_from(row.index())
            .ok()
            .and_then(|i| values.get(i))
        {
            core.send(&format!("set {property} {value}"));
        }
        popover.popdown();
    });
    column
}

/// A track's name and a detail line: `("English", "Director's cut · AC3 ·
/// 5.1 · Default")`. The name falls back to the title, then `Track 2`.
fn track_label(track: &Value, id: i64) -> (String, String) {
    let text = |key: &str| track[key].as_str().filter(|s| !s.is_empty());
    let lang = text("lang").map(language);
    let title = text("title");
    let name = lang
        .clone()
        .or(title.map(str::to_string))
        .unwrap_or_else(|| format!("Track {id}"));
    let mut detail: Vec<String> = Vec::new();
    if let (Some(_), Some(title)) = (&lang, title) {
        detail.push(title.to_string());
    }
    if let Some(codec) = text("codec") {
        detail.push(codec.to_uppercase());
    }
    match track["demux-channel-count"].as_i64() {
        Some(1) => detail.push("Mono".into()),
        Some(2) => detail.push("Stereo".into()),
        Some(6) => detail.push("5.1".into()),
        Some(8) => detail.push("7.1".into()),
        Some(n) if n > 0 => detail.push(format!("{n} ch")),
        _ => {}
    }
    for (flag, word) in [
        ("default", "Default"),
        ("forced", "Forced"),
        ("external", "External"),
    ] {
        if track[flag] == true {
            detail.push(word.into());
        }
    }
    (name, detail.join(" · "))
}

/// English name of an ISO 639 language code (`tur`, `tr`); unknown codes
/// are shown as they are, upper-cased.
fn language(code: &str) -> String {
    const NAMES: &[(&[&str], &str)] = &[
        (&["en", "eng"], "English"),
        (&["tr", "tur"], "Turkish"),
        (&["de", "ger", "deu"], "German"),
        (&["fr", "fre", "fra"], "French"),
        (&["es", "spa"], "Spanish"),
        (&["it", "ita"], "Italian"),
        (&["pt", "por"], "Portuguese"),
        (&["nl", "dut", "nld"], "Dutch"),
        (&["ru", "rus"], "Russian"),
        (&["uk", "ukr"], "Ukrainian"),
        (&["pl", "pol"], "Polish"),
        (&["el", "gre", "ell"], "Greek"),
        (&["ar", "ara"], "Arabic"),
        (&["fa", "per", "fas"], "Persian"),
        (&["he", "heb"], "Hebrew"),
        (&["hi", "hin"], "Hindi"),
        (&["ja", "jpn"], "Japanese"),
        (&["ko", "kor"], "Korean"),
        (&["zh", "chi", "zho"], "Chinese"),
        (&["sv", "swe"], "Swedish"),
        (&["no", "nor", "nob"], "Norwegian"),
        (&["da", "dan"], "Danish"),
        (&["fi", "fin"], "Finnish"),
        (&["ro", "rum", "ron"], "Romanian"),
        (&["hu", "hun"], "Hungarian"),
        (&["cs", "cze", "ces"], "Czech"),
        (&["sr", "srp"], "Serbian"),
        (&["hr", "hrv"], "Croatian"),
        (&["bg", "bul"], "Bulgarian"),
        (&["sq", "alb", "sqi"], "Albanian"),
        (&["ku", "kur"], "Kurdish"),
        (&["az", "aze"], "Azerbaijani"),
        (&["und"], "Unknown language"),
    ];
    let lower = code.to_ascii_lowercase();
    NAMES
        .iter()
        .find(|(codes, _)| codes.contains(&lower.as_str()))
        .map_or_else(|| code.to_uppercase(), |(_, name)| name.to_string())
}

/// The render context, alive while the area is realized.
struct Render {
    ctx: *mut RenderContext,
    /// Passed to mpv's update callback; must outlive `ctx`.
    waker: *mut glib::SendWeakRef<gtk::GLArea>,
}

impl Drop for Render {
    fn drop(&mut self) {
        // SAFETY: the area's GL context is current (unrealize); after
        // `free` mpv calls no more callbacks, so the waker can go.
        unsafe {
            mpv_render_context_free(self.ctx);
            drop(Box::from_raw(self.waker));
        }
    }
}

unsafe extern "C" fn proc_address(_: *mut c_void, name: *const c_char) -> *mut c_void {
    // SAFETY: mpv passes a NUL-terminated GL function name.
    unsafe { eglGetProcAddress(name) }
}

/// Called by mpv on its own thread when a new frame is ready.
unsafe extern "C" fn on_update(waker: *mut c_void) {
    // SAFETY: `waker` is the `Render::waker` box, alive until after free.
    let area = unsafe { &*(waker as *const glib::SendWeakRef<gtk::GLArea>) }.clone();
    glib::idle_add_once(move || {
        if let Some(area) = area.upgrade() {
            area.queue_render();
        }
    });
}

fn video_area(core: Arc<Core>) -> gtk::GLArea {
    let area = gtk::GLArea::builder()
        .hexpand(true)
        .vexpand(true)
        .height_request(180)
        .build();
    let render: Rc<RefCell<Option<Render>>> = Rc::default();

    area.connect_realize({
        let render = render.clone();
        let core = core.clone();
        move |area| {
            area.make_current();
            if let Some(e) = area.error() {
                eprintln!("tuner-gtk: no OpenGL for embedded video: {e}");
                return;
            }
            let mut init = GlInitParams {
                get_proc_address: proc_address,
                get_proc_address_ctx: ptr::null_mut(),
            };
            let mut params = [
                RenderParam {
                    kind: PARAM_API_TYPE,
                    data: c"opengl".as_ptr() as *mut c_void,
                },
                RenderParam {
                    kind: PARAM_OPENGL_INIT,
                    data: (&raw mut init).cast(),
                },
                RenderParam {
                    kind: PARAM_END,
                    data: ptr::null_mut(),
                },
            ];
            let mut ctx = ptr::null_mut();
            // SAFETY: GL context current, params end with PARAM_END.
            let code = unsafe { mpv_render_context_create(&mut ctx, core.0, params.as_mut_ptr()) };
            if let Err(e) = check(code, "creating the mpv render context") {
                eprintln!("tuner-gtk: {e}");
                return;
            }
            let waker = Box::into_raw(Box::new(glib::SendWeakRef::from(area.downgrade())));
            // SAFETY: `waker` stays valid until `Render`'s drop frees `ctx`.
            unsafe { mpv_render_context_set_update_callback(ctx, Some(on_update), waker.cast()) };
            *render.borrow_mut() = Some(Render { ctx, waker });
        }
    });

    area.connect_unrealize({
        let render = render.clone();
        move |area| {
            // Capturing the core keeps libmpv alive as long as this area.
            let _ = &core;
            area.make_current();
            render.borrow_mut().take();
        }
    });

    area.connect_render(move |area, _| {
        let Some(r) = render.borrow().as_ref().map(|r| r.ctx) else {
            return glib::Propagation::Proceed;
        };
        let mut fbo: c_int = 0;
        // SAFETY: glGetIntegerv's signature; GTK made the context current.
        unsafe {
            let get = eglGetProcAddress(c"glGetIntegerv".as_ptr());
            if !get.is_null() {
                let get: unsafe extern "C" fn(u32, *mut c_int) = std::mem::transmute(get);
                get(GL_FRAMEBUFFER_BINDING, &mut fbo);
            }
        }
        let scale = area.scale_factor();
        let mut target = GlFbo {
            fbo,
            w: area.width() * scale,
            h: area.height() * scale,
            internal_format: 0,
        };
        let mut flip: c_int = 1;
        let mut params = [
            RenderParam {
                kind: PARAM_OPENGL_FBO,
                data: (&raw mut target).cast(),
            },
            RenderParam {
                kind: PARAM_FLIP_Y,
                data: (&raw mut flip).cast(),
            },
            RenderParam {
                kind: PARAM_END,
                data: ptr::null_mut(),
            },
        ];
        // SAFETY: valid context on this thread; params end with PARAM_END.
        unsafe {
            mpv_render_context_update(r);
            mpv_render_context_render(r, params.as_mut_ptr());
        }
        glib::Propagation::Stop
    });

    area
}

/// Pass pointer moves, clicks and scrolls to mpv (double-click toggles
/// fullscreen, the wheel seeks). Coordinates are in the render target's
/// pixels.
fn forward_pointer(area: &gtk::GLArea, core: &Arc<Core>) {
    let motion = gtk::EventControllerMotion::new();
    motion.connect_motion({
        let core = core.clone();
        move |m, x, y| {
            let scale = f64::from(m.widget().map_or(1, |w| w.scale_factor()));
            core.send(&format!(
                "mouse {} {}",
                (x * scale) as i32,
                (y * scale) as i32
            ));
        }
    });
    motion.connect_leave({
        let core = core.clone();
        move |_| core.send("keypress MOUSE_LEAVE")
    });
    area.add_controller(motion);

    let click = gtk::GestureClick::builder().button(0).build();
    let button = |g: &gtk::GestureClick| match g.current_button() {
        2 => "MBTN_MID",
        3 => "MBTN_RIGHT",
        _ => "MBTN_LEFT",
    };
    click.connect_pressed({
        let core = core.clone();
        move |g, _, _, _| core.send(&format!("keydown {}", button(g)))
    });
    click.connect_released({
        let core = core.clone();
        move |g, _, _, _| core.send(&format!("keyup {}", button(g)))
    });
    area.add_controller(click);

    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    scroll.connect_scroll({
        let core = core.clone();
        move |_, _, dy| {
            core.send(if dy < 0.0 {
                "keypress WHEEL_UP"
            } else {
                "keypress WHEEL_DOWN"
            });
            glib::Propagation::Stop
        }
    });
    area.add_controller(scroll);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quit_embedded_mpv_falls_back_to_mpvs_window() {
        // An uninitialized handle stands in for a core that quit: nothing
        // listens on the socket.
        // SAFETY: plain constructor; `Core`'s drop destroys it.
        let core = Arc::new(Core(unsafe { mpv_create() }));
        let detached = Arc::new(AtomicBool::new(false));
        let mut player = EmbeddedPlayer {
            core,
            socket: std::env::temp_dir().join("tuner-no-such-mpv.sock"),
            mpv: None,
            // Never starts: the test needs no mpv binary.
            external: Player::new(MpvOptions {
                program: "/nonexistent/mpv".into(),
                extra_args: Vec::new(),
            }),
            detached: detached.clone(),
            detach_next: Arc::default(),
        };
        let err = player.play("http://h/live").unwrap_err();
        assert!(matches!(err, tuner_mpv::Error::Spawn { .. }), "{err}");
        assert!(detached.load(Ordering::Relaxed));
    }

    #[test]
    fn stream_info_reads_like_mpvs_stats() {
        assert_eq!(
            stream_info(Some((1920.0, 1080.0)), Some(25.0), Some(4_230_000.0)),
            "1920×1080 · 25 fps · 4.2 Mb/s"
        );
        assert_eq!(
            stream_info(Some((1280.0, 720.0)), Some(29.97), Some(850_000.0)),
            "1280×720 · 29.97 fps · 850 kb/s"
        );
        assert_eq!(stream_info(None, None, Some(128_000.0)), "128 kb/s");
        assert_eq!(stream_info(None, None, None), "No stream info yet");
    }

    #[test]
    fn clock_formats_hours_only_when_needed() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(123.9), "2:03");
        assert_eq!(clock(3723.0), "1:02:03");
        assert_eq!(clock(-1.0), "0:00");
    }

    #[test]
    fn track_labels_name_the_language_and_fall_back_to_the_number() {
        let t = serde_json::json!({"title": "Director", "lang": "eng", "codec": "ac3",
                                   "demux-channel-count": 6, "default": true});
        assert_eq!(
            track_label(&t, 1),
            ("English".into(), "Director · AC3 · 5.1 · Default".into())
        );
        let t = serde_json::json!({"lang": "tur", "codec": "mov_text"});
        assert_eq!(track_label(&t, 2), ("Turkish".into(), "MOV_TEXT".into()));
        let t = serde_json::json!({"title": "Commentary"});
        assert_eq!(track_label(&t, 3), ("Commentary".into(), String::new()));
        let t = serde_json::json!({"title": ""});
        assert_eq!(track_label(&t, 4), ("Track 4".into(), String::new()));
    }

    #[test]
    fn unknown_language_codes_are_shown_as_they_are() {
        assert_eq!(language("TR"), "Turkish");
        assert_eq!(language("xyz"), "XYZ");
    }
}
