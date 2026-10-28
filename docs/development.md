# Development

## Prerequisites

- Rust stable ≥ 1.92 (edition 2024), e.g. via [rustup](https://rustup.rs)
  (Arch: `sudo pacman -S rustup && rustup default stable`)
- `clippy` and `rustfmt` components (`rustup component add clippy rustfmt`)
- `mpv` for running the app (not needed for tests)
- GTK4 development files for `tuner-gtk` (Arch: `gtk4`; Ubuntu:
  `libgtk-4-dev`; Fedora: `gtk4-devel`)

## Commands

| Task                | Command                                                   |
| ------------------- | --------------------------------------------------------- |
| Build               | `cargo build`                                             |
| Test everything     | `cargo test --workspace`                                  |
| Test one crate      | `cargo test -p tuner-core`                                |
| Smoke test real mpv | `cargo test -p tuner-mpv -- --ignored` (needs mpv; not in CI) |
| Format              | `cargo fmt --all`                                         |
| Lint (as CI does)   | `cargo clippy --workspace --all-targets -- -D warnings`   |
| Run (TUI)           | `cargo run -p tuner -- <source name, path or URL>`        |
| Run (GUI)           | `cargo run -p tuner-gtk -- <source name, path or URL>`    |
| GUI, embedded video | `cargo run -p tuner-gtk --features embedded -- <source>` (needs libmpv) |
| Run with mpv flags  | `cargo run -p tuner -- list.m3u --mpv-arg=--hwdec=auto`   |
| Release build       | `cargo build --release` → `target/release/tuner`          |

`examples/sample.m3u` contains a few public test streams for manual checks.

## Layout

```
crates/
  tuner-core/   pure domain: Channel, m3u parser, Library
  tuner-mpv/    mpv process + JSON IPC
  tuner-app/    front-end independent: config, sources (HTTP), stream limits
  tuner/        binary: CLI + ratatui TUI
  tuner-gtk/    binary: GTK4 desktop GUI
packaging/      desktop entry
docs/           this folder
examples/       sample playlists
```

See [../ARCHITECTURE.md](../ARCHITECTURE.md) for what may depend on what.

## Project description

Use **Tuner** for the app name, `tuner` / `tuner-gtk` for the commands, and
`tuner-iptv` for the distribution package (`tuner-iptv-bin` / `tuner-iptv-git`
for the Arch recipes).

Short description, used at the top of the README:

> A lean, Linux-native IPTV player written in Rust.

GitHub About description:

> A lean, Linux-native IPTV player written in Rust. Keyboard-first terminal and GTK4 interfaces, powered by mpv.

GitHub topics: `rust`, `linux`, `iptv`, `m3u`, `xtream-codes`, `gtk4`, `tui`, `mpv`.

Describe the focus through concrete features: native interfaces, direct playlist
opening, shared settings, and external mpv playback. Performance numbers need a
reproducible benchmark with the build, hardware, input, and measurement method;
distinguish the terminal binary from the GUI and mpv's resource use.

## Conventions

- Workspace-wide lints live in the root `Cargo.toml` (`[workspace.lints]`);
  every crate opts in with `[lints] workspace = true`. `unsafe_code` is
  forbidden.
- Shared dependency versions go in `[workspace.dependencies]`; crates use
  `dep.workspace = true`.
- Errors: `thiserror` enums in `tuner-core` and `tuner-mpv`; `anyhow` in
  `tuner-app` (application glue) and the binary.
- Tests live next to the code (`#[cfg(test)] mod tests`). They must not need
  mpv, a network or a TTY.
- Commits: Conventional Commits. Branch, open a PR, let CI pass, then merge.

## Window manager tips

The mpv window uses Wayland app-id (and X11 class) `tuner-player`, so a
window rule matching `tuner-player` (Hyprland, Sway, i3, KWin, …) can
float, size or place it. A tiling window manager puts it next to the
terminal running tuner, which is the intended layout.

A launcher bind just needs to run tuner in a terminal, e.g.
`<terminal> -e tuner ~/iptv/list.m3u`.
