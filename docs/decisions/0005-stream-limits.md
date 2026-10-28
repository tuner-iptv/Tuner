# 0005. Enforce provider stream limits

- Status: Accepted
- Date: 2026-10-27

## Context

IPTV plans limit simultaneous streams (commonly to 1) and providers ban
accounts that exceed them. Users easily do so by accident: a second terminal
running tuner, or a TV box already watching. tuner itself never opens two
streams in one window (mpv replaces the stream on channel change), so the
risk is other windows and other devices.

## Decision

- A per-account **OS file lock per stream slot** (`std::fs::File::try_lock`)
  hard-blocks tuner windows on this machine from exceeding `max_streams`.
  The OS releases locks if tuner dies, so there are no stale locks.
- Before starting a stream from a stopped state, an **Xtream login check** of
  `active_cons` vs `max_connections`. If full, or if the check fails, the user
  must explicitly confirm. We warn instead of blocking because the count can
  lag (a stream we just stopped) and the user may know better.
- Limit priority: config `max_streams` > provider `max_connections` > 1.
- Lock keys identify the account (host + username) so an Xtream source and
  an M3U `get.php` URL for the same login share slots.

## Consequences

- MSRV raised to 1.89 for `File::try_lock` (avoids a `libc`/`fs2` dependency).
- Starting playback costs one extra HTTP request for Xtream sources.
- The guard lives in `tuner-app` behind the `StreamGate` trait so every
  front-end (TUI now, GUI later) gets identical behaviour.
- Protection against other devices is only as good as the provider's
  `active_cons` reporting.
