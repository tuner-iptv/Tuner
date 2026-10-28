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
    /// `None` for an mpv we don't own (libmpv embedded in the GUI, reached
    /// through its IPC server); dropping that only stops its stream.
    child: Option<Child>,
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

        Self::attach(Some(child), stream, socket)
    }

    /// Connect to the IPC server of an mpv that is already running, such as
    /// libmpv with `input-ipc-server` set.
    pub fn connect(socket: PathBuf) -> Result<Self> {
        let stream = UnixStream::connect(&socket)?;
        Self::attach(None, stream, socket)
    }

    fn attach(child: Option<Child>, stream: UnixStream, socket: PathBuf) -> Result<Self> {
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
        self.child
            .as_mut()
            .is_none_or(|c| matches!(c.try_wait(), Ok(None)))
    }

    pub fn command(&mut self, args: &[Value]) -> Result<Value> {
        self.conn.command(args)
    }

    pub fn load(&mut self, url: &str) -> Result<()> {
        self.load_with(url, &[])
    }

    /// Like [`Mpv::load`], with per-file options such as `start=754.500` or
    /// `aid=2` (a stream moved here from another player; see
    /// [`Mpv::handoff`]). They apply to this file only. mpv before 0.38 has
    /// no per-file options in `loadfile`; it rejects them and the file plays
    /// with its defaults from the beginning.
    pub fn load_with(&mut self, url: &str, options: &[String]) -> Result<()> {
        // A window kept for the previous stream stays while the next one
        // connects only if that one had video; an audio-only one's closes.
        if self.window_forced && self.child.is_some() {
            self.set_force_window(false)?;
        }
        let plain = [json!("loadfile"), json!(url), json!("replace")];
        let mut resuming = false;
        let reply = if options.is_empty() {
            self.command(&plain)?
        } else {
            let mut with = plain.to_vec();
            with.extend([json!(-1), json!(options.join(","))]);
            match self.command(&with) {
                Err(Error::Command(_)) => self.command(&plain)?,
                other => {
                    resuming = options.iter().any(|o| o.starts_with("start="));
                    other?
                }
            }
        };
        // mpv starts the file only after replying, so everything queued so
        // far is about the previous one.
        self.conn.take_events();
        let entry = reply.get("playlist_entry_id").and_then(Value::as_i64);
        self.watch.begin(entry, Instant::now());
        if resuming {
            self.watch.resuming();
        }
        Ok(())
    }

    /// What the stream playing here should keep in another player, as
    /// per-file options for [`Mpv::load_with`]: the position (a movie or
    /// episode; a live channel has no duration), the audio and subtitle
    /// track, volume, mute, and any audio or subtitle delay. Track ids carry
    /// over because the other player opens the same stream.
    pub fn handoff(&mut self) -> Vec<String> {
        let mut get = |name: &str| {
            self.command(&[json!("get_property"), json!(name)])
                .unwrap_or(Value::Null)
        };
        let (duration, pos) = (get("duration"), get("time-pos"));
        let (aid, sid) = (get("aid"), get("sid"));
        let (volume, mute) = (get("volume"), get("mute"));
        let (audio_delay, sub_delay) = (get("audio-delay"), get("sub-delay"));

        let finite = |v: &Value| v.as_f64().filter(|n| n.is_finite());
        let mut options = Vec::new();
        if finite(&duration).is_some_and(|d| d > 0.0)
            && let Some(pos) = finite(&pos).filter(|p| *p > 0.0)
        {
            options.push(format!("start={pos:.3}"));
        }
        for (name, track) in [("aid", aid), ("sid", sid)] {
            match track {
                Value::Number(id) => options.push(format!("{name}={id}")),
                // Turned off ("no" in older mpv, false in newer).
                Value::Bool(false) => options.push(format!("{name}=no")),
                Value::String(s) if s == "no" => options.push(format!("{name}=no")),
                _ => {}
            }
        }
        if let Some(v) = finite(&volume) {
            options.push(format!("volume={v}"));
        }
        if let Some(muted) = mute.as_bool() {
            options.push(format!("mute={}", if muted { "yes" } else { "no" }));
        }
        for (name, delay) in [("audio-delay", audio_delay), ("sub-delay", sub_delay)] {
            if let Some(d) = finite(&delay).filter(|d| *d != 0.0) {
                options.push(format!("{name}={d}"));
            }
        }
        options
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
        if self.watch.stage() == Some(LoadStage::Playing)
            && !self.window_forced
            && self.child.is_some()
        {
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
        let Some(child) = &mut self.child else {
            // Not ours to quit, but its stream must not outlive us (it holds
            // a provider connection).
            let _ = self.conn.command(&[json!("stop")]);
            return;
        };
        if matches!(child.try_wait(), Ok(None)) {
            let _ = self.conn.command(&[json!("quit")]);
            let deadline = Instant::now() + Duration::from_secs(1);
            while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
                thread::sleep(POLL_INTERVAL);
            }
            let _ = child.kill();
        }
        let _ = child.wait();
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
        self.play_with(url, &[])
    }

    /// [`Player::play`] with per-file options; see [`Mpv::load_with`].
    pub fn play_with(&mut self, url: &str, options: &[String]) -> Result<()> {
        match self.running()?.load_with(url, options) {
            Err(e) if e.is_disconnect() => {
                self.mpv = None;
                self.running()?.load_with(url, options)
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

/// A fresh path for an mpv IPC socket in `$XDG_RUNTIME_DIR`.
pub fn socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    // Add the time so a stale socket from a reused PID never collides.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    dir.join(format!("tuner-mpv-{}-{nanos}.sock", std::process::id()))
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    use super::*;

    /// A fake mpv on a socket: answers each command with `reply(command)`
    /// and returns every command it got once the client hangs up.
    fn fake_mpv(
        reply: impl Fn(&Value) -> Value + Send + 'static,
    ) -> (PathBuf, thread::JoinHandle<Vec<Value>>) {
        let socket = std::env::temp_dir().join(format!(
            "tuner-fake-mpv-{}-{:?}.sock",
            std::process::id(),
            thread::current().id()
        ));
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).unwrap();
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut out = stream.try_clone().unwrap();
            let mut seen = Vec::new();
            for line in BufReader::new(stream).lines() {
                let Ok(line) = line else { break };
                let msg: Value = serde_json::from_str(&line).unwrap();
                let mut answer = reply(&msg["command"]);
                answer["request_id"] = msg["request_id"].clone();
                writeln!(out, "{answer}").unwrap();
                seen.push(msg["command"].clone());
            }
            seen
        });
        (socket, handle)
    }

    fn loadfiles(seen: &[Value]) -> Vec<&Value> {
        seen.iter().filter(|c| c[0] == "loadfile").collect()
    }

    fn options(list: &[&str]) -> Vec<String> {
        list.iter().map(|o| o.to_string()).collect()
    }

    #[test]
    fn loads_with_per_file_options() {
        let (socket, fake) = fake_mpv(|_| json!({"error": "success", "data": null}));
        let mut mpv = Mpv::connect(socket.clone()).unwrap();
        mpv.load_with("http://h/movie.mkv", &options(&["start=754.500", "aid=2"]))
            .unwrap();
        drop(mpv);
        let seen = fake.join().unwrap();
        assert_eq!(
            loadfiles(&seen),
            [&json!([
                "loadfile",
                "http://h/movie.mkv",
                "replace",
                -1,
                "start=754.500,aid=2"
            ])]
        );
        let _ = std::fs::remove_file(socket);
    }

    #[test]
    fn older_mpv_without_per_file_options_starts_from_the_beginning() {
        let (socket, fake) = fake_mpv(|c| {
            if c[0] == "loadfile" && c.as_array().is_some_and(|a| a.len() > 3) {
                json!({"error": "invalid parameter"})
            } else {
                json!({"error": "success", "data": null})
            }
        });
        let mut mpv = Mpv::connect(socket.clone()).unwrap();
        mpv.load_with("http://h/movie.mkv", &options(&["start=60.000"]))
            .unwrap();
        drop(mpv);
        let seen = fake.join().unwrap();
        let loads = loadfiles(&seen);
        assert_eq!(loads.len(), 2);
        assert_eq!(
            loads[1],
            &json!(["loadfile", "http://h/movie.mkv", "replace"])
        );
        let _ = std::fs::remove_file(socket);
    }

    /// A fake mpv answering `get_property` from `props`.
    fn playing(props: Value) -> (PathBuf, thread::JoinHandle<Vec<Value>>) {
        fake_mpv(
            move |c| match c[1].as_str().and_then(|name| props.get(name)) {
                Some(value) => json!({"error": "success", "data": value}),
                None if c[0] == "get_property" => json!({"error": "property unavailable"}),
                None => json!({"error": "success", "data": null}),
            },
        )
    }

    #[test]
    fn a_movie_hands_off_position_tracks_and_audio_settings() {
        let (socket, fake) = playing(json!({
            "duration": 7200.0, "time-pos": 754.5, "aid": 2, "sid": 3,
            "volume": 80.0, "mute": false, "audio-delay": 0.0, "sub-delay": -0.5,
        }));
        let mut mpv = Mpv::connect(socket.clone()).unwrap();
        assert_eq!(
            mpv.handoff(),
            options(&[
                "start=754.500",
                "aid=2",
                "sid=3",
                "volume=80",
                "mute=no",
                "sub-delay=-0.5"
            ])
        );
        drop(mpv);
        fake.join().unwrap();
        let _ = std::fs::remove_file(socket);
    }

    #[test]
    fn a_live_channel_hands_off_tracks_but_no_position() {
        // No duration; subtitles off (false in newer mpv).
        let (socket, fake) = playing(json!({
            "time-pos": 42.0, "aid": 1, "sid": false, "volume": 100.0, "mute": true,
        }));
        let mut mpv = Mpv::connect(socket.clone()).unwrap();
        assert_eq!(
            mpv.handoff(),
            options(&["aid=1", "sid=no", "volume=100", "mute=yes"])
        );
        drop(mpv);
        fake.join().unwrap();
        let _ = std::fs::remove_file(socket);
    }
}
