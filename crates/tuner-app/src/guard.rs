//! Stream-limit enforcement. IPTV providers can ban accounts that open more
//! streams than their plan allows, so before tuner starts a stream it:
//!
//! 1. takes an OS file lock on one of the account's `max_streams` slots, so
//!    tuner windows on this machine can never exceed the limit (hard block);
//! 2. asks the provider how many connections are in use *across all
//!    devices* (Xtream `active_cons`). If the account looks full it rechecks
//!    a few times (providers release a just-stopped stream with a delay),
//!    then asks the user to confirm, or refuses in `LimitMode::Block`.
//!
//! While a stream plays, front-ends call [`StreamGate::watch`] periodically
//! to warn when another device pushes the account over its limit.
//!
//! The lock is held while a stream plays and released on stop or exit (the
//! OS releases it even if tuner crashes). See `docs/stream-limits.md`.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::source::{Source, Usage};

/// Outcome of asking to start a stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Begin {
    /// Safe to play.
    Go,
    /// Playing may exceed the limit; the user must explicitly confirm.
    Confirm(String),
    /// Playing would certainly exceed the limit; do not play.
    Blocked(String),
}

/// What to do when the provider reports the account as full.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LimitMode {
    /// Warn and let the user explicitly choose to play anyway.
    #[default]
    Confirm,
    /// Refuse to play.
    Block,
}

impl LimitMode {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// What a front-end needs to respect stream limits. Implemented by
/// [`StreamGuard`]; front-ends use fakes in tests.
pub trait StreamGate {
    /// Call before starting playback when nothing is playing. On `Go` or
    /// `Confirm` a slot is held until [`StreamGate::end`].
    fn begin(&mut self) -> Begin;
    /// Call when playback stops, the user cancels a `Confirm`, or the player
    /// goes away.
    fn end(&mut self);
    /// Call periodically while playing. Returns a warning when the provider
    /// reports more connections than the account allows (another device
    /// started streaming).
    fn watch(&mut self) -> Option<String> {
        None
    }
}

type UsageFn = Box<dyn FnMut() -> Result<Option<Usage>> + Send>;

/// How often to recheck a full account before deciding, and how long to wait
/// between checks.
const FULL_RETRIES: u32 = 2;
const FULL_RETRY_DELAY: Duration = Duration::from_secs(2);

/// A `flock` belongs to the open file description, which a forked child
/// shares until it execs. Another thread spawning a process (a
/// `password_command`, mpv) can therefore keep a just-released slot locked
/// for a moment. Before reporting "all slots taken" we retry for this long.
const LOCK_RETRIES: u32 = 10;
const LOCK_RETRY_DELAY: Duration = Duration::from_millis(20);

pub struct StreamGuard {
    key: String,
    slots: u32,
    mode: LimitMode,
    dir: PathBuf,
    held: Option<File>,
    usage: UsageFn,
    retries: u32,
    retry_delay: Duration,
    lock_retries: u32,
    lock_retry_delay: Duration,
}

impl StreamGuard {
    /// `slots` is how many streams the account may play at once (≥ 1).
    pub fn new(source: Source, slots: u32, mode: LimitMode) -> Self {
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("tuner")
            .join("locks");
        let key = source.account_key();
        Self::with_parts(key, slots, mode, dir, Box::new(move || source.usage()))
    }

    fn with_parts(key: String, slots: u32, mode: LimitMode, dir: PathBuf, usage: UsageFn) -> Self {
        Self {
            key,
            slots: slots.max(1),
            mode,
            dir,
            held: None,
            usage,
            retries: FULL_RETRIES,
            retry_delay: FULL_RETRY_DELAY,
            lock_retries: LOCK_RETRIES,
            lock_retry_delay: LOCK_RETRY_DELAY,
        }
    }

    pub fn slots(&self) -> u32 {
        self.slots
    }

    /// Try each slot's lock file; keep the first one we get. When every slot
    /// is taken, retry briefly in case the holder is only a forked child that
    /// has not exec'd yet (see [`LOCK_RETRIES`]).
    fn try_acquire(&mut self) -> io::Result<bool> {
        for attempt in 0..=self.lock_retries {
            if attempt > 0 {
                thread::sleep(self.lock_retry_delay);
            }
            if self.try_slots()? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn try_slots(&mut self) -> io::Result<bool> {
        std::fs::create_dir_all(&self.dir)?;
        let hash = fnv1a(&self.key);
        for slot in 0..self.slots {
            let path = self.dir.join(format!("{hash:016x}-{slot}.lock"));
            let file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(path)?;
            match file.try_lock() {
                Ok(()) => {
                    self.held = Some(file);
                    return Ok(true);
                }
                Err(TryLockError::WouldBlock) => continue,
                Err(TryLockError::Error(e)) => return Err(e),
            }
        }
        Ok(false)
    }
}

impl StreamGate for StreamGuard {
    fn begin(&mut self) -> Begin {
        if self.held.is_none() {
            match self.try_acquire() {
                Ok(true) => {}
                Ok(false) => {
                    let n = self.slots;
                    let plural = if n == 1 { "stream" } else { "streams" };
                    return Begin::Blocked(format!(
                        "Another tuner window is already playing on this account \
                         ({n} {plural} allowed). Stop playback there first."
                    ));
                }
                Err(e) => {
                    return Begin::Confirm(format!(
                        "Could not check for other tuner windows ({e}). \
                         Make sure nothing else is playing on this account."
                    ));
                }
            }
        }
        let mut usage = (self.usage)();
        for _ in 0..self.retries {
            if !is_full(&usage) {
                break;
            }
            thread::sleep(self.retry_delay);
            usage = (self.usage)();
        }
        assess(usage, self.mode)
    }

    fn end(&mut self) {
        // Dropping the file releases the lock.
        self.held = None;
    }

    fn watch(&mut self) -> Option<String> {
        match (self.usage)() {
            Ok(Some(Usage { active, max })) if active > max => Some(format!(
                "Your account is over its limit: the provider reports {active} \
                 connections in use, {max} allowed. Another device or app started \
                 a stream while this one is playing. Providers can ban accounts \
                 for this. Stop one of the streams."
            )),
            _ => None,
        }
    }
}

fn is_full(usage: &Result<Option<Usage>>) -> bool {
    matches!(usage, Ok(Some(Usage { active, max })) if active >= max)
}

/// Decide from the provider's reported usage. Pure, for testing.
pub fn assess(usage: Result<Option<Usage>>, mode: LimitMode) -> Begin {
    match usage {
        Ok(Some(Usage { active, max })) if active >= max => match mode {
            LimitMode::Confirm => Begin::Confirm(format!(
                "Your provider reports {active} of {max} connections already in use: \
                 another device or app, or a stream stopped moments ago that the \
                 server has not released yet. Opening another stream may exceed \
                 your plan, and providers can ban accounts for that."
            )),
            LimitMode::Block => Begin::Blocked(format!(
                "Your provider reports {active} of {max} connections already in use \
                 by another device or app. This source has limit_mode = \"block\", \
                 so tuner will not open another stream. Stop the other stream and \
                 try again."
            )),
        },
        Ok(_) => Begin::Go,
        Err(e) => Begin::Confirm(format!(
            "Could not check connection usage with the provider ({e:#}). \
             Make sure nothing else is playing on this account."
        )),
    }
}

/// Stable across builds (unlike `DefaultHasher`), so different tuner
/// versions agree on lock file names.
fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use std::sync::{Arc, Mutex};

    fn dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tuner-test-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// A guard whose provider reports nothing (like an M3U source), so only
    /// the lock decides.
    fn guard(test: &str, slots: u32) -> StreamGuard {
        StreamGuard::with_parts(
            "k".into(),
            slots,
            LimitMode::Confirm,
            dir(test),
            Box::new(|| Ok(None)),
        )
    }

    fn twin(g: &StreamGuard) -> StreamGuard {
        StreamGuard::with_parts(
            g.key.clone(),
            g.slots,
            g.mode,
            g.dir.clone(),
            Box::new(|| Ok(None)),
        )
    }

    /// A guard whose provider answers with `answers` in order (last repeats),
    /// recording how many times it was asked.
    fn provider(
        test: &str,
        mode: LimitMode,
        answers: Vec<(u32, u32)>,
    ) -> (StreamGuard, Arc<Mutex<usize>>) {
        let calls = Arc::new(Mutex::new(0));
        let counter = calls.clone();
        let usage = Box::new(move || {
            let mut n = counter.lock().unwrap();
            let (active, max) = answers[(*n).min(answers.len() - 1)];
            *n += 1;
            Ok(Some(Usage { active, max }))
        });
        let mut g = StreamGuard::with_parts("k".into(), 1, mode, dir(test), usage);
        g.retry_delay = Duration::ZERO;
        (g, calls)
    }

    #[test]
    fn second_window_is_blocked_until_first_ends() {
        let mut first = guard("single", 1);
        let mut second = twin(&first);
        assert_eq!(first.begin(), Begin::Go);
        assert!(matches!(second.begin(), Begin::Blocked(_)));
        first.end();
        assert_eq!(second.begin(), Begin::Go);
    }

    #[test]
    fn a_lock_briefly_kept_alive_by_a_forked_child_is_retried() {
        let mut first = guard("leaked", 1);
        let mut second = twin(&first);
        assert_eq!(first.begin(), Begin::Go);
        // A clone shares the open file description, like a forked child that
        // has not exec'd yet: the flock survives `first.end()`.
        let leaked = first.held.as_ref().unwrap().try_clone().unwrap();
        first.end();
        let child = thread::spawn(move || {
            thread::sleep(Duration::from_millis(60));
            drop(leaked);
        });
        assert_eq!(second.begin(), Begin::Go);
        child.join().unwrap();
    }

    #[test]
    fn a_lock_that_stays_held_still_blocks() {
        let mut first = guard("held", 1);
        let mut second = twin(&first);
        second.lock_retry_delay = Duration::ZERO;
        assert_eq!(first.begin(), Begin::Go);
        assert!(matches!(second.begin(), Begin::Blocked(_)));
    }

    #[test]
    fn begin_while_holding_does_not_take_another_slot() {
        let mut first = guard("rebegin", 2);
        let mut second = twin(&first);
        assert_eq!(first.begin(), Begin::Go);
        assert_eq!(first.begin(), Begin::Go);
        assert_eq!(second.begin(), Begin::Go);
    }

    #[test]
    fn slots_follow_the_limit() {
        let first = guard("multi", 2);
        let (mut a, mut b, mut c) = (twin(&first), twin(&first), twin(&first));
        assert_eq!(a.begin(), Begin::Go);
        assert_eq!(b.begin(), Begin::Go);
        assert!(matches!(c.begin(), Begin::Blocked(m) if m.contains("2 streams")));
    }

    #[test]
    fn zero_slots_means_one() {
        assert_eq!(guard("zero", 0).slots(), 1);
    }

    #[test]
    fn full_or_unknown_account_needs_confirmation() {
        let confirm = LimitMode::Confirm;
        assert_eq!(assess(Ok(None), confirm), Begin::Go);
        assert_eq!(
            assess(Ok(Some(Usage { active: 0, max: 1 })), confirm),
            Begin::Go
        );
        assert!(matches!(
            assess(Ok(Some(Usage { active: 1, max: 1 })), confirm),
            Begin::Confirm(m) if m.contains("1 of 1")
        ));
        assert!(matches!(
            assess(Err(anyhow!("timeout")), confirm),
            Begin::Confirm(m) if m.contains("timeout")
        ));
    }

    #[test]
    fn block_mode_refuses_a_full_account() {
        let full = Ok(Some(Usage { active: 2, max: 2 }));
        assert!(matches!(
            assess(full, LimitMode::Block),
            Begin::Blocked(m) if m.contains("2 of 2")
        ));
        // An unverifiable account still asks rather than blocking outright.
        assert!(matches!(
            assess(Err(anyhow!("x")), LimitMode::Block),
            Begin::Confirm(_)
        ));
    }

    #[test]
    fn rechecks_a_full_account_before_deciding() {
        // Provider still counts a stream we just stopped, then releases it.
        let (mut g, calls) = provider("retry-ok", LimitMode::Confirm, vec![(1, 1), (0, 1)]);
        assert_eq!(g.begin(), Begin::Go);
        assert_eq!(*calls.lock().unwrap(), 2);

        let (mut g, calls) = provider("retry-full", LimitMode::Confirm, vec![(1, 1)]);
        assert!(matches!(g.begin(), Begin::Confirm(_)));
        assert_eq!(*calls.lock().unwrap(), 1 + FULL_RETRIES as usize);
    }

    #[test]
    fn watch_warns_only_when_over_the_limit() {
        // While we play, our own stream counts: 1 of 1 is fine, 2 of 1 is not.
        let (mut g, _) = provider("watch", LimitMode::Confirm, vec![(1, 1), (2, 1)]);
        assert_eq!(g.watch(), None);
        assert!(matches!(g.watch(), Some(m) if m.contains("2 connections in use, 1 allowed")));
    }
}
