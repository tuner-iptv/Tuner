# 0001. External mpv over JSON IPC

- Status: Accepted
- Date: 2026-10-27

## Context

Video decoding is the hardest and riskiest part of an IPTV player. IPTV
streams are often malformed (broken MPEG-TS, odd HLS, dropping servers).
mpv, built on FFmpeg, handles them robustly and already supports hardware
decoding on Wayland. Embedding video (libmpv render API in GTK4/Slint, or
GStreamer) is possible but adds GUI and GL plumbing before anything is usable.

A tiling window manager places a separate mpv window next to the terminal
automatically, and on other desktops it is just another window, so an
external window costs little UX.

## Decision

tuner spawns `mpv --idle --force-window --input-ipc-server=<socket>` and
controls it with JSON IPC commands. One mpv window is reused across channel
changes. tuner never links libmpv or decodes video.

## Consequences

- Robust playback and the user's own `mpv.conf` for free.
- No `unsafe`, no C bindings, CI does not need mpv.
- Video cannot be drawn inside the TUI. A future embedded GUI would be a new
  front-end crate (likely GTK4 + libmpv render API) reusing `tuner-core`,
  with its own ADR. GStreamer is not preferred because of weaker tolerance
  of broken IPTV streams.
- Player state (buffering, errors) must be read from mpv events; not done yet.
