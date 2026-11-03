#!/bin/sh
# =============================================================================
# nrzube — assemble the stage 1 (boot gate) image.
#
# A single FAT partition holding the bootloader, the kernel, the initramfs and
# the DTB. It is enough to validate the boot chain on the hardware:
# firmware -> U-Boot -> kernel -> initramfs -> shell.
#
# usage: build-boot-gate-image.sh <buildroot-output-dir> <out-dir> <dist-dir>
# =============================================================================
set -eu

BR_OUT="${1:?usage: build-boot-gate-image.sh <buildroot-output-dir> <out-dir> <dist-dir>}"
OUT_DIR="${2:?usage: build-boot-gate-image.sh <buildroot-output-dir> <out-dir> <dist-dir>}"
DIST_DIR="${3:?usage: build-boot-gate-image.sh <buildroot-output-dir> <out-dir> <dist-dir>}"

REPO_DIR="$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)"
BOARD_DIR="$REPO_DIR/buildroot/board/nrzube"
HOST="$BR_OUT/host"

PATH="$HOST/bin:$PATH"
export PATH

for tool in genimage mkfs.vfat mcopy; do
	command -v "$tool" >/dev/null 2>&1 || {
		echo "build-boot-gate-image: host tool '$tool' not found in $HOST/bin" >&2
		exit 1
	}
done

for artifact in Image initrd.img u-boot.bin boot.scr; do
	[ -f "$OUT_DIR/$artifact" ] || {
		echo "build-boot-gate-image: missing artifact $OUT_DIR/$artifact" >&2
		exit 1
	}
done
[ -f "$BR_OUT/images/bcm2712-rpi-5-b.dtb" ] || {
	echo "build-boot-gate-image: missing DTB" >&2
	exit 1
}
cp "$BR_OUT/images/bcm2712-rpi-5-b.dtb" "$OUT_DIR/bcm2712-rpi-5-b.dtb"

# genimage works in its own root directory: the output image must not be
# inside it (genimage copies the root directory to build the filesystems).
IMAGE_ROOT="$OUT_DIR/image-root"
rm -rf "$IMAGE_ROOT"
mkdir -p "$IMAGE_ROOT"

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

GENIMAGE_CFG="$OUT_DIR/genimage-boot-gate.cfg"
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
	size = 512M
}

image nrzube-boot-gate.img {
	hdimage {
		partition-table-type = "gpt"
	}

	partition boot {
		partition-type-uuid = "U"
		image = "boot.vfat"
	}
}
EOF

RUNNER=""
if command -v fakeroot >/dev/null 2>&1; then
	RUNNER="fakeroot"
fi

# genimage creates its temporary directory in the current directory: it must
# not be inside the root directory it copies.
(cd "$OUT_DIR" && $RUNNER genimage --rootpath "$IMAGE_ROOT" --inputpath "$IMAGE_ROOT" \
	--config "$GENIMAGE_CFG" "nrzube-boot-gate.img")

mkdir -p "$DIST_DIR"
cp "$OUT_DIR/images/nrzube-boot-gate.img" "$DIST_DIR/nrzube-boot-gate.img"
(cd "$DIST_DIR" && sha256sum nrzube-boot-gate.img > nrzube-boot-gate.img.sha256)

echo "build-boot-gate-image: $DIST_DIR/nrzube-boot-gate.img"
sha256sum "$DIST_DIR/nrzube-boot-gate.img"
