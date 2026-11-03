#!/bin/sh
# =============================================================================
# nrzube — build U-Boot out of tree.
#
# Buildroot 2026.08 does not ship the U-Boot package any more: the bootloader
# is built here, from the pinned commit, with the rpi_arm64_defconfig and the
# nrzube fragment.
#
# The cross toolchain and the host tools (dtc, python3) come from the
# Buildroot output directory.
#
# usage: build-uboot.sh <buildroot-output-dir> <out-dir> <src-dir>
# =============================================================================
set -eu

BR_OUT="${1:?usage: build-uboot.sh <buildroot-output-dir> <out-dir> <src-dir>}"
OUT_DIR="${2:?usage: build-uboot.sh <buildroot-output-dir> <out-dir> <src-dir>}"
SRC_DIR="${3:?usage: build-uboot.sh <buildroot-output-dir> <out-dir> <src-dir>}"

REPO_DIR="$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)"
BOARD_DIR="$REPO_DIR/buildroot/board/nrzube"
HOST="$BR_OUT/host"

# shellcheck disable=SC1091
. "$REPO_DIR/scripts/lib-versions.sh"

PATH="$HOST/bin:$PATH"
export PATH

CROSS_COMPILE=""
for candidate in "$HOST"/bin/*-gcc; do
	[ -x "$candidate" ] || continue
	CROSS_COMPILE="$(basename -- "$candidate" | sed 's/-gcc$//')"
	break
done
[ -n "$CROSS_COMPILE" ] || {
	echo "nrzube[build-uboot]: no cross compiler found in $HOST/bin" >&2
	exit 1
}

# --- Fetch the pinned source --------------------------------------------------
# The release tarball is used instead of a git checkout: the SHA-256 pinned in
# versions.lock is then verified directly, and the build does not depend on
# the git transport (some hosts rewrite https://github.com/ to ssh).
TARBALL="$SRC_DIR/../dl/u-boot-$(echo "$UBOOT_TAG" | tr '/' '_').tar.gz"
mkdir -p "$(dirname -- "$TARBALL")"

if [ -f "$TARBALL" ] && echo "$UBOOT_SHA256  $TARBALL" | sha256sum -c --status; then
	echo "nrzube[build-uboot]: using the cached $TARBALL"
else
	echo "nrzube[build-uboot]: downloading $UBOOT_URL"
	curl --fail --location --silent --show-error --retry 3 -o "$TARBALL.part" "$UBOOT_URL"
	echo "$UBOOT_SHA256  $TARBALL.part" | sha256sum -c --status || {
		echo "nrzube[build-uboot]: ERROR: SHA-256 mismatch for $UBOOT_URL" >&2
		rm -f "$TARBALL.part"
		exit 1
	}
	mv "$TARBALL.part" "$TARBALL"
fi

if [ ! -f "$SRC_DIR/Makefile" ]; then
	echo "nrzube[build-uboot]: extracting the source"
	rm -rf "$SRC_DIR"
	mkdir -p "$SRC_DIR"
	tar -xzf "$TARBALL" -C "$SRC_DIR" --strip-components=1
fi

# --- Configure ----------------------------------------------------------------
BUILD_DIR="$SRC_DIR/build"
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR"

make -C "$SRC_DIR" O="$BUILD_DIR" rpi_arm64_defconfig

# Merge the nrzube fragment.
"$SRC_DIR/scripts/kconfig/merge_config.sh" -m -O "$BUILD_DIR" \
	"$BUILD_DIR/.config" "$BOARD_DIR/uboot-nrzube.fragment" >/dev/null
make -C "$SRC_DIR" O="$BUILD_DIR" olddefconfig

# --- Build --------------------------------------------------------------------
echo "nrzube[build-uboot]: building u-boot.bin"
make -C "$SRC_DIR" O="$BUILD_DIR" -j"$(nproc 2>/dev/null || echo 4)" \
	CROSS_COMPILE="$CROSS_COMPILE-" \
	HOSTCFLAGS="-I$HOST/include" HOSTLDFLAGS="-L$HOST/lib"

[ -f "$BUILD_DIR/u-boot.bin" ] || {
	echo "nrzube[build-uboot]: ERROR: u-boot.bin was not produced" >&2
	exit 1
}

install -m 0644 "$BUILD_DIR/u-boot.bin" "$OUT_DIR/u-boot.bin"
echo "nrzube[build-uboot]: $OUT_DIR/u-boot.bin ($(stat -c %s "$OUT_DIR/u-boot.bin") bytes)"
