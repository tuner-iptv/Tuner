//! Follows one `loadfile` through mpv's events, so a stream that cannot be
//! opened (or stops delivering data before its first frame) is reported
//! instead of leaving the user waiting, and its progress can be shown. Pure:
//! the caller feeds events and the current time.
//!
//! Rather than one short timeout, each stage has its own signal:
//! connecting is bounded by mpv's `--network-timeout` (a server silent for
//! 10 s), with [`CONNECT_TIMEOUT`] only as a last resort, since slow
//! providers can take many seconds to open an HLS stream while still
//! working. Buffering fails only when no data arrives for
//! [`NO_DATA_TIMEOUT`], so a slow but live stream is never cut off.

use std::fmt;
use std::time::{Duration, Instant};

use serde_json::Value;

/// Last-resort bound on opening a stream (playlist, first segments,
/// probing). mpv's own `--network-timeout` fires first for a silent server.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
/// Once the stream is open, how long it may receive nothing at all before
/// its first frame.
pub const NO_DATA_TIMEOUT: Duration = Duration::from_secs(8);
/// [`NO_DATA_TIMEOUT`] for a stream started at a position (a movie resumed
/// in another player): it seeks into the file before any frame, which
/// takes longer on slow providers.
pub const RESUME_NO_DATA_TIMEOUT: Duration = Duration::from_secs(25);

/// The mpv property observed for data arriving. Its `total-bytes` grows as
/// packets reach the demuxer cache, for plain HTTP and HLS alike
/// (`cache-speed` misses HLS segments, which ffmpeg fetches itself).
pub(crate) const CACHE_PROPERTY: &str = "demuxer-cache-state";

/// Where the stream from the last `loadfile` is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadStage {
    /// Reaching the server, fetching playlists and probing the stream.
    Connecting,
    /// Opened; filling the cache for the first frame.
    Buffering,
    /// The first frame (or audio) is out.
    Playing,
}

/// A snapshot of a stream being opened, for showing progress.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadProgress {
    pub stage: LoadStage,
    /// Since the `loadfile`.
    pub elapsed: Duration,
    /// Bytes in mpv's demuxer cache (buffering only).
    pub bytes: Option<u64>,
    /// Media time in mpv's demuxer cache (buffering only).
    pub buffered: Option<Duration>,
    /// The latest network/demuxer problem mpv logged for this stream, e.g.
    /// `HTTP error 403 Forbidden`. URLs are cut to `scheme://host:port`.
    pub problem: Option<String>,
}

/// Why the stream that tuner loaded is no longer playing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEnd {
    /// mpv could not open or play it; carries mpv's `file_error`.
    Failed(String),
    /// The stream did not open within [`CONNECT_TIMEOUT`].
    TimedOut,
    /// Opened, but no data arrived for this long ([`NO_DATA_TIMEOUT`], or
    /// [`RESUME_NO_DATA_TIMEOUT`] when started at a position).
    Stalled(Duration),
    /// It played and then ended (a live stream dropped, a movie finished).
    Ended,
    /// Stopped from inside mpv rather than by tuner (its keys or on-screen
    /// controller, MPRIS media keys).
    Stopped,
}

impl fmt::Display for StreamEnd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StreamEnd::Failed(reason) => write!(f, "the stream could not be played ({reason})"),
            StreamEnd::TimedOut => write!(
                f,
                "the stream did not open within {} s",
                CONNECT_TIMEOUT.as_secs()
            ),
            StreamEnd::Stalled(after) => write!(
                f,
                "the stream opened but sent no data for {} s",
                after.as_secs()
            ),
            StreamEnd::Ended => write!(f, "the stream ended"),
            StreamEnd::Stopped => write!(f, "playback was stopped in mpv"),
        }
    }
}

struct Load {
    /// mpv's playlist entry for our file, once known.
    entry: Option<i64>,
    /// Our `start-file` was seen, so later events are about our file.
    started: bool,
    stage: LoadStage,
    began: Instant,
    /// When data last arrived (buffering).
    last_data: Instant,
    /// How long buffering may go without data.
    no_data: Duration,
    bytes: Option<u64>,
    buffered: Option<Duration>,
    problem: Option<String>,
}

#[derive(Default)]
pub(crate) struct LoadWatch {
    load: Option<Load>,
}

impl LoadWatch {
    /// A `loadfile` was just accepted. `entry` is the `playlist_entry_id`
    /// from its reply (mpv 0.38+); older mpv reports it in `start-file`.
    pub fn begin(&mut self, entry: Option<i64>, now: Instant) {
        self.load = Some(Load {
            entry,
            started: false,
            stage: LoadStage::Connecting,
            began: now,
            last_data: now,
            no_data: NO_DATA_TIMEOUT,
            bytes: None,
            buffered: None,
            problem: None,
        });
    }

    /// The stream just begun starts at a position: allow
    /// [`RESUME_NO_DATA_TIMEOUT`] for its seek.
    pub fn resuming(&mut self) {
        if let Some(load) = &mut self.load {
            load.no_data = RESUME_NO_DATA_TIMEOUT;
        }
    }

    pub fn clear(&mut self) {
        self.load = None;
    }

    /// The stage of the stream being followed; `None` when none is (nothing
    /// loaded, or its end was already reported).
    pub fn stage(&self) -> Option<LoadStage> {
        self.load.as_ref().map(|l| l.stage)
    }

    pub fn progress(&self, now: Instant) -> Option<LoadProgress> {
        let load = self.load.as_ref()?;
        Some(LoadProgress {
            stage: load.stage,
            elapsed: now.saturating_duration_since(load.began),
            bytes: load.bytes,
            buffered: load.buffered,
            problem: load.problem.clone(),
        })
    }

    pub fn on_event(&mut self, event: &Value, now: Instant) -> Option<StreamEnd> {
        let load = self.load.as_mut()?;
        let id = event.get("playlist_entry_id").and_then(Value::as_i64);
        match event.get("event").and_then(Value::as_str)? {
            "start-file" => {
                if load.entry.is_none() {
                    load.entry = id;
                }
                if id.is_some() && id == load.entry {
                    load.started = true;
                }
                None
            }
            "file-loaded" if load.started && load.stage == LoadStage::Connecting => {
                load.stage = LoadStage::Buffering;
                load.last_data = now;
                None
            }
            "playback-restart" if load.started => {
                load.stage = LoadStage::Playing;
                None
            }
            "property-change"
                if load.stage == LoadStage::Buffering
                    && event.get("name").and_then(Value::as_str) == Some(CACHE_PROPERTY) =>
            {
                let cache = event.get("data");
                let field = |name| cache.and_then(|c| c.get(name)).and_then(Value::as_f64);
                // mpv reports on change, so new bytes mean data arrived.
                let bytes = field("total-bytes").map(|b| b as u64);
                if bytes.is_some_and(|b| b > 0) && bytes != load.bytes {
                    load.last_data = now;
                }
                load.bytes = bytes.or(load.bytes);
                load.buffered = field("cache-duration")
                    .filter(|d| d.is_finite() && *d >= 0.0)
                    .map(Duration::from_secs_f64)
                    .or(load.buffered);
                None
            }
            // ffmpeg's own words ("HTTP error 403 Forbidden", "Connection
            // timed out") beat mpv's generic "loading failed".
            "log-message" if load.stage != LoadStage::Playing => {
                let from_ffmpeg = event
                    .get("prefix")
                    .and_then(Value::as_str)
                    .is_some_and(|p| p.starts_with("ffmpeg"));
                let serious = matches!(
                    event.get("level").and_then(Value::as_str),
                    Some("fatal" | "error" | "warn")
                );
                if from_ffmpeg && serious {
                    let text = event.get("text").and_then(Value::as_str).unwrap_or("");
                    if let Some(problem) = clean_log(text) {
                        load.problem = Some(problem);
                    }
                }
                None
            }
            "end-file" if id.is_some() && id == load.entry => {
                let end = match event.get("reason").and_then(Value::as_str) {
                    Some("error") => {
                        let error = event
                            .get("file_error")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown error");
                        StreamEnd::Failed(match &load.problem {
                            Some(problem) => format!("{problem}; {error}"),
                            None => error.to_string(),
                        })
                    }
                    Some("eof") if load.stage == LoadStage::Playing => StreamEnd::Ended,
                    Some("eof") => StreamEnd::Failed("no audio or video received".into()),
                    // A playlist link expands into new entries; follow the
                    // next file mpv starts.
                    Some("redirect") => {
                        load.entry = None;
                        load.started = false;
                        return None;
                    }
                    // tuner forgets the file before it stops or replaces it,
                    // so this stop came from mpv's side.
                    Some("stop" | "quit") => StreamEnd::Stopped,
                    _ => {
                        self.load = None;
                        return None;
                    }
                };
                self.load = None;
                Some(end)
            }
            _ => None,
        }
    }

    /// Report a stream stuck connecting, or open but receiving nothing.
    pub fn check(&mut self, now: Instant) -> Option<StreamEnd> {
        let load = self.load.as_ref()?;
        let end = match load.stage {
            LoadStage::Connecting
                if now.saturating_duration_since(load.began) >= CONNECT_TIMEOUT =>
            {
                StreamEnd::TimedOut
            }
            LoadStage::Buffering
                if now.saturating_duration_since(load.last_data) >= load.no_data =>
            {
                StreamEnd::Stalled(load.no_data)
            }
            _ => return None,
        };
        self.load = None;
        Some(end)
    }
}

/// One log line, trimmed, without ffmpeg's `component: ` prefix and with
/// every URL cut to `scheme://host:port` (stream URLs carry credentials).
fn clean_log(text: &str) -> Option<String> {
    let text = text.trim();
    let text = match text.split_once(": ") {
        Some((component, rest))
            if !component.is_empty()
                && component
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') =>
        {
            rest
        }
        _ => text,
    };
    let words: Vec<String> = text.split(' ').map(redact_url).collect();
    let line = words.join(" ");
    (!line.is_empty()).then_some(line)
}

/// `http://user:pass@host:8080/live/u/p/1.ts?x` → `http://host:8080`;
/// anything that isn't a URL is returned unchanged.
pub(crate) fn redact_url(word: &str) -> String {
    let Some((scheme, rest)) = word.split_once("://") else {
        return word.to_string();
    };
    let start = scheme
        .rfind(|c: char| !c.is_ascii_alphanumeric() && c != '+')
        .map_or(0, |i| i + 1);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("");
    let host = host.trim_end_matches(['.', ',', ';', ')', '\'', '"']);
    format!("{}{}://{host}", &scheme[..start], &scheme[start..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn begun(entry: Option<i64>) -> (LoadWatch, Instant) {
        let now = Instant::now();
        let mut w = LoadWatch::default();
        w.begin(entry, now);
        (w, now)
    }

    fn feed(w: &mut LoadWatch, at: Instant, events: &[Value]) -> Option<StreamEnd> {
        events.iter().find_map(|e| w.on_event(e, at))
    }

    fn cache(bytes: u64, secs: f64) -> Value {
        json!({"event": "property-change", "id": 2, "name": CACHE_PROPERTY,
               "data": {"total-bytes": bytes, "fw-bytes": bytes, "cache-duration": secs,
                        "raw-input-rate": 64}})
    }

    fn opened(entry: i64) -> [Value; 2] {
        [
            json!({"event": "start-file", "playlist_entry_id": entry}),
            json!({"event": "file-loaded"}),
        ]
    }

    #[test]
    fn unreachable_stream_fails_with_mpvs_reason() {
        let (mut w, t) = begun(Some(2));
        assert_eq!(w.stage(), Some(LoadStage::Connecting));
        let end = feed(
            &mut w,
            t,
            &[
                json!({"event": "start-file", "playlist_entry_id": 2}),
                json!({"event": "end-file", "reason": "error", "playlist_entry_id": 2,
                       "file_error": "loading failed"}),
            ],
        );
        assert_eq!(end, Some(StreamEnd::Failed("loading failed".into())));
        // Reported once.
        assert_eq!(w.stage(), None);
        assert_eq!(w.check(t + CONNECT_TIMEOUT), None);
    }

    #[test]
    fn learns_the_entry_from_start_file_on_older_mpv() {
        let (mut w, t) = begun(None);
        let end = feed(
            &mut w,
            t,
            &[
                json!({"event": "start-file", "playlist_entry_id": 5}),
                json!({"event": "end-file", "reason": "error", "playlist_entry_id": 5,
                       "file_error": "unrecognized file format"}),
            ],
        );
        assert!(matches!(end, Some(StreamEnd::Failed(_))));
    }

    #[test]
    fn ignores_the_replaced_file_ending() {
        let (mut w, t) = begun(Some(2));
        let end = feed(
            &mut w,
            t,
            &[
                json!({"event": "end-file", "reason": "stop", "playlist_entry_id": 1}),
                json!({"event": "end-file", "reason": "error", "playlist_entry_id": 1}),
                json!({"event": "playback-restart"}),
                json!({"event": "file-loaded"}),
            ],
        );
        assert_eq!(end, None);
        // The old file's events moved nothing along.
        assert_eq!(w.stage(), Some(LoadStage::Connecting));
    }

    #[test]
    fn a_stop_from_inside_mpv_is_reported() {
        let (mut w, t) = begun(Some(1));
        let end = feed(
            &mut w,
            t,
            &[
                json!({"event": "start-file", "playlist_entry_id": 1}),
                json!({"event": "playback-restart"}),
                json!({"event": "end-file", "reason": "stop", "playlist_entry_id": 1}),
            ],
        );
        assert_eq!(end, Some(StreamEnd::Stopped));
        assert_eq!(w.stage(), None);
    }

    #[test]
    fn connecting_has_a_backstop() {
        let (mut w, t) = begun(Some(1));
        feed(
            &mut w,
            t,
            &[json!({"event": "start-file", "playlist_entry_id": 1})],
        );
        assert_eq!(w.check(t + CONNECT_TIMEOUT / 2), None);
        assert_eq!(w.check(t + CONNECT_TIMEOUT), Some(StreamEnd::TimedOut));
        assert_eq!(w.check(t + CONNECT_TIMEOUT * 2), None);
    }

    #[test]
    fn open_stream_sending_nothing_stalls_quickly() {
        let (mut w, t) = begun(Some(1));
        let open = t + Duration::from_secs(3);
        feed(&mut w, open, &opened(1));
        assert_eq!(w.stage(), Some(LoadStage::Buffering));
        assert_eq!(w.check(open + NO_DATA_TIMEOUT / 2), None);
        assert_eq!(
            w.check(open + NO_DATA_TIMEOUT),
            Some(StreamEnd::Stalled(NO_DATA_TIMEOUT))
        );
    }

    #[test]
    fn a_resumed_stream_gets_longer_to_seek() {
        let (mut w, t) = begun(Some(1));
        w.resuming();
        feed(&mut w, t, &opened(1));
        assert_eq!(w.check(t + NO_DATA_TIMEOUT * 2), None);
        assert_eq!(
            w.check(t + RESUME_NO_DATA_TIMEOUT),
            Some(StreamEnd::Stalled(RESUME_NO_DATA_TIMEOUT))
        );
    }

    #[test]
    fn slow_stream_that_keeps_receiving_is_not_cut_off() {
        let (mut w, t) = begun(Some(1));
        feed(&mut w, t, &opened(1));
        // A trickle: a few bytes every 6 s, for a minute.
        for i in 1..=10u64 {
            let at = t + Duration::from_secs(6 * i);
            assert_eq!(w.check(at), None);
            feed(&mut w, at, &[cache(1000 * i, 0.1 * i as f64)]);
        }
        let last = t + Duration::from_secs(60);
        assert_eq!(w.check(last + NO_DATA_TIMEOUT / 2), None);
        // The same numbers again are not new data.
        feed(&mut w, last + Duration::from_secs(1), &[cache(10_000, 1.0)]);
        assert_eq!(
            w.check(last + NO_DATA_TIMEOUT),
            Some(StreamEnd::Stalled(NO_DATA_TIMEOUT))
        );
    }

    #[test]
    fn playing_is_never_stalled() {
        let (mut w, t) = begun(Some(1));
        feed(&mut w, t, &opened(1));
        feed(&mut w, t, &[json!({"event": "playback-restart"})]);
        assert_eq!(w.stage(), Some(LoadStage::Playing));
        assert_eq!(w.check(t + NO_DATA_TIMEOUT * 10), None);
    }

    #[test]
    fn slow_hls_open_is_not_cut_off() {
        let (mut w, t) = begun(Some(1));
        feed(
            &mut w,
            t,
            &[json!({"event": "start-file", "playlist_entry_id": 1})],
        );
        assert_eq!(w.check(t + Duration::from_secs(30)), None);
        assert_eq!(w.check(t + CONNECT_TIMEOUT), Some(StreamEnd::TimedOut));
    }

    #[test]
    fn progress_reports_stage_elapsed_and_cache() {
        let (mut w, t) = begun(Some(1));
        let p = w.progress(t + Duration::from_secs(2)).unwrap();
        assert_eq!(p.stage, LoadStage::Connecting);
        assert_eq!(p.elapsed, Duration::from_secs(2));
        assert_eq!(p.bytes, None);

        feed(&mut w, t, &opened(1));
        feed(&mut w, t, &[cache(655_360, 1.5)]);
        let p = w.progress(t + Duration::from_secs(5)).unwrap();
        assert_eq!(p.stage, LoadStage::Buffering);
        assert_eq!(p.bytes, Some(655_360));
        assert_eq!(p.buffered, Some(Duration::from_millis(1500)));
        // A cache update without numbers keeps the last ones.
        feed(
            &mut w,
            t,
            &[json!({"event": "property-change", "id": 2, "name": CACHE_PROPERTY})],
        );
        assert_eq!(w.progress(t).unwrap().bytes, Some(655_360));
    }

    #[test]
    fn eof_after_playing_is_an_end_and_before_is_a_failure() {
        let (mut w, t) = begun(Some(1));
        let end = feed(
            &mut w,
            t,
            &[
                json!({"event": "start-file", "playlist_entry_id": 1}),
                json!({"event": "playback-restart"}),
                json!({"event": "end-file", "reason": "eof", "playlist_entry_id": 1}),
            ],
        );
        assert_eq!(end, Some(StreamEnd::Ended));

        let (mut w, t) = begun(Some(1));
        let end = feed(
            &mut w,
            t,
            &[
                json!({"event": "start-file", "playlist_entry_id": 1}),
                json!({"event": "end-file", "reason": "eof", "playlist_entry_id": 1}),
            ],
        );
        assert!(matches!(end, Some(StreamEnd::Failed(_))));
    }

    #[test]
    fn follows_playlist_redirects() {
        let (mut w, t) = begun(Some(1));
        let end = feed(
            &mut w,
            t,
            &[
                json!({"event": "start-file", "playlist_entry_id": 1}),
                json!({"event": "end-file", "reason": "redirect", "playlist_entry_id": 1}),
                json!({"event": "start-file", "playlist_entry_id": 2}),
                json!({"event": "end-file", "reason": "error", "playlist_entry_id": 2,
                       "file_error": "loading failed"}),
            ],
        );
        assert!(matches!(end, Some(StreamEnd::Failed(_))));
    }

    #[test]
    fn idle_watch_reports_nothing() {
        let mut w = LoadWatch::default();
        let now = Instant::now();
        assert_eq!(
            w.on_event(&json!({"event": "end-file", "reason": "error"}), now),
            None
        );
        assert_eq!(w.check(now), None);
        assert_eq!(w.stage(), None);
    }

    fn log(level: &str, prefix: &str, text: &str) -> Value {
        json!({"event": "log-message", "level": level, "prefix": prefix, "text": text})
    }

    #[test]
    fn ffmpegs_reason_is_shown_and_reported() {
        let (mut w, t) = begun(Some(1));
        let end = feed(
            &mut w,
            t,
            &[
                json!({"event": "start-file", "playlist_entry_id": 1}),
                log("v", "ffmpeg", "Opening http://h:80/live/u/secret/1.ts\n"),
                log(
                    "error",
                    "ffmpeg",
                    "tcp: Connection to tcp://10.0.0.9:8080 failed: Connection timed out\n",
                ),
            ],
        );
        assert_eq!(end, None);
        assert_eq!(
            w.progress(t).unwrap().problem.as_deref(),
            Some("Connection to tcp://10.0.0.9:8080 failed: Connection timed out")
        );
        let end = feed(
            &mut w,
            t,
            &[
                // mpv's own line repeats the URL; not from ffmpeg, ignored.
                log(
                    "error",
                    "stream",
                    "Failed to open http://h:80/live/u/secret/1.ts.\n",
                ),
                json!({"event": "end-file", "reason": "error", "playlist_entry_id": 1,
                       "file_error": "loading failed"}),
            ],
        );
        assert_eq!(
            end,
            Some(StreamEnd::Failed(
                "Connection to tcp://10.0.0.9:8080 failed: Connection timed out; loading failed"
                    .into()
            ))
        );
    }

    #[test]
    fn log_lines_never_carry_credentials() {
        assert_eq!(
            clean_log("http: HTTP error 403 Forbidden\n").as_deref(),
            Some("HTTP error 403 Forbidden")
        );
        assert_eq!(
            clean_log("Failed to open 'http://user:pw@h.example:8080/live/user/pw/1.ts?token=x'")
                .as_deref(),
            Some("Failed to open 'http://h.example:8080")
        );
        assert_eq!(redact_url("https://h/p.m3u8,"), "https://h");
        assert_eq!(redact_url("(http://h:1/a)"), "(http://h:1");
        assert_eq!(redact_url("plain"), "plain");
        assert_eq!(clean_log("  \n"), None);
    }
}
