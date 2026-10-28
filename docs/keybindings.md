# Keybindings

Implemented in `crates/tuner/src/app.rs` (`handle_normal_key`,
`handle_search_key`). The hint line is in `crates/tuner/src/ui.rs`. Keep all
three in sync, and the browser demo too (`site/assets/demo.js`, see
[website.md](website.md)).

## Normal mode

| Key                    | Action                                          |
| ---------------------- | ----------------------------------------------- |
| `1` / `2` / `3`        | Show Live / Movies / Series (Movies and Series need an Xtream source; loaded on first use) |
| `j` / `↓`              | Move down in focused pane                       |
| `k` / `↑`              | Move up                                         |
| `PgDn` / `PgUp`        | Move 10                                         |
| `g` / `Home`           | First item                                      |
| `G` / `End`            | Last item                                       |
| `h` / `←`              | Focus groups pane                               |
| `l` / `→`              | Focus channels pane                             |
| `Tab` / `Shift-Tab`    | Toggle pane focus                               |
| `Enter`                | Groups: focus list · List: play (Series: open its episodes) |
| `/`                    | Enter search mode                               |
| `Esc`                  | Clear active search; otherwise leave a series' episodes |
| `Backspace`            | Leave a series' episodes                        |
| `i`                    | Toggle channel details pane                     |
| `u`                    | Show / hide the full stream URL in the details  |
| `a`                    | Open the account switcher                       |
| `Space`                | Toggle pause in mpv                             |
| `s`                    | Stop: quit mpv (closes its window), free slot   |
| `q` / `Ctrl-C`         | Quit (also closes mpv)                          |

Moving in the groups pane filters the channel list live. "All" is the first
group.

## Movies and series

`2` loads the movie catalogue on first use. A "Loading movies…" overlay with a
spinner shows while a worker thread fetches it; keys are ignored until it
arrives, except `q` / `Ctrl-C`. Switching back to a loaded section is instant, apart from a 160 ms
wipe-in: the lists reveal from the top over a few frames after any section,
series or account change (the event loop redraws every 25 ms meanwhile).
The left pane then lists categories. `3` does the same for series. `Enter` on a series fetches its
episodes (same overlay) and lists them with **Seasons** in the left pane; `Esc` or
`Backspace` returns to the series list with the same selection. Switching
sections keeps playing and keeps the stream slot, and an open series stays
open until you leave it. Search and group selection reset when you switch
sections.

## Account switcher

`a` lists the named sources from the config used at startup (including
`--config`). The current account has a `●` marker and is selected initially.
Accounts are shown by name; credentials and playlist URLs are never displayed.

| Key                        | Action                         |
| -------------------------- | ------------------------------ |
| `j` / `↓`, `k` / `↑`       | Move selection                 |
| `PgDn` / `PgUp`            | Move 10                        |
| `g` / `Home`, `G` / `End`  | First / last account           |
| `Enter`                    | Load the selected account      |
| `Esc` / `q` / `a`          | Close without switching        |
| `Ctrl-C`                   | Quit tuner                     |

Switching successfully stops mpv, releases the old account's stream slot,
clears the search and group filter, and loads the new channels and stream
limit. Press `Enter` on a channel to start playback. Selecting the current
account or cancelling keeps playback and filters as they are. If loading
fails, the status line reports the error and the previous account and playback
remain available. While loading, a small spinner shows the selected account;
fetching and password commands run on a worker so the animation stays live.
`q` / `Ctrl-C` still quit. Other keys are ignored until loading finishes,
preventing duplicate switches. The old stream can keep playing during loading;
closing its mpv window still releases the slot. Periodic provider checks resume
after loading finishes.

The picker also works when tuner was started with a playlist path or URL;
that playlist is not added to the saved accounts. With no configured sources,
the picker explains how to add them. Switching does not change the default
account or write the config file; restart tuner to pick up config edits.

## Search mode

Entered with `/`. Filters channels by case-insensitive substring of the name,
within the selected group.

| Key            | Action                              |
| -------------- | ----------------------------------- |
| any character  | Append to query                     |
| `Backspace`    | Delete last character               |
| `↑` / `↓`      | Move selection                      |
| `Enter`        | Play selection, back to normal mode (query kept) |
| `Esc`          | Clear query, back to normal mode    |
| `Ctrl-C`       | Quit                                |

## Details pane

`i` shows details for the selected entry under the list. Live channels: name,
group, number, catch-up days, EPG id, stream format (MPEG-TS / HLS), server
host and whether a logo exists. Movies and episodes: the same minus the live-only
fields. Series: category, year, rating and plot. It follows the selection. The full stream URL is hidden by default because it
usually embeds the account password; `u` reveals it (and hides it again).

## Stream-limit popup

Shown when pressing `Enter` with nothing playing and the stream limit is
reached or can't be verified. See [stream-limits.md](stream-limits.md).

| Popup             | Keys                                          |
| ----------------- | --------------------------------------------- |
| Stream limit      | any key closes; nothing was opened            |
| Confirm           | `y` plays anyway · any other key cancels      |

## Planned

`f` favorite, `Ctrl-K` command palette, number-key channel zapping, mpv
volume/fullscreen passthrough. See [roadmap.md](roadmap.md).
