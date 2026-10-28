# Tuner documentation

Tuner is a lean, Linux-native IPTV player written in Rust, with terminal and
GTK4 interfaces powered by mpv. Start with the [README](../README.md) for
installation and everyday use.

| Doc                                        | Covers                                              |
| ------------------------------------------ | --------------------------------------------------- |
| [development.md](development.md)           | Toolchain, build/test/lint commands, running locally |
| [ci.md](ci.md)                             | CI workflows, release process, versioning           |
| [website.md](website.md)                   | GitHub Pages site and its in-browser demo (`site/`) |
| [packaging.md](packaging.md)               | Release assets, distro support, .deb/.rpm/AUR, static TUI |
| [sources.md](sources.md)                   | Source types, config file, Xtream API, credentials |
| [m3u-format.md](m3u-format.md)             | Supported M3U syntax and tolerated quirks           |
| [stream-limits.md](stream-limits.md)       | Max simultaneous streams: locks, provider check, prompts |
| [mpv-ipc.md](mpv-ipc.md)                   | How tuner launches and talks to mpv                 |
| [keybindings.md](keybindings.md)           | TUI keys and modes                                  |
| [gui.md](gui.md)                           | GTK4 front-end: layout, keys, theming, threads |
| [roadmap.md](roadmap.md)                   | Planned milestones and their scope                  |
| [decisions/](decisions/README.md)          | Architecture Decision Records (ADRs)                |

Top-level: [../ARCHITECTURE.md](../ARCHITECTURE.md) (code map) and
[../AGENTS.md](../AGENTS.md) (agent entry point and rules).

When you add a doc, add a row here **and** in the "Where to look" table in
AGENTS.md if agents should read it before touching some area.
