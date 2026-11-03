#!/bin/sh
# =============================================================================
# nrzube — Buildroot build driver.
#
# Runs the Buildroot build with the reference defconfig, then checks that the
# kernel checkout is the pinned commit (the tag is stable, the branch is not).
#
# usage: build-rootfs.sh <buildroot-dir> <br2-external-dir>
# =============================================================================
set -eu

BR2_DIR="${1:?usage: build-rootfs.sh <buildroot-dir> <br2-external-dir>}"
BR2_EXTERNAL="${2:?usage: build-rootfs.sh <buildroot-dir> <br2-external-dir>}"

REPO_DIR="$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)"

# The configuration to build (boot-gate by default).
DEFCONFIG="${NRZUBE_DEFCONFIG:-nrzube_pi5_defconfig}"

# The versions must be exported: the defconfig refers to $(KERNEL_TAG) and
# friends, which are expanded when Buildroot uses them.
# shellcheck disable=SC1091
. "$REPO_DIR/scripts/lib-versions.sh"
RUNC_MIN_VERSION="${RUNC_MIN_VERSION:-1.5.2}"
export RUNC_MIN_VERSION

echo "nrzube[build-rootfs]: building $DEFCONFIG in $BR2_DIR"
make -C "$BR2_DIR" BR2_EXTERNAL="$BR2_EXTERNAL" "$DEFCONFIG"
make -C "$BR2_DIR" BR2_EXTERNAL="$BR2_EXTERNAL"

# --- Verify the kernel commit -------------------------------------------------
LINUX_GIT=""
for candidate in "$BR2_DIR"/dl/linux/git "$BR2_DIR"/output/build/linux-*/.git; do
	[ -d "$candidate" ] && LINUX_GIT="$candidate" && break
done

if [ -z "$LINUX_GIT" ]; then
	echo "nrzube[build-rootfs]: WARNING: no kernel git cache found, skipping the commit check" >&2
else
	COMMIT="$(git -C "$LINUX_GIT" rev-parse HEAD)"
	if [ "$COMMIT" != "${KERNEL_COMMIT:-}" ]; then
		echo "nrzube[build-rootfs]: ERROR: kernel commit mismatch" >&2
		echo "  expected: ${KERNEL_COMMIT:-<unset>}" >&2
		echo "  actual:   $COMMIT" >&2
		echo "  the kernel tag must be immutable: update versions.lock" >&2
		exit 1
	fi
	echo "nrzube[build-rootfs]: kernel commit verified ($COMMIT)"
fi

echo "nrzube[build-rootfs]: done"
