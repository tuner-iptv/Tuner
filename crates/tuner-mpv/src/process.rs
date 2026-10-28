use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::load::{CACHE_PROPERTY, LoadProgress, LoadStage, LoadWatch, StreamEnd};
use crate::{Connection, Error, Result};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// How to launch mpv.
#[derive(Debug, Clone)]
pub struct MpvOptions {
    /// Executable name or path.
    pub program: String,
    /// Extra arguments appended after tuner's own (user overrides win).
    pub extra_args: Vec<String>,
}

impl Default for MpvOptions {
    fn default() -> Self {
        Self {
            program: "mpv".into(),
            extra_args: Vec::new(),
        }
    }
}

/// One running mpv process plus its IPC connection. The process is asked to
/// quit (and killed if needed) on drop.
pub struct Mpv {
    child: Child,
    conn: Connection,
    socket: PathBuf,
    watch: LoadWatch,
    /// `force-window` was switched on after playback started.
    window_forced: bool,
}

impl Mpv {
    pub fn spawn(opts: &MpvOptions) -> Result<Self> {
        let socket = socket_path();
        let _ = std::fs::remove_file(&socket);

        let mut child = Command::new(&opts.program)
            .arg("--idle=yes")
            // No window until the first frame: an unreachable stream never
            // shows a black window. `poll` forces one once playback starts,
            // for audio-only streams.
            .arg("--force-window=no")
            .arg(format!("--input-ipc-server={}", socket.display()))
            .arg("--title=tuner: ${media-title}")
            // Stable app-id/class so window-manager rules can target it.
            .arg("--wayland-app-id=tuner-player")
            .arg("--x11-name=tuner-player")
            // Give up on a server that goes silent for 10 s (mpv's default
            // is 60 s), with mpv's own reason.
            .arg("--network-timeout=10")
            // No yt-dlp fallback: IPTV links never need it, and after a
            // failed open it re-requests the stream URL (another provider
            // connection) and adds its own 20 s timeout.
            .arg("--ytdl=no")
            .args(&opts.extra_args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|source| Error::Spawn {
                program: opts.program.clone(),
                source,
            })?;

        let deadline = Instant::now() + STARTUP_TIMEOUT;
        let stream = loop {
            if let Ok(stream) = UnixStream::connect(&socket) {
                break stream;
            }
            if child.try_wait()?.is_some() {
                return Err(Error::ExitedEarly);
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::SocketTimeout(socket));
            }
            thread::sleep(POLL_INTERVAL);
        };

        let mut mpv = Self {
            child,
            conn: Connection::new(stream)?,
            socket,
            watch: LoadWatch::default(),
            window_forced: false,
        };
        // Data reaching the cache, to tell a slow stream from a dead one.
        mpv.command(&[json!("observe_property"), json!(1), json!(CACHE_PROPERTY)])?;
        // ffmpeg's warnings and errors, for a precise failure reason.
        mpv.command(&[json!("request_log_messages"), json!("warn")])?;
        Ok(mpv)
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub fn command(&mut self, args: &[Value]) -> Result<Value> {
        self.conn.command(args)
    }

    pub fn load(&mut self, url: &str) -> Result<()> {
        // A window kept for the previous stream stays while the next one
        // connects only if that one had video; an audio-only one's closes.
        if self.window_forced {
            self.set_force_window(false)?;
        }
        let reply = self.command(&[json!("loadfile"), json!(url), json!("replace")])?;
        // mpv starts the file only after replying, so everything queued so
        // far is about the previous one.
        self.conn.take_events();
        let entry = reply.get("playlist_entry_id").and_then(Value::as_i64);
        self.watch.begin(entry, Instant::now());
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        self.watch.clear();
        self.command(&[json!("stop")]).map(drop)
    }

    /// Without blocking, check whether the last loaded stream failed, never
    /// started, or ended. Reported once.
    pub fn poll(&mut self) -> Option<StreamEnd> {
        // A broken socket shows up as a closed player; nothing to add here.
        let _ = self.conn.poll();
        let now = Instant::now();
        for event in self.conn.take_events() {
            if let Some(end) = self.watch.on_event(&event, now) {
                return Some(end);
            }
        }
        if self.watch.stage() == Some(LoadStage::Playing) && !self.window_forced {
            // Video already opened the window; an audio-only stream needs
            // one so the user can see and close the player. Keeping it also
            // avoids the window closing and reopening on every zap.
            let _ = self.set_force_window(true);
        }
        self.watch.check(now)
    }

    fn set_force_window(&mut self, on: bool) -> Result<()> {
        self.command(&[json!("set_property"), json!("force-window"), json!(on)])?;
        self.window_forced = on;
        Ok(())
    }

    /// Where the last loaded stream is, as of the last [`Mpv::poll`].
    pub fn stage(&self) -> Option<LoadStage> {
        self.watch.stage()
    }

    /// Stage, elapsed time and cache fill of the last loaded stream, as of
    /// the last [`Mpv::poll`].
    pub fn progress(&self) -> Option<LoadProgress> {
        self.watch.progress(Instant::now())
    }

    pub fn toggle_pause(&mut self) -> Result<()> {
        self.command(&[json!("cycle"), json!("pause")]).map(drop)
    }

    /// Whether playback is paused (also if the user paused inside mpv).
    pub fn is_paused(&mut self) -> Result<bool> {
        let value = self.command(&[json!("get_property"), json!("pause")])?;
        Ok(value.as_bool().unwrap_or(false))
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        if self.is_running() {
            let _ = self.conn.command(&[json!("quit")]);
            let deadline = Instant::now() + Duration::from_secs(1);
            while self.is_running() && Instant::now() < deadline {
                thread::sleep(POLL_INTERVAL);
            }
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket);
    }
}

/// Lazily spawns mpv on first use and respawns it if the user closed the
/// window. This is what the UI talks to.
pub struct Player {
    opts: MpvOptions,
    mpv: Option<Mpv>,
}

impl Player {
    pub fn new(opts: MpvOptions) -> Self {
        Self { opts, mpv: None }
    }

    /// Play `url`, starting (or restarting) mpv if needed.
    pub fn play(&mut self, url: &str) -> Result<()> {
        match self.running()?.load(url) {
            Err(e) if e.is_disconnect() => {
                self.mpv = None;
                self.running()?.load(url)
            }
            other => other,
        }
    }

    /// Whether mpv is paused; `None` when mpv is not running or not answering.
    pub fn is_paused(&mut self) -> Option<bool> {
        let mpv = self.mpv.as_mut()?;
        if !mpv.is_running() {
            return None;
        }
        mpv.is_paused().ok()
    }

    /// Whether the stream from the last `play` failed, stalled, or ended.
    /// Non-blocking; call about once a second. `None` when mpv is not
    /// running.
    pub fn poll(&mut self) -> Option<StreamEnd> {
        let mpv = self.mpv.as_mut()?;
        if !mpv.is_running() {
            return None;
        }
        mpv.poll()
    }

    /// Where the stream from the last `play` is (stage, elapsed time, cache
    /// fill), as of the last [`Player::poll`].
    pub fn progress(&self) -> Option<LoadProgress> {
        self.mpv.as_ref().and_then(Mpv::progress)
    }

    /// Quit mpv (closing its window) if it is running.
    pub fn shutdown(&mut self) {
        self.mpv = None;
    }

    /// Whether an mpv process is alive (the user has not closed its window).
    pub fn is_running(&mut self) -> bool {
        self.mpv.as_mut().is_some_and(Mpv::is_running)
    }

    /// Stop playback. A no-op when mpv is not running.
    pub fn stop(&mut self) -> Result<()> {
        self.with_existing(Mpv::stop)
    }

    /// Toggle pause. A no-op when mpv is not running.
    pub fn toggle_pause(&mut self) -> Result<()> {
        self.with_existing(Mpv::toggle_pause)
    }

    fn running(&mut self) -> Result<&mut Mpv> {
        if !self.mpv.as_mut().is_some_and(Mpv::is_running) {
            self.mpv = None;
            self.mpv = Some(Mpv::spawn(&self.opts)?);
        }
        Ok(self.mpv.as_mut().expect("just spawned"))
    }

    fn with_existing(&mut self, f: impl FnOnce(&mut Mpv) -> Result<()>) -> Result<()> {
        if !self.mpv.as_mut().is_some_and(Mpv::is_running) {
            self.mpv = None;
            return Ok(());
        }
        let Some(mpv) = self.mpv.as_mut() else {
            return Ok(());
        };
        match f(mpv) {
            Err(e) if e.is_disconnect() => {
                self.mpv = None;
                Ok(())
            }
            other => other,
        }
    }
}

fn socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    // Add the time so a stale socket from a reused PID never collides.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    dir.join(format!("tuner-mpv-{}-{nanos}.sock", std::process::id()))
}
