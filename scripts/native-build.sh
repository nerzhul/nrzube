#!/bin/sh
# =============================================================================
# nrzube — native build (no container).
#
# The host needs the Buildroot build dependencies:
#   Debian/Ubuntu: build-essential bc bison cpio curl file flex git libssl-dev
#                   python3 rsync swig unzip u-boot-tools dosfstools mtools
#                   squashfs-tools e2fsprogs parted gdisk device-tree-compiler
#   Arch:          base-devel bc bison cpio curl file flex git openssl python
#                   rsync swig unzip u-boot-tools dosfstools mtools
#                   squashfs-tools e2fsprogs parted gdisk dtc
#
# usage: native-build.sh [boot-gate|full]
# =============================================================================
set -eu

REPO_DIR="$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)"
TARGET_KIND="${1:-full}"

BR2_DIR="$REPO_DIR/build/buildroot"
OUT_DIR="$REPO_DIR/build/out"
DIST_DIR="$REPO_DIR/dist"
SRC_DIR="$REPO_DIR/build/src"

mkdir -p "$BR2_DIR" "$OUT_DIR" "$DIST_DIR" "$SRC_DIR"

exec "$REPO_DIR/scripts/build-all.sh" \
	"$REPO_DIR" "$BR2_DIR" "$OUT_DIR" "$DIST_DIR" "$SRC_DIR" "$TARGET_KIND"
