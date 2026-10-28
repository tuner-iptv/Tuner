#!/usr/bin/env bash
# Point tuner-iptv-bin at a published release: sets pkgver, resets pkgrel and
# takes the checksum from the release's .sha256 asset.
# Usage: packaging/aur/bump-bin.sh X.Y.Z
set -euo pipefail

version=${1:?usage: bump-bin.sh X.Y.Z}
version=${version#v}
pkgbuild="$(dirname "$0")/tuner-iptv-bin/PKGBUILD"
asset="tuner-v$version-x86_64-linux.tar.gz"

sum=$(gh release download "v$version" -R tuner-iptv/Tuner -p "$asset.sha256" -O - | awk '{print $1}')
[[ $sum =~ ^[0-9a-f]{64}$ ]] || { echo "bad checksum for $asset" >&2; exit 1; }

sed -i \
  -e "s/^pkgver=.*/pkgver=$version/" \
  -e "s/^pkgrel=.*/pkgrel=1/" \
  -e "s/^sha256sums=.*/sha256sums=('$sum')/" \
  "$pkgbuild"
echo "tuner-iptv-bin -> $version ($sum)"
