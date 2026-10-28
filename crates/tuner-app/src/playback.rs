//! The play / stop / stream-limit flow shared by every front-end, so the TUI
//! and the GUI enforce limits identically. Front-ends only render
//! [`Request`] outcomes (e.g. as a popup) and call back with the user's
//! choice. See `docs/stream-limits.md`.

use std::time::Duration;

use tuner_mpv::{LoadProgress, LoadStage};

use crate::guard::{Begin, StreamGate};

/// What the front-ends need from a player. Implemented for
/// [`tuner_mpv::Player`]; tests use fakes.
pub trait Playback {
    fn play(&mut self, url: &str) -> tuner_mpv::Result<()>;
    fn toggle_pause(&mut self) -> tuner_mpv::Result<()>;
    /// False once the user has closed the player window.
    fn is_running(&mut self) -> bool;
    /// Close the player entirely (e.g. the front-end is exiting).
    fn shutdown(&mut self) {}
    /// Whether playback is paused, if the player can tell.
    fn is_paused(&mut self) -> Option<bool> {
        None
    }
    /// Non-blocking: whether the stream from the last `play` failed, never
    /// started, or ended. Reported once.
    fn poll_end(&mut self) -> Option<tuner_mpv::StreamEnd> {
        None
    }
    /// Where the stream from the last `play` is (stage, elapsed time, cache
    /// fill), as of the last `poll_end`.
    fn progress(&self) -> Option<LoadProgress> {
        None
    }
}

impl Playback for tuner_mpv::Player {
    fn play(&mut self, url: &str) -> tuner_mpv::Result<()> {
        tuner_mpv::Player::play(self, url)
    }
    fn toggle_pause(&mut self) -> tuner_mpv::Result<()> {
        tuner_mpv::Player::toggle_pause(self)
    }
    fn is_running(&mut self) -> bool {
        tuner_mpv::Player::is_running(self)
    }
    fn shutdown(&mut self) {
        tuner_mpv::Player::shutdown(self)
    }
    fn is_paused(&mut self) -> Option<bool> {
        tuner_mpv::Player::is_paused(self)
    }
    fn poll_end(&mut self) -> Option<tuner_mpv::StreamEnd> {
        tuner_mpv::Player::poll(self)
    }
    fn progress(&self) -> Option<LoadProgress> {
        tuner_mpv::Player::progress(self)
    }
}

impl<T: Playback + ?Sized> Playback for Box<T> {
    fn play(&mut self, url: &str) -> tuner_mpv::Result<()> {
        (**self).play(url)
    }
    fn toggle_pause(&mut self) -> tuner_mpv::Result<()> {
        (**self).toggle_pause()
    }
    fn is_running(&mut self) -> bool {
        (**self).is_running()
    }
    fn shutdown(&mut self) {
        (**self).shutdown()
    }
    fn is_paused(&mut self) -> Option<bool> {
        (**self).is_paused()
    }
    fn poll_end(&mut self) -> Option<tuner_mpv::StreamEnd> {
        (**self).poll_end()
    }
    fn progress(&self) -> Option<LoadProgress> {
        (**self).progress()
    }
}

impl<T: StreamGate + ?Sized> StreamGate for Box<T> {
    fn begin(&mut self) -> Begin {
        (**self).begin()
    }
    fn end(&mut self) {
        (**self).end()
    }
    fn watch(&mut self) -> Option<String> {
        (**self).watch()
    }
}

/// Outcome of [`Controller::request`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// The channel is playing.
    Started,
    /// The stream limit may be exceeded. Show the message and call
    /// [`Controller::confirm`] (user said yes) or [`Controller::cancel`].
    NeedsConfirm(String),
    /// The stream limit would be exceeded. Show the message; nothing opened.
    Blocked(String),
    /// The player failed; nothing is playing (or the previous channel is).
    Failed(String),
}

/// Something [`Controller::tick`] noticed. In every case playback has
/// stopped and the stream slot is released.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tick {
    /// The user closed the player window.
    Closed,
    /// The stream could not be opened or never started (unreachable server,
    /// dead link, …). The player was closed; show the message.
    Failed(String),
    /// The stream played and then ended. The player was closed.
    Ended,
}

/// Owns the player and the stream gate and keeps them consistent: a stream
/// slot is held exactly while something plays (or a confirm is pending).
/// Channels are identified by the caller's index (into its channel list).
pub struct Controller<P, G> {
    player: P,
    gate: G,
    playing: Option<usize>,
}

impl<P: Playback, G: StreamGate> Controller<P, G> {
    pub fn new(player: P, gate: G) -> Self {
        Self {
            player,
            gate,
            playing: None,
        }
    }

    pub fn playing(&self) -> Option<usize> {
        self.playing
    }

    /// Ask to play channel `index` at `url`. Switching channels while
    /// something plays reuses the slot (mpv replaces the stream), so the
    /// limit is only checked when starting from a stopped state.
    pub fn request(&mut self, index: usize, url: &str) -> Request {
        if self.playing.is_some() {
            return self.start(index, url);
        }
        match self.gate.begin() {
            Begin::Go => self.start(index, url),
            Begin::Confirm(message) => Request::NeedsConfirm(message),
            Begin::Blocked(message) => Request::Blocked(message),
        }
    }

    /// The user accepted a [`Request::NeedsConfirm`].
    pub fn confirm(&mut self, index: usize, url: &str) -> Request {
        self.start(index, url)
    }

    /// The user declined a [`Request::NeedsConfirm`].
    pub fn cancel(&mut self) {
        if self.playing.is_none() {
            self.gate.end();
        }
    }

    /// Stop playback by quitting mpv (its window closes), and release the
    /// stream slot. Quitting rather than just stopping the stream means no
    /// idle player is left behind.
    pub fn stop(&mut self) {
        self.player.shutdown();
        self.playing = None;
        self.gate.end();
    }

    /// Switch accounts: close the old stream and release its slot before
    /// installing the new account's gate. The next play checks the new limit.
    pub fn replace_gate(&mut self, gate: G) {
        self.stop();
        self.gate = gate;
    }

    /// Whether the playing stream is paused; `None` when nothing plays or the
    /// player can't tell.
    pub fn is_paused(&mut self) -> Option<bool> {
        self.playing?;
        self.player.is_paused()
    }

    pub fn toggle_pause(&mut self) -> Result<(), String> {
        self.player.toggle_pause().map_err(|e| e.to_string())
    }

    /// Close the player and release the stream slot. Call when the
    /// front-end exits.
    pub fn shutdown(&mut self) {
        self.player.shutdown();
        self.playing = None;
        self.gate.end();
    }

    /// Call about once a minute while playing: asks the provider whether the
    /// account is over its limit (another device joined). Returns a warning
    /// to show, if any. Does a network request; run it off the UI thread
    /// where possible.
    pub fn watch(&mut self) -> Option<String> {
        self.playing?;
        self.gate.watch()
    }

    /// Where the playing stream is (stage, elapsed time, cache fill), as of
    /// the last [`Controller::tick`]. `None` when nothing plays or the player
    /// can't tell. Show it with [`status_line`] so a slow start isn't a
    /// silent wait.
    pub fn progress(&self) -> Option<LoadProgress> {
        self.playing?;
        self.player.progress()
    }

    /// Call about once a second. Reports when playback stopped by itself
    /// since the last call: the user closed the player window, or the stream
    /// failed or ended. A failed stream would otherwise leave mpv's black
    /// window holding the stream slot, so the player is closed. Non-blocking.
    pub fn tick(&mut self) -> Option<Tick> {
        self.playing?;
        if !self.player.is_running() {
            self.playing = None;
            self.gate.end();
            return Some(Tick::Closed);
        }
        let end = self.player.poll_end()?;
        self.stop();
        Some(match end {
            tuner_mpv::StreamEnd::Ended => Tick::Ended,
            end => Tick::Failed(capitalize(&end.to_string())),
        })
    }

    fn start(&mut self, index: usize, url: &str) -> Request {
        match self.player.play(url) {
            Ok(()) => {
                self.playing = Some(index);
                Request::Started
            }
            Err(e) => {
                if self.playing.is_none() {
                    self.gate.end();
                }
                Request::Failed(e.to_string())
            }
        }
    }
}

/// The status line for channel `name` while it opens and plays, e.g.
/// `Buffering BBC News… 640 KB, 1.5 s of video · step 2/2 · 7 s`. Front-ends
/// refresh it every tick while the stream is not playing yet.
pub fn status_line(name: &str, progress: Option<&LoadProgress>) -> String {
    let Some(p) = progress.filter(|p| p.stage != LoadStage::Playing) else {
        return format!("Playing {name}");
    };
    let secs = p.elapsed.as_secs();
    // What mpv is struggling with, e.g. a timed-out connection it retries.
    let problem = p
        .problem
        .as_deref()
        .map(|problem| format!(" · {problem}"))
        .unwrap_or_default();
    let line = match p.stage {
        LoadStage::Connecting => format!("Connecting to {name}… · step 1/2 · {secs} s"),
        _ => {
            let mut got = Vec::new();
            if let Some(bytes) = p.bytes.filter(|b| *b > 0) {
                got.push(human_bytes(bytes));
            }
            if let Some(d) = p.buffered.filter(|d| !d.is_zero()) {
                got.push(format!("{} of video", human_secs(d)));
            }
            let got = if got.is_empty() {
                String::new()
            } else {
                format!(" {}", got.join(", "))
            };
            format!("Buffering {name}…{got} · step 2/2 · {secs} s")
        }
    };
    line + &problem
}

fn human_bytes(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / f64::from(1 << 20)),
        b if b >= 1 << 10 => format!("{} KB", b >> 10),
        b => format!("{b} B"),
    }
}

fn human_secs(d: Duration) -> String {
    format!("{:.1} s", d.as_secs_f64())
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    type Log = Rc<RefCell<Vec<String>>>;

    struct FakePlayer {
        log: Log,
        running: Rc<RefCell<bool>>,
        fail: bool,
        end: Rc<RefCell<Option<tuner_mpv::StreamEnd>>>,
    }

    impl Playback for FakePlayer {
        fn play(&mut self, url: &str) -> tuner_mpv::Result<()> {
            if self.fail {
                return Err(tuner_mpv::Error::ExitedEarly);
            }
            self.log.borrow_mut().push(format!("play {url}"));
            *self.running.borrow_mut() = true;
            Ok(())
        }
        fn shutdown(&mut self) {
            self.log.borrow_mut().push("shutdown".into());
            *self.running.borrow_mut() = false;
        }
        fn toggle_pause(&mut self) -> tuner_mpv::Result<()> {
            Ok(())
        }
        fn is_running(&mut self) -> bool {
            *self.running.borrow()
        }
        fn poll_end(&mut self) -> Option<tuner_mpv::StreamEnd> {
            self.end.borrow_mut().take()
        }
    }

    struct FakeGate {
        verdict: Begin,
        log: Log,
    }

    impl StreamGate for FakeGate {
        fn begin(&mut self) -> Begin {
            self.log.borrow_mut().push("begin".into());
            self.verdict.clone()
        }
        fn end(&mut self) {
            self.log.borrow_mut().push("end".into());
        }
    }

    fn controller(
        verdict: Begin,
        fail: bool,
    ) -> (Controller<FakePlayer, FakeGate>, Log, Rc<RefCell<bool>>) {
        let log = Log::default();
        let running = Rc::new(RefCell::new(false));
        let player = FakePlayer {
            log: log.clone(),
            running: running.clone(),
            fail,
            end: Rc::default(),
        };
        let gate = FakeGate {
            verdict,
            log: log.clone(),
        };
        (Controller::new(player, gate), log, running)
    }

    #[test]
    fn go_plays_and_switching_skips_the_gate() {
        let (mut c, log, _) = controller(Begin::Go, false);
        assert_eq!(c.request(0, "a"), Request::Started);
        assert_eq!(c.request(1, "b"), Request::Started);
        assert_eq!(c.playing(), Some(1));
        assert_eq!(*log.borrow(), ["begin", "play a", "play b"]);
    }

    #[test]
    fn stop_releases_slot() {
        let (mut c, log, _) = controller(Begin::Go, false);
        c.request(0, "a");
        c.stop();
        assert_eq!(c.playing(), None);
        assert_eq!(log.borrow()[2..], ["shutdown", "end"]);
    }

    #[test]
    fn replacing_account_stops_old_stream_before_using_new_gate() {
        let (mut c, old_log, _) = controller(Begin::Go, false);
        c.request(0, "old");
        let new_log = Log::default();
        c.replace_gate(FakeGate {
            verdict: Begin::Blocked("new account full".into()),
            log: new_log.clone(),
        });
        assert_eq!(c.playing(), None);
        assert_eq!(*old_log.borrow(), ["begin", "play old", "shutdown", "end"]);
        assert!(new_log.borrow().is_empty());
        assert_eq!(
            c.request(0, "new"),
            Request::Blocked("new account full".into())
        );
        assert_eq!(*new_log.borrow(), ["begin"]);
    }

    #[test]
    fn replacing_account_releases_a_pending_confirmation() {
        let (mut c, old_log, _) = controller(Begin::Confirm("full".into()), false);
        c.request(0, "old");
        c.replace_gate(FakeGate {
            verdict: Begin::Go,
            log: Log::default(),
        });
        assert_eq!(*old_log.borrow(), ["begin", "shutdown", "end"]);
        assert_eq!(c.playing(), None);
    }

    #[test]
    fn confirm_and_cancel() {
        let (mut c, log, _) = controller(Begin::Confirm("full".into()), false);
        assert_eq!(c.request(0, "a"), Request::NeedsConfirm("full".into()));
        c.cancel();
        assert_eq!(*log.borrow(), ["begin", "end"]);
        c.request(0, "a");
        assert_eq!(c.confirm(0, "a"), Request::Started);
        assert_eq!(c.playing(), Some(0));
    }

    #[test]
    fn blocked_opens_nothing() {
        let (mut c, log, _) = controller(Begin::Blocked("busy".into()), false);
        assert_eq!(c.request(0, "a"), Request::Blocked("busy".into()));
        assert_eq!(*log.borrow(), ["begin"]);
        assert_eq!(c.playing(), None);
    }

    #[test]
    fn failed_start_releases_slot() {
        let (mut c, log, _) = controller(Begin::Go, true);
        assert!(matches!(c.request(0, "a"), Request::Failed(_)));
        assert_eq!(*log.borrow(), ["begin", "end"]);
    }

    #[test]
    fn watch_only_asks_while_playing() {
        let (mut c, log, _) = controller(Begin::Go, false);
        assert_eq!(c.watch(), None);
        c.request(0, "a");
        assert_eq!(c.watch(), None);
        assert_eq!(*log.borrow(), ["begin", "play a"]);
    }

    #[test]
    fn closed_player_releases_slot_once() {
        let (mut c, log, running) = controller(Begin::Go, false);
        c.request(0, "a");
        *running.borrow_mut() = false;
        assert_eq!(c.tick(), Some(Tick::Closed));
        assert_eq!(c.tick(), None);
        assert_eq!(c.playing(), None);
        assert_eq!(log.borrow().iter().filter(|l| *l == "end").count(), 1);
    }

    #[test]
    fn failed_stream_closes_player_and_releases_slot() {
        let log = Log::default();
        let running = Rc::new(RefCell::new(false));
        let end = Rc::new(RefCell::new(None));
        let player = FakePlayer {
            log: log.clone(),
            running,
            fail: false,
            end: end.clone(),
        };
        let gate = FakeGate {
            verdict: Begin::Go,
            log: log.clone(),
        };
        let mut c = Controller::new(player, gate);
        assert_eq!(c.tick(), None);
        c.request(0, "a");
        assert_eq!(c.tick(), None);

        *end.borrow_mut() = Some(tuner_mpv::StreamEnd::Failed("loading failed".into()));
        assert_eq!(
            c.tick(),
            Some(Tick::Failed(
                "The stream could not be played (loading failed)".into()
            ))
        );
        assert_eq!(c.playing(), None);
        assert_eq!(*log.borrow(), ["begin", "play a", "shutdown", "end"]);
        assert_eq!(c.tick(), None);

        c.request(0, "b");
        *end.borrow_mut() = Some(tuner_mpv::StreamEnd::Ended);
        assert_eq!(c.tick(), Some(Tick::Ended));
        assert_eq!(c.playing(), None);
    }

    #[test]
    fn status_line_shows_step_progress_and_elapsed() {
        let p = |stage, secs, bytes, buffered: Option<u64>| LoadProgress {
            stage,
            elapsed: Duration::from_secs(secs),
            bytes,
            buffered: buffered.map(Duration::from_millis),
            problem: None,
        };
        assert_eq!(
            status_line("BBC", Some(&p(LoadStage::Connecting, 4, None, None))),
            "Connecting to BBC… · step 1/2 · 4 s"
        );
        assert_eq!(
            status_line("BBC", Some(&p(LoadStage::Buffering, 7, None, None))),
            "Buffering BBC… · step 2/2 · 7 s"
        );
        assert_eq!(
            status_line(
                "BBC",
                Some(&p(LoadStage::Buffering, 7, Some(655_360), Some(1500)))
            ),
            "Buffering BBC… 640 KB, 1.5 s of video · step 2/2 · 7 s"
        );
        assert_eq!(
            status_line("BBC", Some(&p(LoadStage::Playing, 9, None, None))),
            "Playing BBC"
        );
        assert_eq!(status_line("BBC", None), "Playing BBC");
        let mut stuck = p(LoadStage::Connecting, 9, None, None);
        stuck.problem = Some("HTTP error 403 Forbidden".into());
        assert_eq!(
            status_line("BBC", Some(&stuck)),
            "Connecting to BBC… · step 1/2 · 9 s · HTTP error 403 Forbidden"
        );
        assert_eq!(human_bytes(3 << 20), "3.0 MB");
        assert_eq!(human_bytes(12), "12 B");
    }
}
