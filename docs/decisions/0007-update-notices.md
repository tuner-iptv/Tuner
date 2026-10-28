# 0007. Update notices, no self-update

- Status: Accepted
- Date: 2026-10-28

## Context

Users should learn when a new release is tagged. Most installs are `.deb`,
`.rpm` or AUR packages in `/usr/bin`, owned by the package manager and not
writable by the user. A self-replacing binary would fight the package
manager, need root, and add a download-and-execute path to secure.

## Decision

At startup each front-end asks GitHub's `releases/latest` API, on a worker
thread, for the newest tag (`tuner_app::update::check`). The answer is cached
in `$XDG_CACHE_HOME/tuner/latest-release` for a day, so tuner makes at most
one request a day. A newer plain `X.Y.Z` tag shows a notice with the update
step for this install: the AUR package to update, a `dnf install <url>` for
the `.rpm`, or the release page (for the `.deb`, tarballs and source builds).
Tuner never downloads or replaces its own binaries.

The notice stays until the user dismisses it (`x` in the TUI, the close
button in the GUI). Dismissing records the version in
`$XDG_CACHE_HOME/tuner/dismissed-release`, so it stays hidden across
restarts until a newer release appears. In the TUI the notice takes at most
half the status line, dropping the update step and then the key hint when
the terminal is narrow.

The check is on by default; `check_updates = false` in `config.toml` turns
it off. It sends nothing but a request with a `tuner/<version>` user agent.

## Consequences

- No privileged or self-modifying code; packages stay consistent.
- Pre-release tags never trigger a notice, since `releases/latest` skips
  them and non-numeric versions don't compare as newer.
- Offline, rate-limited or blocked requests fail silently.
- Install detection is a heuristic (`/var/lib/pacman/local`, `rpm -q`); a
  wrong guess only falls back to the release page.
