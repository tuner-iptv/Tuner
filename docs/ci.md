# CI and releases

GitHub Actions workflows live in `.github/workflows/`. They use
`dtolnay/rust-toolchain` and `Swatinem/rust-cache` to stay fast.

## `ci.yml`: every PR and push to `main`

One job on `ubuntu-latest` (installs `libgtk-4-dev libmpv-dev` first):

1. `cargo fmt --all --check`
2. `cargo clippy --workspace --all-targets --locked -- -D warnings`
3. `cargo test --workspace --locked`
4. `cargo clippy -p tuner-gtk --no-default-features --all-targets --locked --
   -D warnings`, so the GUI without embedded video
   ([ADR 0008](decisions/0008-embedded-video.md)) keeps building.
5. An MSRV job builds with the `rust-version` from `Cargo.toml` (1.92, set by gtk-rs).

Superseded runs on the same branch are cancelled. `Cargo.lock` is committed
and `--locked` makes CI fail if it is out of date. Run `cargo build` and commit
the lockfile.

## `pages.yml`: the website

Publishes `site/` to GitHub Pages on pushes to `main` that touch it, and
checks it on pull requests (JavaScript parses; `themes-data.js` matches
`themes.rs`). Needs **Settings → Pages → Source: GitHub Actions** once.
Details: [website.md](website.md).

## `release.yml`: manual releases and tags `v*`

Run the **Release** workflow on `main` and enter a stable version such as
`0.3.0`. It updates the workspace version and lockfile, commits them to
`main`, and creates the matching `v0.3.0` tag. It then builds the artifacts
from that commit and publishes the release. The workflow token needs write
access to `main` and tags; branch protection can prevent this push.

Existing `v*` tag pushes also run the workflow, provided the tagged commit
already contains the matching workspace version.

1. Checks that the tag matches `workspace.package.version` in `Cargo.toml`
   (tag `v0.2.0` ⇔ `version = "0.2.0"`).
2. Runs the test suite.
3. Builds `tuner` and `tuner-gtk` `--release` for `x86_64-unknown-linux-gnu`.
4. Publishes a GitHub Release with `tuner-<tag>-x86_64-linux.tar.gz`
   (both binaries, the desktop entry, app icon, README) and its `.sha256`, with auto-generated notes.

Before the release job, `build-artifacts.yml` (a reusable workflow) builds
the static musl TUI binaries for x86_64 and aarch64 and the `.deb`/`.rpm`
packages; the release job attaches them too. That workflow also runs on PRs
that touch `packaging/`, `crates/tuner-gtk/Cargo.toml` or the release
workflows. Details: [packaging.md](packaging.md).

After a release, run `packaging/aur/bump-bin.sh X.Y.Z`, commit the PKGBUILD, regenerate
`.SRCINFO` and push it to the AUR (see [packaging.md](packaging.md#aur)).

## Cutting a release

In GitHub, open **Actions → Release → Run workflow**, select `main`, and
enter the version without a `v`. An existing tag is never overwritten. If a
build or publication fails after the tag is created, rerun the failed jobs in
that workflow run instead of starting a new run for the same version.

Versioning follows SemVer. While on 0.x, breaking CLI or behaviour changes bump
the minor version.
