# 0004. Credentials in config, never on the command line

- Status: Accepted
- Date: 2026-10-27

## Context

Xtream Codes logins (and later Stalker MACs) are secrets that grant paid
access. Passing them as CLI flags leaks them into shell history and the
process list. Xtream also puts the password in every API query string and
stream URL path, so naive error messages (which often include the request
URL) leak it too.

## Decision

- Xtream sources are configured in `config.toml`, with either `password` or
  `password_command` (first stdout line, run via `sh -c`) so users can keep
  secrets in `pass`, a keyring or a password manager.
- No `--password` CLI flag.
- HTTP errors are rebuilt from status code / transport kind and never include
  the URL. Labels show `host:port` only. `Credentials` redacts the password in
  `Debug`.
- Stream URLs still contain the password (protocol requirement) and are handed
  to mpv unchanged.
- The details pane hides the stream URL by default and shows it only after
  an explicit toggle (`u` in the TUI, a button in the GUI).

## Consequences

- Safe defaults without a keyring dependency.
- `password_command` executes user-configured shell; that is the user's own
  config, same trust level as their shell rc.
- Anyone adding logging must not log request URLs or stream URLs.
