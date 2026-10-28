# M3U playlist format

Implemented in `crates/tuner-core/src/m3u.rs`. The parser is **lenient**: IPTV
playlists in the wild are frequently malformed, and rejecting a whole list over
one bad line is worse than skipping or guessing.

## Supported syntax

```
#EXTM3U
#EXTINF:-1 tvg-id="bbc1.uk" tvg-name="BBC One" tvg-logo="http://…/bbc1.png" group-title="UK",BBC One HD
http://provider/stream/123.m3u8
```

| Source                         | Maps to             |
| ------------------------------ | ------------------- |
| text after first unquoted `,`  | `Channel::name`     |
| next non-`#` line              | `Channel::url`      |
| `group-title`                  | `Channel::group`    |
| `#EXTGRP:<group>`              | `Channel::group` (only if no `group-title`) |
| `tvg-logo`                     | `Channel::logo`     |
| `tvg-id`                       | `Channel::tvg_id` (EPG join key, used later) |
| `tvg-name`                     | `Channel::name` fallback when display name is empty |

Attribute keys are case-insensitive. Values may be `"quoted"` (and may then
contain commas and spaces) or bare (`group-title=Sports`). Empty values become
`None`.

## Tolerated quirks

- Missing `#EXTM3U` header.
- UTF-8 BOM, CRLF line endings, leading/trailing whitespace.
- Unknown directives (`#EXTVLCOPT`, `#KODIPROP`, comments) are ignored.
- Bare URL lines without `#EXTINF` become channels named after the URL.
- Display name missing → `tvg-name` → URL.

The only error is a playlist that yields **no** channels.

## Not yet handled

- `#EXTVLCOPT:http-user-agent` / `http-referrer` (would need passing to mpv
  as `--user-agent` / `--referrer` per stream).
- `catchup*` attributes (catch-up TV).
- `url-tvg` / `x-tvg-url` in the header (EPG source URL; needed for EPG).

Every new quirk you handle needs a unit test in `m3u.rs` with a minimal
reproducing input.
