# Architecture

Tuner is a lean, Linux-native IPTV player written in Rust. Its terminal and
GTK4 interfaces share application logic and delegate playback to external mpv.

A map of the code for people and agents who need to change it. Conventions and
the task-to-doc index are in [AGENTS.md](AGENTS.md); rationale for big choices
is in [docs/decisions/](docs/decisions/README.md).

## Bird's-eye view

```
┌─────────────────────┐  ┌──────────────────────────┐
│ tuner (bin, TUI)    │  │ tuner-gtk (bin, GUI)     │  front-ends: render
│ main  app  ui       │  │ main window theme logos  │  and route input only
└─────────┬───────────┘  └────────────┬─────────────┘
          └──────────┬────────────────┘
                     ▼ load · Controller (play/stop/limits)
┌─────────────────────────────────────┐   ┌──────────────────────┐        ┌─────────────┐
│ tuner-app                           │──▶│ tuner-mpv            │  JSON  │ mpv process │
│ config  source  guard  playback     │   │ Player → Mpv         │◀──────▶│ (own window,│
└───┬──────────────┬──────────────────┘   │ Connection (IPC)     │  Unix  │  placed by  │
    │ HTTP         │ parse                └──────────────────────┘ socket │  your WM)   │
    ▼              ▼                                                      └─────────────┘
 M3U URL /   ┌──────────────┐
 Xtream      │ tuner-core   │  Channel, m3u, xtream, Library (pure)
 server      └──────────────┘
```

## Crate layering

Dependencies point one way only:

```
tuner, tuner-gtk ──▶ tuner-app ──▶ tuner-core
                         └──────▶ tuner-mpv
```

Front-ends may also use `tuner-core`/`tuner-mpv` types directly. `tuner-core`
and `tuner-mpv` depend on nothing in the workspace and not on each other.
`tuner-app` never depends on a UI crate (ratatui, gtk).

### `crates/tuner-core`: domain, pure

- `channel.rs`: `Channel { name, url, group, logo, tvg_id, number, catchup_days }`.
  Also used for movies and episodes.
- `series.rs`: `Series`, a browsable but not playable series; its episodes are
  `Channel`s fetched on demand.
- `m3u.rs`: lenient extended-M3U parser, `&str → Vec<Channel>`. Format notes
  and accepted quirks: [docs/m3u-format.md](docs/m3u-format.md).
- `xtream.rs`: Xtream Codes `Credentials` (server normalisation, stream URL
  building with percent-encoding, password-redacting `Debug`), plus lenient
  parsing of the login, live-stream, movie, series and episode JSON into
  `Account`, `Vec<Channel>` and `Vec<Series>`, plus per-element builders
  (`live_channel`, `movie`, `series`) for callers that read arrays
  incrementally.
  See [docs/sources.md](docs/sources.md).
- `library.rs`: `Library<T: Entry>` (channels by default, also series), which
  owns entries, derives the group list and sizes and answers
  `filter(query, group) → Vec<usize>`. `extend` appends a batch; indices
  stay valid because entries are only appended.

**Invariant:** no I/O of any kind. No network, filesystem, process, terminal or
clock. JSON and text come in as `&str`; HTTP happens in `tuner-app`. That
keeps it trivially testable and reusable by any front-end.

### `crates/tuner-mpv`: player control, knows nothing about playlists

- `ipc.rs`: `Connection` sends `{"command": [...], "request_id": n}` lines and
  matches replies by `request_id`, queueing interleaved events; `poll` reads
  events without blocking.
- `load.rs`: `LoadWatch` follows a `loadfile` through mpv's events
  (connecting → buffering → playing, watching data reach the cache), reports
  progress for the status line and a `StreamEnd` (failed, timed out,
  stalled, ended). Pure; fed the time.
- `process.rs`:
  - `Mpv` is one spawned `mpv --idle --force-window=no --input-ipc-server=…`
    process. It quits/kills on drop and removes the socket.
  - `Player` spawns mpv lazily and transparently respawns it if the user
    closed the window. This is the UI-facing type.
- Protocol subset and spawn flags: [docs/mpv-ipc.md](docs/mpv-ipc.md).

**Invariant:** never decode or render video in-process. See
[ADR 0001](docs/decisions/0001-external-mpv-over-ipc.md).

### `crates/tuner-app`: front-end independent services

Everything a TUI and a future GUI must behave identically on.

- `config.rs`: `~/.config/tuner/config.toml` (serde + toml). Named
  `[[source]]`s, `password` / `password_command`, `max_streams`,
  `limit_mode`, mpv defaults. `resolve()` maps the CLI argument to a
  `Resolved` source. Also writes the file (`save`, atomic, mode 600, keeps
  `.bak`) and edits accounts (`upsert`, `remove`, `make_default`,
  `validate`) for the front-ends' account managers.
- `source.rs`: `Source::{M3u, Xtream}`. Fetches over HTTP (blocking `ureq`)
  and hands text to `tuner-core`; large Xtream lists are parsed as they
  download (`batches.rs`) and delivered in batches (`open` + `channels`,
  `Catalog::*_batched`). `account_key()` identifies the provider
  account; `usage()` asks for active/max connections. HTTP errors are rebuilt
  without URLs so credentials never leak
  ([ADR 0004](docs/decisions/0004-credentials.md)). The `Catalog` trait
  (implemented by `Source`) fetches movies, series and episodes lazily.
- `guard.rs`: `StreamGuard` (implements the `StreamGate` trait) enforces the
  stream limit with per-account slot file locks and a provider check.
  See [docs/stream-limits.md](docs/stream-limits.md).
- `playback.rs`: the `Playback` trait (implemented for `tuner_mpv::Player`)
  and `Controller<P, G>`, the single place that decides when to check the
  limit, holds/releases the slot, and tracks what is playing. Returns a
  `Request` (`Started` / `NeedsConfirm` / `Blocked` / `Failed`) that
  front-ends render.

### `crates/tuner`: binary, TUI

- `main.rs`: clap CLI, loads config and source, builds the `StreamGuard`,
  owns the terminal and the event loop. Handles account-switch requests by
  loading the new source before replacing the library, catalogue and stream
  gate.
- `app.rs`: all state and key handling. No terminal I/O. Live / Movies /
  Series sections; movies and series load on first visit through a `Catalog`
  (a blocking request deferred to after the "Loading…" frame is drawn), and a
  series drills into its episodes. Plays through a
  `Controller<Box<dyn Playback>, Box<dyn StreamGate>>`, so tests use fakes.
- `ui.rs`: pure rendering from `App` state. Uses only terminal palette colours
  and modifiers so the terminal's theme applies.

The account picker lists configured sources by name. A successful switch
stops playback through the `Controller` before replacing the gate; failed
loads leave the previous account available. Account loading and password
commands run on a worker thread, returning data over a standard-library
channel. A modal spinner animates while the TUI waits for the result.

### `crates/tuner-gtk`: binary, GTK4 GUI

- `main.rs`: same CLI as the TUI; resolves config before opening a window.
- `window.rs`: widgets, signals, shortcuts, stream-limit dialogs. Live /
  Movies / Series sections (catalogues fetched lazily through `Catalog` and
  appended to the view batch by batch, a series drills into its episodes). Blocking work runs on `gio::spawn_blocking`; the `Controller` sits in an
  `Arc<Mutex<…>>`.
- `theme.rs`: the user's `theme.toml` → CSS, live reload via file monitors, or
  a pinned built-in palette (`themes.rs`) chosen by `theme` / `--theme`.
- `logos.rs`: background logo fetch + texture cache for recycled list rows.
- `accounts.rs`: the Accounts window. Edits `tuner_app::Config` (validate,
  upsert, remove, make default) and saves it; asks `window.rs` to reopen or
  switch sources.

Windows own their state: every signal handler holds a `Weak` reference and
exactly one strong reference lives in the window's `close-request` handler.
Without it the state is freed after setup and input silently does nothing.

See [docs/gui.md](docs/gui.md).
## Runtime model

TUI: `event::poll` waits up to 500 ms (100 ms while an account loads), a
key mutates `App`, `App::tick` notices a closed mpv window or a failed
stream, the frame is redrawn. Account switches use a worker thread and a channel; the event loop
polls for completion without waiting and applies results through `Controller`.
Periodic blocking provider checks are deferred during account loading to keep
the spinner and quit keys responsive. GUI: GTK main loop; blocking calls go to GIO worker threads (see
[docs/gui.md](docs/gui.md#threads)). mpv calls are blocking with a 5 s reply timeout. The first
`play` may block up to 5 s while mpv starts. That is acceptable for the MVP.
If background work (EPG downloads, Xtream API) needs it later, introduce a
worker thread with a channel into the event loop before reaching for an async
runtime.

## Where future features go

| Feature             | Lives in                                                    |
| ------------------- | ----------------------------------------------------------- |
| Stalker portals     | new `tuner-core::stalker` (parsing) + `Source::Stalker` in `tuner-app` |
| EPG (XMLTV)         | new `tuner-epg` crate: streaming parse (`quick-xml`) into storage; joins on `Channel::tvg_id` |
| Favorites / history | new `tuner-store` crate (SQLite) |

Details and ordering: [docs/roadmap.md](docs/roadmap.md).

## Testing strategy

- `tuner-core`: plain unit tests on strings (captured M3U text, Xtream JSON).
- `tuner-app`: unit tests on TOML strings, account keys, URL redaction and
  stream-guard locking (temp lock dirs). HTTP is checked by hand against a
  real or mock panel (not in CI).
- `tuner-mpv`: `UnixStream::pair()` fakes the mpv side of the socket. No real
  mpv is required in CI.
- `tuner-app::playback`: `Controller` flows against fake player and gate.
- `tuner`: `App` driven by synthetic `KeyEvent`s against `FakePlayer` and
  `FakeGate`.
- `tuner-gtk`: palette parsing and CSS generation are unit tested; the window
  is checked by hand (`cargo run -p tuner-gtk`).
- Manual end-to-end: `cargo run -p tuner -- examples/sample.m3u`.
