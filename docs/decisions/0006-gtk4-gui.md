# 0006. GTK4 desktop front-end, mpv still external

- Status: Accepted
- Date: 2026-10-27

## Context

Users want an IPTVnator-style desktop window next to the TUI. Many Linux
desktops theme apps from a palette file rather than a single toolkit theme. libadwaita apps ignore custom themes; plain GTK4 can be styled
freely with CSS. Embedding video in-window is wanted as an *option* later.

Options considered: GTK4 (gtk4-rs), iced, Slint.

## Decision

- New crate `tuner-gtk`: plain GTK4 (no libadwaita), styled with CSS
  generated from a palette (a user theme file or a built-in theme) and
  live-reloaded when the file changes.
- Playback stays in an external mpv window (ADR 0001). All play/stop and
  stream-limit logic goes through the shared `tuner_app::Controller`, the
  same code the TUI uses.
- GTK4 is chosen over iced/Slint mainly because embedding mpv later is a
  proven path there (libmpv render API into a `GLArea`, as Celluloid does).

## Consequences

- Building the workspace needs GTK4 development files (CI installs
  `libgtk-4-dev`). MSRV rises to 1.92 (gtk-rs 0.11).
- Embedded video, when added, will be an opt-in player mode in `tuner-gtk`
  implementing the same `Playback` trait, behind a cargo feature so default
  builds don't need libmpv. It needs a small, isolated `unsafe` exception and
  its own ADR.
- Two front-ends to keep in sync. Mitigated by keeping behaviour in
  `tuner-app`; front-ends only render.
