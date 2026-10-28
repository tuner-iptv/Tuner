#!/usr/bin/env bash
# Build release binaries plus .deb and .rpm packages into dist/.
#
# CI runs this on Ubuntu 24.04 (the oldest base with GTK >= 4.10), so the
# packages work on Ubuntu 24.04+, Debian 13+ and Fedora 40+. Locally, use
# --docker to build in the same environment regardless of your distro:
#
#   packaging/build-packages.sh --docker
#
# See docs/packaging.md.
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ "${1:-}" == --docker ]]; then
    exec docker run --rm -v "$PWD":/src -w /src \
        -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" ubuntu:24.04 bash -c '
        set -euo pipefail
        apt-get update -qq
        apt-get install -y -qq --no-install-recommends \
            curl ca-certificates build-essential pkg-config libgtk-4-dev libmpv-dev >/dev/null
        curl -sSf https://sh.rustup.rs | sh -s -- -y -q --profile minimal >/dev/null
        . "$HOME/.cargo/env"
        CARGO_TARGET_DIR=/tmp/target packaging/build-packages.sh
        chown -R "$HOST_UID:$HOST_GID" dist'
fi

target="${CARGO_TARGET_DIR:-target}"
cargo build --release --locked -p tuner -p tuner-gtk
command -v cargo-deb >/dev/null || cargo install --locked --quiet cargo-deb
command -v cargo-generate-rpm >/dev/null || cargo install --locked --quiet cargo-generate-rpm

mkdir -p dist
cargo deb -p tuner-gtk --no-build --output dist/
cargo generate-rpm -p crates/tuner-gtk --target-dir "$target" -o dist/
ls -l dist
