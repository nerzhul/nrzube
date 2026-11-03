#!/bin/sh
# =============================================================================
# nrzube — local build (containerised).
#
# Builds the Buildroot tree and the image inside a container: the host does
# not need any cross toolchain, and no emulation is required (Buildroot
# cross-compiles natively).
#
# usage: local-build.sh [boot-gate|full]
#   boot-gate  kernel + U-Boot + stage 1 initramfs (Phase 1 validation)
#   full       the complete image (slots, state, rootfs)
# =============================================================================
set -eu

REPO_DIR="$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)"
TARGET_KIND="${1:-full}"

ENGINE="${CONTAINER_ENGINE:-}"
if [ -z "$ENGINE" ]; then
	if command -v podman >/dev/null 2>&1; then
		ENGINE=podman
	elif command -v docker >/dev/null 2>&1; then
		ENGINE=docker
	else
		echo "local-build: neither podman nor docker was found" >&2
		exit 1
	fi
fi

# The versions are exported: the defconfig refers to $(KERNEL_TAG) and
# friends, which are expanded when Buildroot uses them.
# shellcheck disable=SC1091
. "$REPO_DIR/scripts/lib-versions.sh"

BUILDER_IMAGE="${BUILDER_IMAGE:-nrzube-builder:local}"

echo "local-build: engine=$ENGINE image=$BUILDER_IMAGE target=$TARGET_KIND"

# The builder image is rebuilt when its definition changes.
$ENGINE build -t "$BUILDER_IMAGE" \
	--build-arg "RUST_VERSION=$RUST_VERSION" \
	-f "$REPO_DIR/build/Dockerfile" "$REPO_DIR"

mkdir -p "$REPO_DIR/build" "$REPO_DIR/dist"

$ENGINE run --rm \
	-v "$REPO_DIR:/src" \
	-v "$REPO_DIR/build:/build" \
	-e "NRZUBE_REPO_DIR=/src" \
	-e "NRZUBE_BR_DIR=/build/buildroot" \
	-e "BR2_EXTERNAL=/src/buildroot" \
	-e "NRZUBE_OUT_DIR=/build/out" \
	-e "NRZUBE_DIST_DIR=/build/dist" \
	-e "NRZUBE_SRC_DIR=/build/src" \
	-e "NRZUBE_TARGET_KIND=$TARGET_KIND" \
	-e "COSIGN_PUBLIC_KEY_PATH=/src/keys/cosign.pub" \
	"$BUILDER_IMAGE" \
	/bin/sh -c "/src/scripts/build-container.sh $TARGET_KIND"

echo "local-build: done"
ls -l "$REPO_DIR/dist"
