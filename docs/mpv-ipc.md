# mpv integration

Implemented in `crates/tuner-mpv`. Decision record:
[decisions/0001-external-mpv-over-ipc.md](decisions/0001-external-mpv-over-ipc.md).

## Spawning

`Mpv::spawn` runs:

```
mpv --idle=yes --force-window=no \
    --input-ipc-server=$XDG_RUNTIME_DIR/tuner-mpv-<pid>-<nanos>.sock \
    --title='tuner: ${media-title}' \
    --wayland-app-id=tuner-player --x11-name=tuner-player \
    --network-timeout=10 --ytdl=no \
    <user --mpv-arg values…>
```

- `--idle` keeps one mpv process between channels. `--force-window=no` means
  **no window until the first frame**: while a stream connects there is
  nothing to see (the front-ends show the step, progress and elapsed time),
  and an unreachable stream never shows a black window.
- Once a stream plays, `Mpv::poll` sets `force-window=yes`, so audio-only
  streams get a window too, and `Mpv::load` sets it back to `no` before the
  next `loadfile`. mpv keeps a video window (showing the last frame) while
  zapping to the next channel; if that channel fails, the window closes.
- `--network-timeout=10` (mpv's default is 60 s) makes a silent server fail
  quickly, with mpv's own reason; see *Detecting failed streams*.
- `--ytdl=no`: by default, when ffmpeg fails to open an HTTP URL, mpv's
  `ytdl_hook` retries it with yt-dlp. For IPTV that only re-requests the
  stream URL (one more connection on the provider's account) and adds yt-dlp's
  own 20 s timeout: an Xtream redirect to a dead streaming server took 30 s
  to fail instead of 10 s.
- User arguments come last so they override tuner's defaults. The user's
  `~/.config/mpv/mpv.conf` (hwdec, profiles, …) applies as usual.
- tuner polls for the socket for up to 5 s, failing early if mpv exits.
- On drop: send `quit`, wait up to 1 s, then kill; remove the socket file.

`Player` wraps an `Option<Mpv>`: it spawns on first `play`, and if the user
closed the mpv window (process gone or socket broken) it respawns and retries
once. `toggle_pause` is a no-op when mpv is not running. `shutdown` quits
mpv; the front-ends' **Stop** uses it, so stopping closes the mpv window
instead of leaving an idle player behind.

## Protocol

Newline-delimited JSON over the Unix socket
([mpv docs](https://mpv.io/manual/stable/#json-ipc)).

Request: `{"command": ["loadfile", "<url>", "replace"], "request_id": 7}`

Reply: `{"request_id": 7, "error": "success", "data": …}`

mpv also pushes events (`{"event": "start-file", …}`) on the same socket at any
time. `Connection::command` reads lines until it sees its own `request_id`,
queueing events it passes. `Connection::poll` reads whatever has arrived
without blocking; `take_events` hands the queue over (capped at 256).
Reads time out after 5 s.

Commands in use:

| Purpose      | Command                           |
| ------------ | --------------------------------- |
| Play channel | `["loadfile", url, "replace"]`    |
| Pause toggle | `["cycle", "pause"]`              |
| Pause state  | `["get_property", "pause"]`       |
| Cache fill   | `["observe_property", 1, "demuxer-cache-state"]` (at spawn) |
| Error detail | `["request_log_messages", "warn"]` (at spawn) |
| Window       | `["set_property", "force-window", bool]` |
| Stop / quit  | `["quit"]` (via `Player::shutdown`) |

## Adding a command

1. Add a method on `Mpv` that calls `self.command(&[json!(...), …])`.
2. If the UI needs it, add it to `Player` (respawn semantics!) and to the
   `Playback` trait and `Controller` in `crates/tuner-app/src/playback.rs`.
3. Test protocol behaviour with the `UnixStream::pair()` fake in `ipc.rs`.

## Detecting failed streams

Without this an unreachable stream leaves mpv's black, idle window open (and
holding the provider slot). `load.rs` follows each `loadfile` through three
stages, each with its own failure signal instead of one fixed timeout:

| Stage        | Entered on                        | Fails when                                                    |
| ------------ | --------------------------------- | ------------------------------------------------------------- |
| `Connecting` | `loadfile` accepted               | `end-file` error (refused, 404, mpv's 10 s network timeout…); last-resort `CONNECT_TIMEOUT` (60 s) |
| `Buffering`  | `file-loaded` (stream opened)     | `end-file` error, or **no new data** in the demuxer cache for `NO_DATA_TIMEOUT` (8 s) |
| `Playing`    | `playback-restart` (first frame)  | not watched; `end-file` reports `Ended` (stalls are left to mpv) |

- Our file is the `playlist_entry_id` from the `loadfile` reply (mpv 0.38+),
  or the first `start-file` after it on older mpv. Events queued before the
  reply belong to the previous file and are dropped.
- `Mpv::spawn` sends `observe_property 1 demuxer-cache-state`; mpv pushes it
  on change. A changed `total-bytes` means data arrived, so the no-data clock
  restarts and a slow stream that keeps receiving is never cut off.
  (`cache-speed` is not used: for HLS it only counts the playlist, because
  ffmpeg fetches the segments itself.)
- Connecting gets no short timeout: a slow HLS provider can take 15 s or more
  (playlist, then segments, each a request) while working fine. mpv's
  network timeout already fails a server that goes silent for 10 s.
- ffmpeg's `warn`/`error` log lines (`log-message` events with an `ffmpeg…`
  prefix) before the first frame are kept as the *problem*, e.g.
  `HTTP error 403 Forbidden` or `Connection to tcp://host:port failed:
  Connection timed out`. Every URL in them is cut to `scheme://host:port`,
  because stream URLs carry credentials ([ADR 0004](decisions/0004-credentials.md)).
- `end-file` for our entry: `reason: "error"` → `StreamEnd::Failed` (the
  problem, then mpv's `file_error`); `"eof"` → `Ended` if it played, else `Failed`;
  `"redirect"` (a playlist link) → follow the next `start-file`;
  `"stop"`/`"quit"` → nothing to report.

Typical times: refused connection or HTTP error, under a second; a server
that accepts and goes silent, or a redirect to an unreachable streaming
server, ~10 s (mpv's network timeout); opened but no data, 8 s after the
data stops.

`Player::poll` (non-blocking) reports each outcome once, and
`Player::progress` the stage, elapsed time and cache fill (`total-bytes`,
`cache-duration`). The front-ends call `Controller::tick` about once a
second and refresh the status line with `tuner_app::status_line` until the
first frame:

```
Connecting to BBC News… · step 1/2 · 4 s
Buffering BBC News… 640 KB, 1.5 s of video · step 2/2 · 7 s
Connecting to BBC News… · step 1/2 · 9 s · HTTP error 503 Service Unavailable
Playing BBC News
```

and and on `Failed`/`TimedOut`/`Stalled`/`Ended` the `Controller`
quits mpv and releases the stream slot, and the front-end shows a
*Stream unavailable* popup (or, for `Ended`, a status line).

## Future: observing state

To show live info (buffering, bitrate, now-playing title) use
`observe_property` and read the events `Connection` already queues.
