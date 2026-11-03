#!/bin/sh
# =============================================================================
# nrzube — fetch and verify the Buildroot tarball.
#
# usage: fetch-buildroot.sh <buildroot-dir> <url> <sha256>
# =============================================================================
set -eu

BR2_DIR="${1:?usage: fetch-buildroot.sh <buildroot-dir> <url> <sha256>}"
URL="${2:?usage: fetch-buildroot.sh <buildroot-dir> <url> <sha256>}"
SHA256="${3:?usage: fetch-buildroot.sh <buildroot-dir> <url> <sha256>}"

TARBALL="$BR2_DIR/$(basename -- "$URL")"

mkdir -p "$BR2_DIR"

if [ -f "$TARBALL" ] && echo "$SHA256  $TARBALL" | sha256sum -c --status; then
	echo "fetch-buildroot: using the cached $TARBALL"
else
	echo "fetch-buildroot: downloading $URL"
	curl --fail --location --silent --show-error --retry 3 -o "$TARBALL.part" "$URL"
	echo "$SHA256  $TARBALL.part" | sha256sum -c --status || {
		echo "fetch-buildroot: SHA-256 mismatch for $URL" >&2
		rm -f "$TARBALL.part"
		exit 1
	}
	mv "$TARBALL.part" "$TARBALL"
fi

# The tree is extracted next to the tarball.
if [ ! -f "$BR2_DIR/Makefile" ]; then
	echo "fetch-buildroot: extracting the Buildroot tree"
	tar -xzf "$TARBALL" -C "$BR2_DIR" --strip-components=1
fi

echo "fetch-buildroot: ready in $BR2_DIR"
