#!/bin/sh
# =============================================================================
# nrzube — assemble the disk image.
#
# GPT layout:
#   p1  FAT32   boot partition (config.txt, u-boot.bin, Image, initrd.img,
#                boot.scr, fallback DTB)
#   p2  squashfs rootfs slot A
#   p3  squashfs rootfs slot B
#   p4  ext4     state partition (persistent /var: slot selector, node config,
#                containerd/kubelet state, /etc overlay upper)
#
# usage: build-image.sh <buildroot-output-dir> <out-dir> <dist-dir>
# =============================================================================
set -eu

BR_OUT="${1:?usage: build-image.sh <buildroot-output-dir> <out-dir> <dist-dir>}"
OUT_DIR="${2:?usage: build-image.sh <buildroot-output-dir> <out-dir> <dist-dir>}"
DIST_DIR="${3:?usage: build-image.sh <buildroot-output-dir> <out-dir> <dist-dir>}"

REPO_DIR="$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)"
BOARD_DIR="$REPO_DIR/buildroot/board/nrzube"
HOST="$BR_OUT/host"
TARGET="$BR_OUT/target"

# Partition sizes (override through the environment).
BOOT_SIZE="${NRZUBE_BOOT_SIZE:-512M}"
STATE_SIZE="${NRZUBE_STATE_SIZE:-4G}"
SLOT_SIZE="${NRZUBE_SLOT_SIZE:-256M}"

NRZUBE_VERSION="${NRZUBE_VERSION:-$(date -u +%Y.%m.%d)}"
NRZUBE_CHANNEL="${NRZUBE_CHANNEL:-stable}"

# The host tools are the ones Buildroot built.
PATH="$HOST/bin:$PATH"
export PATH

for tool in genimage mksquashfs mkfs.vfat mcopy mkfs.ext4; do
	command -v "$tool" >/dev/null 2>&1 || {
		echo "build-image: host tool '$tool' not found in $HOST/bin" >&2
		exit 1
	}
done

# --- 1. Check the artifacts ---------------------------------------------------
for artifact in Image initrd.img u-boot.bin boot.scr; do
	[ -f "$OUT_DIR/$artifact" ] || {
		echo "build-image: missing artifact $OUT_DIR/$artifact" >&2
		exit 1
	}
done
[ -f "$BR_OUT/images/bcm2712-rpi-5-b.dtb" ] || {
	echo "build-image: missing DTB $BR_OUT/images/bcm2712-rpi-5-b.dtb" >&2
	exit 1
}
cp "$BR_OUT/images/bcm2712-rpi-5-b.dtb" "$OUT_DIR/bcm2712-rpi-5-b.dtb"

# --- 2. Slot trees ------------------------------------------------------------
# Each slot carries its own identifier (/nrzube-slot-id at the root of the
# rootfs): the initramfs identifies the slots by *content*, not by partition
# order.
# genimage works in its own root directory: the output image must not be
# inside it (genimage copies the root directory to build the filesystems).
IMAGE_ROOT="$OUT_DIR/image-root"
rm -rf "$IMAGE_ROOT"
mkdir -p "$IMAGE_ROOT"

SLOT_ROOT="$OUT_DIR/slot-root"
rm -rf "$SLOT_ROOT"
mkdir -p "$SLOT_ROOT"
cp -a "$TARGET/." "$SLOT_ROOT/"

build_slot() {
	slot="$1"
	output="$2"
	printf '%s\n' "$slot" > "$SLOT_ROOT/nrzube-slot-id"
	rm -f "$output"
	# Same options as Buildroot (zstd), without the append behaviour.
	mksquashfs "$SLOT_ROOT" "$output" \
		-comp zstd -noappend -no-progress -all-root \
		-Xcompression-level 19 >/dev/null
	# The slot partition has a FIXED size so that it does not have to be
	# resized when a new rootfs is installed. The trailing zeros are ignored
	# by the squashfs superblock.
	truncate -s "$SLOT_SIZE" "$output"
	echo "build-image: slot $slot -> $output ($(stat -c %s "$output") bytes)"
}

build_slot A "$IMAGE_ROOT/slot-a.squashfs"
build_slot B "$IMAGE_ROOT/slot-b.squashfs"

# --- 3. State partition -------------------------------------------------------
# The root of p4 IS the /var of the running system.
STATE_ROOT="$IMAGE_ROOT/state-root"
rm -rf "$STATE_ROOT"
mkdir -p "$STATE_ROOT/lib/nrzube" "$STATE_ROOT/lib/kubelet" \
	"$STATE_ROOT/lib/containerd" "$STATE_ROOT/log"

: > "$STATE_ROOT/nrzube-state"
printf 'A\n' > "$STATE_ROOT/lib/nrzube/active-slot"
rm -f "$STATE_ROOT/lib/nrzube/pending-slot" \
	"$STATE_ROOT/lib/nrzube/boot-ok" \
	"$STATE_ROOT/lib/nrzube/pending-boot-count" \
	"$STATE_ROOT/lib/nrzube/boot-fail-count"
printf '0\n' > "$STATE_ROOT/lib/nrzube/pending-boot-count"
printf '0\n' > "$STATE_ROOT/lib/nrzube/boot-fail-count"

# --- 4. Boot partition --------------------------------------------------------
# The boot files are copied at the root of the image directory: genimage
# resolves them relatively to its --inputpath.
cp "$BOARD_DIR/config.txt" "$IMAGE_ROOT/config.txt"
cp "$OUT_DIR/u-boot.bin" "$IMAGE_ROOT/u-boot.bin"
cp "$OUT_DIR/Image" "$IMAGE_ROOT/Image"
cp "$OUT_DIR/initrd.img" "$IMAGE_ROOT/initrd.img"
cp "$OUT_DIR/boot.scr" "$IMAGE_ROOT/boot.scr"

# GPU firmware and device tree blob of the Raspberry Pi release: the EEPROM
# bootloader loads the firmware from the boot partition, and the firmware
# hands its DTB over to U-Boot through x0.
cp "$FIRMWARE_BOOT_DIR/start4.elf" "$IMAGE_ROOT/start4.elf"
cp "$FIRMWARE_BOOT_DIR/fixup4.dat" "$IMAGE_ROOT/fixup4.dat"
cp "$FIRMWARE_BOOT_DIR/bcm2712-rpi-5-b.dtb" "$IMAGE_ROOT/bcm2712-rpi-5-b.dtb"
# The DTB compiled from the kernel tree is kept as a fallback: U-Boot only
# uses it when the firmware provided no DTB.
cp "$OUT_DIR/bcm2712-rpi-5-b.dtb" "$IMAGE_ROOT/bcm2712-rpi-5-b-kernel.dtb"

# --- 5. genimage --------------------------------------------------------------
GENIMAGE_CFG="$OUT_DIR/genimage.cfg"
cat > "$GENIMAGE_CFG" <<EOF
# The boot partition is filled with the files below: genimage copies them
# with mcopy.
image config.txt {
	file {
		name = "config.txt"
	}
}

image u-boot.bin {
	file {
		name = "u-boot.bin"
	}
}

image kernel-image {
	file {
		name = "Image"
	}
}

image initrd-image {
	file {
		name = "initrd.img"
	}
}

image boot-script {
	file {
		name = "boot.scr"
	}
}

image dtb-image {
	file {
		name = "bcm2712-rpi-5-b.dtb"
	}
}

image dtb-kernel-image {
	file {
		name = "bcm2712-rpi-5-b-kernel.dtb"
	}
}

image start4-elf {
	file {
		name = "start4.elf"
	}
}

image fixup4-dat {
	file {
		name = "fixup4.dat"
	}
}

image boot.vfat {
	vfat {
		label = "NRZUBE-BOOT"
		file "config.txt" {
			image = "config.txt"
		}
		file "u-boot.bin" {
			image = "u-boot.bin"
		}
		file "Image" {
			image = "kernel-image"
		}
		file "initrd.img" {
			image = "initrd-image"
		}
		file "boot.scr" {
			image = "boot-script"
		}
		file "bcm2712-rpi-5-b.dtb" {
			image = "dtb-image"
		}
		file "bcm2712-rpi-5-b-kernel.dtb" {
			image = "dtb-kernel-image"
		}
		file "start4.elf" {
			image = "start4-elf"
		}
		file "fixup4.dat" {
			image = "fixup4-dat"
		}
	}
	size = $BOOT_SIZE
}

image slot-a.squashfs {
	file {
		name = "slot-a.squashfs"
	}
}

image slot-b.squashfs {
	file {
		name = "slot-b.squashfs"
	}
}

image state.ext4 {
	ext4 {
		sourcedir = "state-root"
		label = "nrzube-state"
	}
	size = $STATE_SIZE
}

image nrzube.img {
	hdimage {
		partition-table-type = "gpt"
	}

	partition boot {
		partition-type-uuid = "U"
		image = "boot.vfat"
	}

	partition slot-a {
		partition-type-uuid = "L"
		image = "slot-a.squashfs"
	}

	partition slot-b {
		partition-type-uuid = "L"
		image = "slot-b.squashfs"
	}

	partition state {
		partition-type-uuid = "L"
		image = "state.ext4"
	}
}
EOF

# The slot images are already built: genimage only has to embed them.
# genimage runs under fakeroot so that the ownership of the files inside the
# filesystems is root:root.
RUNNER=""
if command -v fakeroot >/dev/null 2>&1; then
	RUNNER="fakeroot"
fi

# genimage creates its temporary directory in the current directory: it must
# not be inside the root directory it copies.
(cd "$OUT_DIR" && $RUNNER genimage --rootpath "$IMAGE_ROOT" --inputpath "$IMAGE_ROOT" \
	--config "$GENIMAGE_CFG" "nrzube.img")

# --- 6. Digests and release manifest -----------------------------------------
mkdir -p "$DIST_DIR"

sha256_of() {
	sha256sum "$1" | cut -d' ' -f1
}

ROOTFS_SHA="$(sha256_of "$IMAGE_ROOT/slot-a.squashfs")"
KERNEL_SHA="$(sha256_of "$OUT_DIR/Image")"
INITRD_SHA="$(sha256_of "$OUT_DIR/initrd.img")"
DTB_SHA="$(sha256_of "$OUT_DIR/bcm2712-rpi-5-b.dtb")"
IMAGE_SHA="$(sha256_of "$OUT_DIR/images/nrzube.img")"

ROOTFS_SIZE="$(stat -c %s "$IMAGE_ROOT/slot-a.squashfs")"
KERNEL_SIZE="$(stat -c %s "$OUT_DIR/Image")"
INITRD_SIZE="$(stat -c %s "$OUT_DIR/initrd.img")"
DTB_SIZE="$(stat -c %s "$OUT_DIR/bcm2712-rpi-5-b.dtb")"
IMAGE_SIZE="$(stat -c %s "$OUT_DIR/nrzube.img")"

cat > "$DIST_DIR/release.json" <<EOF
{
  "schemaVersion": 1,
  "version": "$NRZUBE_VERSION",
  "channel": "$NRZUBE_CHANNEL",
  "components": [
    { "name": "rootfs", "mediaType": "application/vnd.nrzube.rootfs.v1",
      "sha256": "$ROOTFS_SHA", "size": $ROOTFS_SIZE, "ociDigest": "sha256:$ROOTFS_SHA" },
    { "name": "kernel", "mediaType": "application/vnd.nrzube.kernel.v1",
      "sha256": "$KERNEL_SHA", "size": $KERNEL_SIZE, "ociDigest": "sha256:$KERNEL_SHA" },
    { "name": "initrd", "mediaType": "application/vnd.nrzube.initrd.v1",
      "sha256": "$INITRD_SHA", "size": $INITRD_SIZE, "ociDigest": "sha256:$INITRD_SHA" },
    { "name": "dtb", "mediaType": "application/vnd.nrzube.dtb.v1",
      "sha256": "$DTB_SHA", "size": $DTB_SIZE, "ociDigest": "sha256:$DTB_SHA" }
  ]
}
EOF

{
	echo "$IMAGE_SHA  nrzube.img"
	echo "$ROOTFS_SHA  rootfs.squashfs"
	echo "$KERNEL_SHA  Image"
	echo "$INITRD_SHA  initrd.img"
	echo "$DTB_SHA  bcm2712-rpi-5-b.dtb"
} > "$DIST_DIR/SHA256SUMS"

cp "$OUT_DIR/images/nrzube.img" "$DIST_DIR/nrzube.img"

echo "build-image: image ready: $DIST_DIR/nrzube.img ($IMAGE_SIZE bytes)"
echo "build-image: sha256 $IMAGE_SHA"
