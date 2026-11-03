#!/bin/sh
# =============================================================================
# nrzube — build the stage 1 (boot gate) initramfs.
#
# Same mechanism as the real initramfs, but the init script only brings the
# console up and drops to a shell.
#
# usage: build-initramfs-stage1.sh <buildroot-output-dir> <out-dir>
# =============================================================================
set -eu

BR_OUT="${1:?usage: build-initramfs-stage1.sh <buildroot-output-dir> <out-dir>}"
OUT_DIR="${2:?usage: build-initramfs-stage1.sh <buildroot-output-dir> <out-dir>}"

BOARD_DIR="$(CDPATH='' cd -- "$(dirname -- "$0")/../buildroot/board/nrzube" && pwd)"
TARGET="$BR_OUT/target"
HOST="$BR_OUT/host"

[ -x "$TARGET/bin/busybox" ] || {
	echo "build-initramfs-stage1: $TARGET/bin/busybox not found" >&2
	exit 1
}

STAGE="$OUT_DIR/initramfs-root"
rm -rf "$STAGE"
mkdir -p "$STAGE/bin" "$STAGE/lib" "$STAGE/proc" "$STAGE/sys" "$STAGE/dev"

cp "$TARGET/bin/busybox" "$STAGE/bin/busybox"
chmod 0755 "$STAGE/bin/busybox"

for applet in sh mount umount mkdir cat tr head mv rm ls grep sed dmesg; do
	ln -sf /bin/busybox "$STAGE/bin/$applet"
done

# --- shared libraries --------------------------------------------------------
READELF=""
for candidate in "$HOST"/bin/*-readelf; do
	[ -x "$candidate" ] && READELF="$candidate" && break
done
[ -n "$READELF" ] || {
	echo "build-initramfs-stage1: no cross readelf found in $HOST/bin" >&2
	exit 1
}

find_lib() {
	for dir in "$TARGET/lib" "$TARGET/usr/lib" "$TARGET/lib64" "$TARGET/usr/lib64"; do
		if [ -e "$dir/$1" ]; then
			printf '%s\n' "$dir/$1"
			return 0
		fi
	done
	return 1
}

copy_lib() {
	lib_path="$1"
	[ -n "$lib_path" ] || return 0
	rel="${lib_path#$TARGET}"
	mkdir -p "$STAGE$(dirname -- "$rel")"
	cp -L "$lib_path" "$STAGE$rel"
	chmod 0755 "$STAGE$rel" 2>/dev/null || true
}

INTERP="$("$READELF" -l "$TARGET/bin/busybox" 2>/dev/null |
	sed -n 's/.*program interpreter: \(.*\)\]/\1/p' | head -n 1)"
if [ -n "$INTERP" ]; then
	# The interpreter path is absolute in the target: look it up by name.
	found="$(find_lib "$(basename -- "$INTERP")" || true)"
	[ -n "$found" ] || {
		echo "build-initramfs-stage1: the dynamic loader ${INTERP#/} is missing" >&2
		exit 1
	}
	copy_lib "$found"
fi

for lib in $("$READELF" -d "$TARGET/bin/busybox" 2>/dev/null |
	sed -n 's/.*Shared library: \[\(.*\)\]/\1/p'); do
	found="$(find_lib "$lib" || true)"
	[ -n "$found" ] || {
		echo "build-initramfs-stage1: the shared library $lib is missing" >&2
		exit 1
	}
	copy_lib "$found"
done

# --- init script -------------------------------------------------------------
cp "$BOARD_DIR/initramfs/stage1-init" "$STAGE/init"
chmod 0755 "$STAGE/init"

mkdir -p "$OUT_DIR"
(cd "$STAGE" && find . -print0 | cpio --null -o -H newc --quiet) |
	gzip -9 > "$OUT_DIR/initrd.img"

echo "build-initramfs-stage1: $OUT_DIR/initrd.img ($(stat -c %s "$OUT_DIR/initrd.img") bytes)"
