# Stream limits

IPTV plans allow a fixed number of simultaneous streams (often **1**).
Exceeding it can get the account suspended or banned, so tuner treats the
limit as a safety feature, not a nicety. Implemented in
`crates/tuner-app/src/guard.rs`; UI in `crates/tuner/src/app.rs`.
Decision record: [ADR 0005](decisions/0005-stream-limits.md).

## The limit

`max_streams`, in priority order:

1. `max_streams = N` on the `[[source]]` in `config.toml` (set it lower than
   your plan to keep a stream free for your TV, say);
2. the provider's `max_connections` from the Xtream login;
3. **1** (M3U sources, or providers that don't report it).

The status bar shows it: `… · limit 1 stream`.

## What happens when you press Enter

Only when **nothing is playing** in this window (switching channels reuses the
same mpv stream, so it never adds a connection):

1. **Local lock (hard block).** tuner takes an OS file lock on one of the
   account's `max_streams` slots in `$XDG_RUNTIME_DIR/tuner/locks/`. If all
   slots are held by other tuner windows, a **Stream limit** popup explains
   and nothing is opened. Locks are released on stop (`s`), when you close the
   mpv window, when the stream fails or ends, on quit, and by the OS if tuner
   crashes. A process forked while the lock is held (a `password_command`,
   mpv) shares the lock until it execs, so a slot can look taken for a few
   milliseconds after release; tuner retries for up to ~200 ms before it
   reports the popup.

   tuner also quits mpv on SIGTERM, SIGINT and SIGHUP (e.g. closing the
   terminal running the TUI), so an orphaned mpv never keeps streaming on
   the account's connection.

   In the TUI, switching accounts (`a`) also quits mpv and releases the old
   slot after the new source loads successfully. The next playback request
   uses the new account's identity, `max_streams` and `limit_mode`; it cannot
   reuse the previous account's slot. A failed load leaves the old account
   and playback in place.
2. **Provider check (all devices).** tuner logs in again and reads
   `active_cons`, the provider's own count of open streams on the account
   from *every* device and app. This is the cross-device signal other IPTV
   players use too; there is no other source of truth. If
   `active_cons >= max_connections` tuner rechecks twice, 2 s apart (servers
   release a just-stopped stream with a delay), then:
   - `limit_mode = "confirm"` (default): a **confirm** popup. `y` / *Play
     anyway* plays, anything else cancels and opens nothing.
   - `limit_mode = "block"`: a **blocked** popup; nothing opens.

   If the check itself fails (network error) you get the confirm popup in
   both modes.

   Works for Xtream sources and for M3U `get.php?username=…&password=…`
   links (tuner asks the same Xtream API with those credentials).

## While playing

Every 60 s tuner asks the provider again. If `active_cons > max_connections`
(our own stream counts as one) another device started streaming on top of
ours, and a **Stream limit exceeded** warning appears. tuner never stops your
stream by itself; stop one of them.

| Situation                                   | Result           |
| ------------------------------------------- | ---------------- |
| Free slot, provider below limit / M3U       | plays            |
| All local slots taken by other tuner windows| blocked          |
| Provider reports account full (after rechecks) | confirm (`y`), or blocked with `limit_mode = "block"` |
| Provider check failed                       | confirm (`y`)    |
| Playing, another device pushes account over | warning popup    |
| Already playing, switch channel             | plays, no checks |

## Account identity

Locks are keyed by account, not by config entry, so these count as the same
account and share slots:

- an `xtream` source with `server = "host:8080"`, `username = "alice"`;
- an `m3u` source `http://host:8080/get.php?username=alice&password=…`.

Other M3U URLs are keyed by URL without the query string; local files by path.
The key never contains the password, and lock file names are a hash of it.

## Limits of the protection

- tuner sees other devices only through the provider's `active_cons`. Some
  panels update it slowly or not at all; plain M3U sources (not `get.php`)
  report nothing, so only the local lock applies to them.
- If tuner is killed with SIGKILL (or crashes hard) mpv can survive it. The
  lock is released, but the provider check will still see mpv's connection.
- When the stream fails to open, stops sending data before its first frame,
  or ends, tuner quits mpv and releases the slot itself (a *Stream
  unavailable* popup explains failures). See [mpv-ipc.md](mpv-ipc.md#detecting-failed-streams).
- mpv is single-connection for `ts`/HLS live streams. Don't add mpv options
  that open parallel connections (e.g. multi-stream `--external-files`).

## Changing this code

- Keep the decision logic pure (`guard::assess`) and unit tested.
- Front-ends talk to the guard through the `StreamGate` trait. Any new
  front-end (GUI) must call `begin()` before starting playback from a stopped
  state and `end()` on stop/close/cancel.
