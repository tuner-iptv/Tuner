# Sources and config

A **source** is where channels come from. tuner supports:

| Type     | Login                          | Status                         |
| -------- | ------------------------------ | ------------------------------ |
| `m3u`    | playlist file or URL           | ✅ (see [m3u-format.md](m3u-format.md)) |
| `xtream` | server + username + password   | ✅ Live TV, movies, series      |
| `stalker`| portal URL + MAC address       | ⏳ planned ([roadmap](roadmap.md)) |

Code: config in `crates/tuner-app/src/config.rs`, fetching in
`crates/tuner-app/src/source.rs`, parsing in `tuner-core` (`m3u`, `xtream`).

## Choosing a source

```sh
tuner                       # first [[source]] in the config
tuner provider              # [[source]] named "provider"
tuner ~/list.m3u            # anything else is an M3U path or URL
tuner --config other.toml   # use a different config file
```

## Managing accounts

In the GUI, **Manage accounts** (`Ctrl+,`) adds, edits, tests and removes
accounts and writes this file for you (see [gui.md](gui.md#accounts-window)).
The TUI reads the same file. Press `a` to select another configured account
without restarting tuner (see [keybindings.md](keybindings.md#account-switcher)).
The picker uses the config loaded at startup, including `--config`, and does
not edit it or change the default account. Hand edits are fine too, but tuner does not
preserve comments when it saves, and keeps the previous file as
`config.toml.bak`.

## Config file

`$XDG_CONFIG_HOME/tuner/config.toml` (usually `~/.config/tuner/config.toml`).
A missing file is fine; unknown top-level keys are an error, to catch typos.

```toml
# theme = "catppuccin-mocha"   # GUI colour scheme; default: theme.toml (see gui.md#theming)
# check_updates = false        # skip the daily GitHub release check (see decisions/0007)

[mpv]
# program = "mpv"
args = ["--hwdec=auto"]          # before any --mpv-arg from the CLI

[[source]]
name = "provider"
type = "xtream"
server = "http://line.example.com:8080"   # scheme optional; /player_api.php is stripped
username = "myuser"
password_command = "pass show iptv/provider"
# max_streams = 1                        # optional; default: provider's limit, else 1
# limit_mode = "block"                    # optional; default "confirm" (see stream-limits.md)

[[source]]
name = "free"
type = "m3u"
location = "https://iptv-org.github.io/iptv/index.m3u"
```

`max_streams` (any source type) caps simultaneous streams for that account.
See [stream-limits.md](stream-limits.md).

### Credentials

Xtream sources need exactly one of:

- `password = "…"`: plain text. Protect the file with `chmod 600`.
- `password_command = "…"`: run with `sh -c`; the first line of stdout is
  the password. Works with `pass`, `secret-tool lookup …`, `bw get password …`,
  `op read …`, etc. Preferred.

There is intentionally **no `--password` CLI flag**: it would leak into shell
history and `ps`. See [ADR 0004](decisions/0004-credentials.md).

tuner never prints credentials. Errors and status messages show only
`host:port`. `Credentials`' `Debug` output redacts the password. Note that
Xtream **stream URLs embed the password in the path** by protocol design, so
they are passed to mpv as-is. Treat mpv logs and watch-history as sensitive.

## Xtream Codes details

All calls are `GET {server}/player_api.php?username=…&password=…[&action=…]`:

| Call                          | Used for                                    |
| ----------------------------- | ------------------------------------------- |
| no action                     | login: `user_info.auth == 1`, `status`, expiry, connections, allowed formats |
| `get_live_categories`         | category id → name (becomes channel group)  |
| `get_live_streams`            | channels: `stream_id`, `name`, `stream_icon`, `epg_channel_id`, `category_id` |
| `get_vod_categories`, `get_vod_streams` | movies: `stream_id`, `name`, `stream_icon`, `category_id`, `container_extension` |
| `get_series_categories`, `get_series`   | series list: `series_id`, `name`, `cover`, `plot`, `releaseDate`, `rating`, `category_id` |
| `get_series_info&series_id=…` | one series' episodes: `episodes` by season, each with `id`, `episode_num`, `title`, `container_extension` |

Live stream URL: `{server}/live/{username}/{password}/{stream_id}.{ext}`, with
path segments percent-encoded. `ext` is `ts` when the account allows it (best
for live TV in mpv), else the first allowed format (usually `m3u8`).

Movie URL: `{server}/movie/{username}/{password}/{stream_id}.{container_extension}`.
Episode URL: `{server}/series/{username}/{password}/{episode_id}.{container_extension}`.
The extension comes from each entry (default `mp4`), not from the account's
allowed live formats.

### Movies and series

Catalogues can hold tens of thousands of entries, so they are **not** fetched
at start-up. Front-ends load movies or series the first time the user opens
that section, and a series' episodes when it is opened, on a worker thread.
Each is fetched once per run. They need no login call and don't count as a stream.

### Loading in batches

The big answers (`get_live_streams`, `get_vod_streams`, `get_series`) are one
JSON array that can be tens of megabytes. `tuner-app` parses it element by
element as the body downloads (`batches::read_array`) and hands entries on in
batches, flushed every 150 ms or 5 000 entries, so the first ones are on screen
long before the download ends. The categories are fetched first (they are
small), then each element goes through `tuner_core::xtream::{live_channel,
movie, series}`; the whole-string `parse_*` functions use the same builders.

- `Source::open()` logs in (Xtream; nothing for M3U) and returns `Opened`
  (connection limit, summary details), so a front-end can set up the account
  before any channel arrives. `Source::channels(&opened, on_batch)` then
  streams them. An M3U playlist arrives as one batch.
- `Catalog::movies_batched` / `series_batched` do the same for the catalogues;
  `movies()` / `series()` collect them, and `Source::load()` is `open` plus
  all of `channels`.
- `on_batch` returns `tuner_app::Flow`: `Break` drops the connection and stops
  reading (e.g. the user switched account). Entries already delivered stay;
  an error part-way returns `Err` after them.
- `Library::extend` appends a batch without invalidating indices already
  handed out, and keeps group sizes up to date.

The GTK window uses this (see [gui.md](gui.md#threads)); the TUI still waits for
the whole list behind its loading overlay.

Movies are plain `Channel`s (category = group, poster = logo) so they browse,
search and play exactly like live channels. A series is a `tuner_core::Series`
(not playable); its episodes are `Channel`s named `S02E05  Title` whose group
is `Season 2`. The `Catalog` trait in `tuner-app::source` is what front-ends
call; `Source` implements it for Xtream logins **and** M3U `get.php?username=…`
links, and reports `has_vod() == false` for any other playlist.

The status bar shows the source name, channel count, expiry date and
`active/max` connections from the login response.

### Tolerated panel quirks

Panels are inconsistent, so `tuner-core::xtream` reads JSON through
`serde_json::Value` and accepts:

- ids, `auth`, `exp_date`, `max_connections` as numbers **or** strings;
- `null` / empty strings for optional fields (name, icon, EPG id, category);
- `{}` or `null` instead of `[]` for empty lists;
- `exp_date: null` or `0` = no expiry; missing `status` = Active.

Streams without a `stream_id` are skipped; a missing name becomes
`Stream <id>`. Add a unit test in `xtream.rs` for every new quirk.

### Not yet supported

- Movie / series details beyond title, year, rating, plot (`get_vod_info`),
  trailers, cast, resume position
- Catch-up / timeshift, EPG (`get_short_epg`, `xmltv.php`)
- Custom HTTP user agent (tuner sends `tuner/<version>`)

## Adding a source type

1. Pure parsing in a new `tuner-core` module, returning `Vec<Channel>`, with
   tests on captured JSON/text.
2. A `SourceKind` variant in `tuner-app`'s `config.rs` and a `Source`
   variant in `source.rs` that fetches over HTTP, calls the parser, and
   implements `account_key()` / `usage()` for stream limits.
3. Document it here and in the table at the top.
