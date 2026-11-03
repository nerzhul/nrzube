#!/bin/sh
# =============================================================================
# nrzube — container build entry point.
#
# Runs inside the build container; the real work is in build-all.sh.
# =============================================================================
set -eu

REPO_DIR="${NRZUBE_REPO_DIR:-/src}"
BR2_DIR="${NRZUBE_BR_DIR:-/build/buildroot}"
OUT_DIR="${NRZUBE_OUT_DIR:-/build/out}"
DIST_DIR="${NRZUBE_DIST_DIR:-/build/dist}"
SRC_DIR="${NRZUBE_SRC_DIR:-/build/src}"
TARGET_KIND="${NRZUBE_TARGET_KIND:-full}"

exec "$REPO_DIR/scripts/build-all.sh" \
	"$REPO_DIR" "$BR2_DIR" "$OUT_DIR" "$DIST_DIR" "$SRC_DIR" "$TARGET_KIND"
