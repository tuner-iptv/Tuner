# AGENTS.md

Entry point for AI coding agents (Claude Code, Codex, Cursor, Copilot, …)
working in this repository. Humans: start with [README.md](README.md).

This file is deliberately short. It tells you **what to read next** for the
task at hand; the detail lives in `docs/`. Read the linked doc before changing
anything in its area.

## What this project is

**Tuner is a lean, Linux-native IPTV player written in Rust**, with a
keyboard-first terminal interface and a native GTK4 desktop app. It loads
channels from M3U playlists and Xtream Codes logins, browses them in a ratatui
TUI (`tuner`)
or a GTK4 window (`tuner-gtk`), and plays streams in an **external mpv
process** controlled over mpv's JSON IPC socket. Tuner never decodes video itself.

## Where to look

| If you are…                                        | Read first                                               |
| -------------------------------------------------- | -------------------------------------------------------- |
| New to the codebase / changing crate boundaries    | [ARCHITECTURE.md](ARCHITECTURE.md)                       |
| Setting up, building, testing, releasing           | [docs/development.md](docs/development.md)               |
| Touching playlist parsing (`tuner-core::m3u`)      | [docs/m3u-format.md](docs/m3u-format.md)                 |
| Touching sources, Xtream, config or credentials    | [docs/sources.md](docs/sources.md)                       |
| Touching playback start/stop or stream limits      | [docs/stream-limits.md](docs/stream-limits.md)           |
| Touching mpv control (`tuner-mpv`)                 | [docs/mpv-ipc.md](docs/mpv-ipc.md)                       |
| Changing keys or TUI behaviour (`tuner`)           | [docs/keybindings.md](docs/keybindings.md)               |
| Changing the GTK4 GUI or theming (`tuner-gtk`)     | [docs/gui.md](docs/gui.md)                               |
| Picking up a feature (Xtream, EPG, favorites, …)   | [docs/roadmap.md](docs/roadmap.md)                       |
| Wondering *why* something is the way it is         | [docs/decisions/](docs/decisions/README.md) (ADRs)       |
| Editing CI or cutting a release                    | [docs/ci.md](docs/ci.md)                                 |
| Touching the project site or its browser demo (`site/`) | [docs/website.md](docs/website.md)                  |
| Packaging (.deb, .rpm, AUR, static TUI)            | [docs/packaging.md](docs/packaging.md)                   |

Full index: [docs/README.md](docs/README.md).

## Commands

```sh
cargo build                                   # debug build
cargo test --workspace                        # all tests (no mpv needed)
cargo fmt --all                               # format
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p tuner -- examples/sample.m3u     # run against the sample playlist
cargo run -p tuner -- <name>                  # a [[source]] from ~/.config/tuner/config.toml
cargo run -p tuner-gtk -- <name>              # same, in the GTK4 GUI
```

CI runs exactly `fmt --check`, `clippy -D warnings` and `test`. Run all three
before you say a change is done.

## Rules

1. **Respect the crate layering** in [ARCHITECTURE.md](ARCHITECTURE.md#crate-layering).
   `tuner-core` stays pure (no I/O, no mpv, no ratatui). `tuner-mpv` knows
   nothing about playlists or the UI. `tuner-app` holds front-end-independent
   logic (config, sources, stream limits, the play/stop `Controller`) and has
   no UI code. Front-ends (`tuner`, `tuner-gtk`) only render and route input;
   behaviour that both need belongs in `tuner-app`.
2. **Never decode video in-process.** Playback goes through mpv. Embedding is
   a future, separate decision; see [ADR 0001](docs/decisions/0001-external-mpv-over-ipc.md).
3. **Be lenient with playlists.** Real IPTV input is malformed. Prefer
   recovering over erroring, and add a test case for every new quirk.
4. **Test without mpv.** Unit tests must not need an mpv binary, network or a
   TTY. Use `UnixStream::pair()` fakes for IPC and the `Playback` trait fake
   for app logic.
5. **No hard-coded colours.** The TUI uses the terminal palette and
   modifiers; the GUI takes every colour from `theme::Palette` (the user's
   `theme.toml` or a built-in theme). Either way the user's theme applies.
   Building needs GTK4 dev files (`libgtk-4-dev` / Arch `gtk4`).
6. **Never leak credentials.** No `--password` flags, no request/stream URLs
   in errors or logs, `host:port` only in labels. See
   [ADR 0004](docs/decisions/0004-credentials.md).
7. **Never exceed the stream limit.** Front-ends start, stop and switch
   playback only through `tuner_app::Controller`, which calls the
   `StreamGate`. Never call the player directly. Providers ban accounts over this. See
   [docs/stream-limits.md](docs/stream-limits.md).
8. **Keep dependencies light.** Justify new crates in the PR description;
   no async runtime until a feature actually needs one.
9. **Docs move with code.** If you change behaviour covered by a doc in the
   table above, update that doc in the same change. Significant design choices
   get a new ADR.
10. Commits use [Conventional Commits](https://www.conventionalcommits.org/)
   (`feat:`, `fix:`, `docs:`, `ci:`, `refactor:`, `test:`, `chore:`).

## Out of scope right now

EPG/XMLTV, Stalker portals, persistence (SQLite),
embedded video in the GUI. They are planned; see [docs/roadmap.md](docs/roadmap.md).
Do not start them unless asked, but do not make choices that block them (e.g. keep
`Channel::tvg_id`, which the EPG will join on).
