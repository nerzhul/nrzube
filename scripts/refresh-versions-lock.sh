#!/bin/sh
# =============================================================================
# nrzube — refresh versions.lock.
#
# Re-resolves the pinned COMMITs from the pinned TAGs (GitHub API) and
# recomputes the SHA-256 of the release artifacts.
#
# usage: refresh-versions-lock.sh [--sha256]
#           --sha256  also re-download the artifacts to refresh their digests
# =============================================================================
set -eu

REPO_DIR="$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)"
LOCK="$REPO_DIR/versions.lock"

WITH_SHA256=0
[ "${1:-}" = "--sha256" ] && WITH_SHA256=1

log() {
	echo "versions-lock: $*"
}

die() {
	echo "versions-lock: ERROR: $*" >&2
	exit 1
}

require_cmd() {
	command -v "$1" >/dev/null 2>&1 || die "$1 is required"
}

require_cmd curl
require_cmd jq
require_cmd git

# --- Read the current lock ----------------------------------------------------
get() {
	sed -n "s/^$1=//p" "$LOCK" | head -n 1
}

set_value() {
	# set_value <key> <value>
	key="$1"
	value="$2"
	if grep -q "^$key=" "$LOCK"; then
		sed -i "s|^$key=.*|$key=$value|" "$LOCK"
	else
		echo "$key=$value" >> "$LOCK"
	fi
	log "$key = $value"
}

resolve_commit() {
	# resolve_commit <owner/repo> <tag> <annotated?>
	repo="$1"
	tag="$2"
	type="$(curl -fsSL "https://api.github.com/repos/$repo/git/ref/tags/$tag" |
		jq -r '.object.type')"
	case "$type" in
	commit)
		curl -fsSL "https://api.github.com/repos/$repo/git/ref/tags/$tag" |
			jq -r '.object.sha'
		;;
	tag)
		curl -fsSL "https://api.github.com/repos/$repo/git/tags/$tag" |
			jq -r '.object.sha'
		;;
	*)
		die "cannot resolve $repo@$tag (type=$type)"
		;;
	esac
}

# --- Git sources ---------------------------------------------------------------
log "resolving the git commits from the tags"

BUILDROOT_COMMIT="$(resolve_commit buildroot/buildroot "$(get BUILDROOT_VERSION)")"
set_value BUILDROOT_COMMIT "$BUILDROOT_COMMIT"

KERNEL_COMMIT="$(resolve_commit "$(get KERNEL_REPO)" "$(get KERNEL_TAG)")"
set_value KERNEL_COMMIT "$KERNEL_COMMIT"

FIRMWARE_COMMIT="$(resolve_commit "$(get FIRMWARE_REPO)" "$(get FIRMWARE_TAG)")"
set_value FIRMWARE_COMMIT "$FIRMWARE_COMMIT"

EEPROM_COMMIT="$(resolve_commit "$(get EEPROM_REPO)" "$(get EEPROM_TAG)")"
set_value EEPROM_COMMIT "$EEPROM_COMMIT"

UBOOT_COMMIT="$(resolve_commit u-boot/u-boot "$(get UBOOT_TAG)")"
set_value UBOOT_COMMIT "$UBOOT_COMMIT"

# --- Release artifacts ---------------------------------------------------------
if [ "$WITH_SHA256" -eq 1 ]; then
	require_cmd sha256sum
	TMP="$(mktemp -d)"
	trap 'rm -rf "$TMP"' EXIT

	refresh_sha256() {
		# refresh_sha256 <key> <url>
		key="$1"
		url="$2"
		log "downloading $(basename -- "$url")"
		curl -fsSL --retry 3 -o "$TMP/artifact" "$url"
		set_value "$key" "$(sha256sum "$TMP/artifact" | cut -d' ' -f1)"
	}

	refresh_sha256 BUILDROOT_SHA256 \
		"https://buildroot.org/downloads/buildroot-$(get BUILDROOT_VERSION).tar.gz"
	refresh_sha256 UBOOT_SHA256 \
		"https://github.com/u-boot/u-boot/archive/refs/tags/$(get UBOOT_TAG).tar.gz"
	refresh_sha256 CONTAINERD_SHA256 \
		"https://github.com/containerd/containerd/releases/download/v$(get CONTAINERD_VERSION)/containerd-static-$(get CONTAINERD_VERSION)-linux-arm64.tar.gz"
	refresh_sha256 RUNC_SHA256 \
		"https://github.com/opencontainers/runc/releases/download/v$(get RUNC_VERSION)/runc.arm64"
	refresh_sha256 KUBELET_SHA256 \
		"https://dl.k8s.io/release/v$(get KUBERNETES_VERSION)/bin/linux/arm64/kubelet"
	refresh_sha256 KUBECTL_SHA256 \
		"https://dl.k8s.io/release/v$(get KUBERNETES_VERSION)/bin/linux/arm64/kubectl"
	refresh_sha256 CNI_SHA256 \
		"https://github.com/containernetworking/plugins/releases/download/v$(get CNI_VERSION)/cni-plugins-linux-arm64-v$(get CNI_VERSION).tgz"
	refresh_sha256 COSIGN_SHA256 \
		"https://github.com/sigstore/cosign/releases/download/v$(get COSIGN_VERSION)/cosign-linux-arm64"
	refresh_sha256 DHCPCD_SHA256 \
		"https://github.com/NetworkConfiguration/dhcpcd/releases/download/v$(get DHCPCD_VERSION)/dhcpcd-$(get DHCPCD_VERSION).tar.xz"
	refresh_sha256 CHRONY_SHA256 \
		"https://chrony-project.org/releases/chrony-$(get CHRONY_VERSION).tar.gz"
else
	log "skipping the SHA-256 refresh (use --sha256 to force it)"
fi

log "done"
