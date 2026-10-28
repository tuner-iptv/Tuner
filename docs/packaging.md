# Packaging and distribution

What each release ships, which distros it covers, and how packages are built
and published. Workflows: `.github/workflows/release.yml` (tags) and
`.github/workflows/build-artifacts.yml` (also runs on PRs touching packaging).

## Release assets

| Asset | Contents | Runs on |
| ----- | -------- | ------- |
| `tuner-vX.Y.Z-x86_64-linux.tar.gz` | `tuner`, `tuner-gtk`, desktop entry, app icon, README | glibc ≥ 2.39 and GTK ≥ 4.10 (Ubuntu 24.04+, Debian 13+, Fedora 40+, Arch) |
| `tuner-tui-vX.Y.Z-x86_64-linux-musl.tar.gz` | static `tuner` (TUI only) | any x86_64 Linux |
| `tuner-tui-vX.Y.Z-aarch64-linux-musl.tar.gz` | static `tuner` (TUI only) | any ARM64 Linux (Raspberry Pi 4/5, ARM laptops) |
| `tuner-iptv_X.Y.Z-1_amd64.deb` | both front-ends | Ubuntu 24.04+, Debian 13+ |
| `tuner-iptv-X.Y.Z-1.x86_64.rpm` | both front-ends | Fedora 40+ |

Every asset has a `.sha256` next to it. All of them need `mpv` installed;
the `.deb`/`.rpm` pull it in as a dependency.

The app is **Tuner**, and the distribution package is **`tuner-iptv`**.
Keep this distinction in package descriptions and install instructions.
The commands are `tuner` and `tuner-gtk`.

## Why these choices

- **Static musl TUI.** No glibc or GTK dependency, so the terminal UI runs
  on any distro, old or new, and on ARM. TLS is rustls + `ring`, which
  builds cleanly for musl. Cross-compiled with `cargo-zigbuild`.
- **One `.deb` / `.rpm` with both binaries.** Built on Ubuntu 24.04, the
  oldest base with GTK ≥ 4.10 (`gtk::AlertDialog`). Dependencies are
  declared explicitly (`mpv`, `libmpv2`, `libgtk-4-1 (>= 4.10)` /
  `mpv`, `mpv-libs`, `gtk4 >= 4.10`). The GUI links libmpv for its
  embedded video ([ADR 0007](decisions/0007-embedded-video.md)).
- **No Flatpak or AppImage for now.** Flatpak would need mpv built inside
  the sandbox and remapped config/theme paths; not worth it yet. Native
  packages cover the main distros.

## Building locally

```sh
# .deb + .rpm into dist/, built in an Ubuntu 24.04 container (needs Docker)
packaging/build-packages.sh --docker

# static TUI, any target (needs Docker)
docker run --rm -v "$PWD":/src -w /src ghcr.io/rust-cross/cargo-zigbuild \
  sh -c 'rustup target add aarch64-unknown-linux-musl &&
         cargo zigbuild --release -p tuner --target aarch64-unknown-linux-musl'
```

`packaging/build-packages.sh` without `--docker` builds on the current
machine (what CI does on Ubuntu 24.04). Package metadata lives in
`crates/tuner-gtk/Cargo.toml` under `[package.metadata.deb]` and
`[package.metadata.generate-rpm]`.

## AUR

Two packages in `packaging/aur/`:

| Package | Builds from | Notes |
| ------- | ----------- | ----- |
| `tuner-iptv-bin` | release tarball | pin `pkgver` and `sha256sums` per release |
| `tuner-iptv-git` | `main` branch | compiles with cargo, runs the tests |

Both conflict with the AUR `tuner` radio player, which also installs
`/usr/bin/tuner`.

Updating for a release:

```sh
packaging/aur/bump-bin.sh X.Y.Z  # sets pkgver, pkgrel=1 and the checksum (needs gh)
cd packaging/aur/tuner-iptv-bin
makepkg --printsrcinfo > .SRCINFO
makepkg -f                       # test build
```

Publishing (one-time setup: an AUR account with your SSH key):

```sh
git clone ssh://aur@aur.archlinux.org/tuner-iptv-bin.git aur-tuner-iptv-bin
cp packaging/aur/tuner-iptv-bin/{PKGBUILD,.SRCINFO} aur-tuner-iptv-bin/
cd aur-tuner-iptv-bin && git add -A && git commit -m "Update to X.Y.Z" && git push
```

## Before publishing anywhere

- **The repository must be public.** The AUR, and anyone downloading
  `.deb`/`.rpm` from release URLs, fetch from GitHub without credentials.
- The license is **GPL-3.0-or-later** (`LICENSE`), set once in
  `[workspace.package]` and repeated in the PKGBUILDs. Every tarball and
  package ships the license text (`.deb`: `/usr/share/doc/tuner-iptv/`,
  `.rpm` and AUR: `/usr/share/licenses/<pkg>/`).
