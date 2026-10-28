# 0007. Embedded video via libmpv, still controlled over IPC

- Status: Accepted
- Date: 2026-10-28

## Context

ADR 0006 kept playback in an external mpv window and left embedding as an
opt-in mode for later. Some users want the video inside the GUI window.
GTK4 has no XEmbed and Wayland cannot embed foreign windows, so mpv's
`--wid` is not an option. What works is libmpv's render API drawing into a
`gtk::GLArea`, as Celluloid does. That needs C bindings and `unsafe`, which
the workspace forbids.

## Decision

- A cargo feature `embedded` in `tuner-gtk`, on by default. With it, the
  GUI creates libmpv in-process (`vo=libmpv`) and draws it in a `GLArea`
  beside the channel list. If libmpv or OpenGL fails, the GUI falls back to
  the external mpv window; `--no-default-features` builds a GUI that
  always uses it.
- libmpv's own IPC server (`input-ipc-server`) is switched on, and the
  embedded player talks to it through the same `tuner_mpv::Mpv` as the
  external one (`Mpv::connect`). Load tracking, failure reasons and stream
  limits are shared code, not a second implementation over the C API.
- The C API is used only to create mpv and to render. The declarations are
  written by hand (about ten functions from libmpv, `eglGetProcAddress` for
  GL) rather than adding a bindings crate.
- `unsafe` is allowed in exactly one module, `tuner-gtk/src/embed.rs`.
  `tuner-gtk` sets `unsafe_code = "deny"` instead of inheriting the
  workspace `forbid`, so that module can `#![allow]` it. Every other crate
  still forbids `unsafe`.
- Rule 2 ("never decode video in-process") is narrowed. Decoding is still
  mpv's, never tuner's code, but with this feature mpv runs inside the
  GUI's process.

## Consequences

- Building `tuner-gtk` needs `libmpv-dev` (Arch: `mpv`) and libEGL, and
  the .deb/.rpm declare `libmpv2` / `mpv-libs`. Every distro's mpv package
  already pulls those in. The TUI is unchanged and needs neither. CI also
  lints the `--no-default-features` build so the external-only GUI keeps
  building.
- A crash in libmpv takes the GUI down with it. Embedding is still the
  default, since the window is what GUI users expect; mpv's own window is
  one click away ("Open in mpv window").
- tuner draws its own control bar (play/pause, seek, volume, subtitle and
  audio tracks, fullscreen, "Open in mpv window") instead of mpv's OSC, so
  it follows the user's theme. It sends mpv input commands and follows
  observed properties through the C API, drained on the GTK thread. Pointer
  moves, clicks and the wheel are also forwarded, for mpv's default mouse
  bindings. Keys stay with tuner's UI.
- Fullscreen and cinema mode hide every widget except the video pane (and,
  in cinema mode, the header) rather than moving the `GLArea` elsewhere,
  because re-parenting would recreate the render context mid-stream.
- "Open in mpv window" swaps players inside `Playback::play`, stopping the
  embedded stream before the external one starts, so the controller's
  stream slot carries over and the limit is never exceeded. Movies resume at
  the same position (`start=`), at the cost of fetching once more. Letting
  the in-process libmpv open its own window instead (no refetch) was tried
  and crashed GTK's thread on NVIDIA, with both OpenGL and Vulkan.
- GL proc addresses come from EGL, GTK's default on Wayland and X11. A
  GLX-only setup would need `glXGetProcAddressARB`.
