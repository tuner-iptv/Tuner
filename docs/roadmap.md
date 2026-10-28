# Roadmap

Milestones are ordered; scope inside one may shift. Each item should land as
its own PR with tests and doc updates.

## v0.1: MVP (current)

- [x] Lenient M3U parser (file or http(s) URL)
- [x] Group + search filtering
- [x] ratatui TUI with Vim-style keys, theme-neutral colours
- [x] External mpv over JSON IPC, single reused window, auto-respawn
- [x] CI for PRs, tagged releases with Linux binary

## v0.2: daily-driver basics

- [x] Config file `~/.config/tuner/config.toml`: named sources, mpv args
- [ ] Playlist cache on disk; refresh on demand (`r`)
- [ ] Favorites and recently watched (`tuner-store`, SQLite via `rusqlite`)
- [ ] Per-stream `#EXTVLCOPT` user-agent / referrer → mpv options
- [ ] Previous-channel toggle, number-key zapping
- [ ] Background loading thread so the UI never blocks on network

## v0.3: providers and safety

- [x] Stream-limit guard: per-account slot locks + provider connection check
- [x] Channel details pane (`i`)

- [x] Xtream Codes: login, account status/expiry, live categories/streams
- [x] Xtream VOD (movies) and series browsing (TUI and GTK)
- [ ] Stalker / MAG portals (portal URL + MAC)
- [ ] Switch between configured sources inside the TUI
- [ ] Multiple sources side by side

## v0.4: EPG (deferred on purpose)

- [ ] XMLTV fetch (incl. `.gz`), from `url-tvg` header or config
- [ ] Streaming parse with `quick-xml` into SQLite; files can be hundreds of MB,
      so no whole-file DOM
- [ ] Now/next in channel list, program guide view
- [ ] Join on `Channel::tvg_id`, fallback to normalised name
- [ ] Catch-up where the provider supports it (Xtream timeshift URL;
      pick a programme from the guide, or a date/time without EPG)

## GUI

- [x] GTK4 front-end (`tuner-gtk`): groups, logos, search, details, stream-limit dialogs
- [x] Live theming from a custom `theme.toml`
- [x] Account switcher in the header bar
- [x] Accounts window: add, edit, test login, remove, make default
- [ ] Account management in the TUI
- [x] Optional embedded video (libmpv render API, feature `embedded`, ADR 0007)
- [x] Embedded video: own control bar, tracks, fullscreen, open in mpv window
- [ ] Embedded video: mpv keyboard shortcuts

## v1.0

- [ ] Command palette (`Ctrl-K`)
- [ ] Live mpv state in status bar (`observe_property`)
- [ ] AUR package (`tuner-iptv-bin`) built from release artifacts
