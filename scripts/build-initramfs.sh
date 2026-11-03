#!/bin/sh
# =============================================================================
# nrzube — build the initramfs (cpio.gz).
#
# The initramfs is deliberately minimal: busybox, the glibc runtime it needs
# and the nrzube /init script. It must be able to mount the state partition,
# the slot squashfs and the /etc overlay without loading a single module
# (everything required is built into the kernel).
#
# usage: build-initramfs.sh <buildroot-output-dir> <out-dir>
# =============================================================================
set -eu

BR_OUT="${1:?usage: build-initramfs.sh <buildroot-output-dir> <out-dir>}"
OUT_DIR="${2:?usage: build-initramfs.sh <buildroot-output-dir> <out-dir>}"

BOARD_DIR="$(CDPATH='' cd -- "$(dirname -- "$0")/../buildroot/board/nrzube" && pwd)"
TARGET="$BR_OUT/target"
HOST="$BR_OUT/host"

[ -x "$TARGET/bin/busybox" ] || {
	echo "build-initramfs: $TARGET/bin/busybox not found (build the rootfs first)" >&2
	exit 1
}

STAGE="$OUT_DIR/initramfs-root"
rm -rf "$STAGE"
mkdir -p "$STAGE/bin" "$STAGE/lib" "$STAGE/usr/bin" "$STAGE/usr/sbin" \
	"$STAGE/proc" "$STAGE/sys" "$STAGE/dev" "$STAGE/run" "$STAGE/tmp" \
	"$STAGE/newroot" "$STAGE/mnt" "$STAGE/state"

# --- busybox and its applet symlinks -----------------------------------------
cp "$TARGET/bin/busybox" "$STAGE/bin/busybox"
chmod 0755 "$STAGE/bin/busybox"

for applet in sh mount umount mkdir rmdir cat tr head mv rm ls grep sed \
	sync sleep tail dmesg reboot poweroff halt printf echo test true false; do
	ln -sf /bin/busybox "$STAGE/bin/$applet"
done

# --- shared libraries --------------------------------------------------------
# The cross binutils shipped by Buildroot are used to resolve the DT_NEEDED
# entries of busybox: the target rootfs is the only source of truth.
READELF=""
for candidate in "$HOST"/bin/*-readelf; do
	[ -x "$candidate" ] && READELF="$candidate" && break
done
[ -n "$READELF" ] || {
	echo "build-initramfs: no cross readelf found in $HOST/bin" >&2
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

# The dynamic loader.
INTERP="$("$READELF" -l "$TARGET/bin/busybox" 2>/dev/null |
	sed -n 's/.*program interpreter: \(.*\)\]/\1/p' | head -n 1)"
if [ -n "$INTERP" ]; then
	loader="$(basename -- "$INTERP")"
	# The interpreter path is absolute in the target: look it up by name.
	found="$(find_lib "$loader" || true)"
	if [ -z "$found" ]; then
		echo "build-initramfs: dynamic loader $loader not found in the target" >&2
		exit 1
	fi
	copy_lib "$found"
fi

# The libraries themselves.
for lib in $("$READELF" -d "$TARGET/bin/busybox" 2>/dev/null |
	sed -n 's/.*Shared library: \[\(.*\)\]/\1/p'); do
	found="$(find_lib "$lib" || true)"
	if [ -z "$found" ]; then
		echo "build-initramfs: shared library $lib not found in the target" >&2
		exit 1
	fi
	copy_lib "$found"
done

# --- init script --------------------------------------------------------------
cp "$BOARD_DIR/initramfs/init" "$STAGE/init"
chmod 0755 "$STAGE/init"

# --- cpio ---------------------------------------------------------------------
mkdir -p "$OUT_DIR"
(cd "$STAGE" && find . -print0 | cpio --null -o -H newc --quiet) |
	gzip -9 > "$OUT_DIR/initrd.img"

echo "build-initramfs: $(ls -l "$OUT_DIR/initrd.img" | awk '{print $5}') bytes -> $OUT_DIR/initrd.img"
