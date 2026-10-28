# Website

The project site (with an in-browser demo of both interfaces) lives in
`site/` and is published to GitHub Pages by `.github/workflows/pages.yml`. It
is plain HTML, CSS and JavaScript: no framework, no bundler, no npm, no
build step. Files are served exactly as committed.

## Run it locally

```sh
python3 -m http.server -d site 8000   # then open http://localhost:8000
```

## Layout

| Path                         | What it is                                                  |
| ---------------------------- | ----------------------------------------------------------- |
| `site/index.html`            | The page: hero, demo, feature cards, screenshot tour, install |
| `site/assets/style.css`      | Page and demo styles; every colour is a theme variable      |
| `site/assets/theme.js`       | Page light/dark switch; separate theme for the demo         |
| `site/assets/themes-data.js` | **Generated** from `crates/tuner-gtk/src/themes.rs`         |
| `site/assets/data.js`        | Fake channels, movies, series and accounts for the demo     |
| `site/assets/demo.js`        | The demo: shared model, GTK mock, terminal mock, mpv window |
| `site/assets/site.js`        | Nav, screenshot tour, copy buttons, mode switch, release info |
| `site/assets/img/`           | Copies of `docs/images/*.png`                               |
| `site/scripts/gen-themes.py` | Regenerates `themes-data.js`                                |

## Publishing

Pushes to `main` that touch `site/` (or `themes.rs`) deploy automatically.
One-time setup: **Settings → Pages → Source: GitHub Actions**. The site is
then at `https://<owner>.github.io/<repo>/`. All asset paths are relative, so
it works under that sub-path and on a custom domain alike. Pull requests run
only the check job.

The check job fails if a script has a syntax error or if `themes-data.js` no
longer matches `themes.rs`. After editing built-in themes, run:

```sh
python3 site/scripts/gen-themes.py
```

## The demo is a mock, and must stay honest

The demo plays no video and talks to no server. It shows the real layout and
behaviour on made-up data, so it can drift from the apps. When you change
behaviour covered below, update the demo in the same change:

| Real behaviour                                  | Demo code (`demo.js`)                 |
| ----------------------------------------------- | ------------------------------------- |
| TUI keys ([keybindings.md](keybindings.md))     | `Tui.key`, the hint line, key chips   |
| GUI layout and keys ([gui.md](gui.md))          | `Gtk` view and `Gtk.key`              |
| Stream limits ([stream-limits.md](stream-limits.md)) | `act.play`, `act.answer`, `data.js` accounts |
| Built-in themes                                 | regenerate `themes-data.js`           |

The mock follows these rules:

- One shared model (`s`, `act`) drives both views, like both front-ends share
  `tuner_app`. Views only render and route input.
- Playback always goes through `act.play` / `act.start`, which applies the
  stream-limit check. Never start playback anywhere else.
- No real channels, providers or credentials. The fake stream URL is hidden
  until revealed, as in the apps.
- The page has only a light/dark switch. The demo has its own theme picker
  (the app's built-in palettes) and never recolours the page.
- Colours come from CSS variables set by `theme.js`; no hard-coded colours
  in the GTK and TUI mocks (the mpv window and channel logos are the
  exception: they stand in for video and artwork).
- The hero's "now playing" card mirrors the demo's playback state; keep that
  hook (end of `demo.js`) when changing how playback is stored.
- Keep copy short. Sections are a one-line heading plus a visual; put detail in
  `docs/` and link to it.
- The page makes one optional network request, to the GitHub API, for the
  latest release number. It must keep working if that fails.

## Screenshots

`site/assets/img/` holds copies of `docs/images/`. Pages publishes only
`site/`, so refresh the copies when those screenshots change.
