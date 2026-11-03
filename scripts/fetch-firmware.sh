#!/bin/sh
# =============================================================================
# nrzube — fetch and verify the Raspberry Pi firmware release.
#
# The firmware provides the GPU firmware (start4.elf / fixup4.dat) and the
# device tree blob that the bootloader hands over to U-Boot through x0.
# Without those files, the EEPROM bootloader cannot start the firmware and
# the board stays dead: they are part of the boot partition.
#
# usage: fetch-firmware.sh <destination-dir>
# =============================================================================
set -eu

DEST_DIR="${1:?usage: fetch-firmware.sh <destination-dir>}"
REPO_DIR="$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)"

# shellcheck disable=SC1091
. "$REPO_DIR/scripts/lib-versions.sh"

TARBALL="$DEST_DIR/raspberrypi-firmware-$FIRMWARE_TAG.tar.gz"
mkdir -p "$DEST_DIR"

if [ -f "$TARBALL" ] && echo "$FIRMWARE_SHA256  $TARBALL" | sha256sum -c --status; then
	echo "fetch-firmware: using the cached $TARBALL"
else
	echo "fetch-firmware: downloading $FIRMWARE_URL"
	curl --fail --location --silent --show-error --retry 3 -o "$TARBALL.part" "$FIRMWARE_URL"
	echo "$FIRMWARE_SHA256  $TARBALL.part" | sha256sum -c --status || {
		echo "fetch-firmware: SHA-256 mismatch for $FIRMWARE_URL" >&2
		rm -f "$TARBALL.part"
		exit 1
	}
	mv "$TARBALL.part" "$TARBALL"
fi

BOOT_DIR="$DEST_DIR/boot"
if [ ! -f "$BOOT_DIR/start4.elf" ]; then
	echo "fetch-firmware: extracting the firmware"
	rm -rf "$BOOT_DIR"
	mkdir -p "$BOOT_DIR"
	tar -xzf "$TARBALL" -C "$BOOT_DIR" --strip-components=2 "firmware-$FIRMWARE_TAG/boot" 2>/dev/null ||
		tar -xzf "$TARBALL" -C "$BOOT_DIR" --strip-components=2
fi

for file in start4.elf fixup4.dat bcm2712-rpi-5-b.dtb; do
	[ -f "$BOOT_DIR/$file" ] || {
		echo "fetch-firmware: ERROR: $file is missing from the firmware release" >&2
		exit 1
	}
done

echo "fetch-firmware: ready in $BOOT_DIR"
