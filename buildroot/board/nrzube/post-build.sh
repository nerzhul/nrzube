#!/bin/sh
# =============================================================================
# nrzube — Buildroot post-build script.
#
# Installs the components that are NOT Buildroot packages:
#   - containerd (static arm64 tarball) + its shim
#   - runc (static arm64 binary) — MANDATORY >= 1.5.2 (kernel >= 6.17)
#   - kubelet and kubectl (upstream static/dynamic binaries)
#   - the CNI plugin binaries in /opt/cni/bin (the CNI configuration itself is
#     written by the cluster)
#   - the cosign public key (PEM) used to verify the OTA manifests
#
# Every download is verified against the SHA-256 pinned in versions.lock.
# =============================================================================
set -eu

TARGET_DIR="${TARGET_DIR:?TARGET_DIR is not set}"
DL_DIR="${BR2_DL_DIR:-${BR2_DIR:-$PWD}/dl}"
BOARD_DIR="${NRZUBE_BOARD_DIR:-$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)/buildroot/board/nrzube}"

mkdir -p "$DL_DIR"

log() {
	echo "nrzube[post-build]: $*"
}

die() {
	echo "nrzube[post-build]: ERROR: $*" >&2
	exit 1
}

# --- Guard rail: runc >= 1.5.2 -------------------------------------------------
# The kernel 6.18 is >= 6.17: runc < 1.5.2 corrupts memory when it configures
# the cgroup v2 device rules (see the plan, §3.3).
version_ge() {
	# version_ge <a> <b> -> true when a >= b
	[ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n 1)" = "$2" ]
}

[ -n "${RUNC_VERSION:-}" ] || die "RUNC_VERSION is not set (versions.lock not loaded)"
if ! version_ge "$RUNC_VERSION" "${RUNC_MIN_VERSION:-1.5.2}"; then
	die "runc $RUNC_VERSION is too old: >= ${RUNC_MIN_VERSION:-1.5.2} is mandatory (kernel >= 6.17)"
fi

fetch() {
	# fetch <url> <sha256> <destination>
	url="$1"
	sha="$2"
	dst="$3"

	if [ -f "$dst" ] && echo "$sha  $dst" | sha256sum -c --status; then
		log "cached: $(basename -- "$dst")"
		return 0
	fi

	log "downloading $(basename -- "$dst")"
	curl --fail --location --silent --show-error --retry 3 -o "$dst.part" "$url" ||
		die "download failed: $url"
	echo "$sha  $dst.part" | sha256sum -c --status ||
		die "SHA-256 mismatch for $url"
	mv "$dst.part" "$dst"
}

# --- containerd ----------------------------------------------------------------
CONTAINERD_TARBALL="$DL_DIR/containerd-static-$CONTAINERD_VERSION-linux-arm64.tar.gz"
fetch "$CONTAINERD_URL" "$CONTAINERD_SHA256" "$CONTAINERD_TARBALL"

CONTAINERD_TMP="$DL_DIR/containerd.$$"
rm -rf "$CONTAINERD_TMP"
mkdir -p "$CONTAINERD_TMP"
tar -xzf "$CONTAINERD_TARBALL" -C "$CONTAINERD_TMP"

mkdir -p "$TARGET_DIR/usr/bin"
for binary in containerd containerd-shim-runc-v2 ctr containerd-stress; do
	if [ -f "$CONTAINERD_TMP/$binary" ]; then
		install -m 0755 "$CONTAINERD_TMP/$binary" "$TARGET_DIR/usr/bin/$binary"
	fi
done
rm -rf "$CONTAINERD_TMP"
log "containerd $CONTAINERD_VERSION installed"

# --- runc ----------------------------------------------------------------------
RUNC_BINARY="$DL_DIR/runc.arm64"
fetch "$RUNC_URL" "$RUNC_SHA256" "$RUNC_BINARY"
install -m 0755 "$RUNC_BINARY" "$TARGET_DIR/usr/bin/runc"
log "runc $RUNC_VERSION installed"

# --- Kubernetes ----------------------------------------------------------------
KUBELET_BINARY="$DL_DIR/kubelet"
fetch "$KUBELET_URL" "$KUBELET_SHA256" "$KUBELET_BINARY"
install -m 0755 "$KUBELET_BINARY" "$TARGET_DIR/usr/bin/kubelet"

KUBECTL_BINARY="$DL_DIR/kubectl"
fetch "$KUBECTL_URL" "$KUBECTL_SHA256" "$KUBECTL_BINARY"
install -m 0755 "$KUBECTL_BINARY" "$TARGET_DIR/usr/bin/kubectl"
log "kubelet/kubectl $KUBERNETES_VERSION installed"

# --- CNI plugins ---------------------------------------------------------------
# Only the plugin BINARIES: the CNI configuration is written by the cluster
# (DaemonSet) or by the node configuration.
CNI_TARBALL="$DL_DIR/cni-plugins-linux-arm64-v$CNI_VERSION.tgz"
fetch "$CNI_URL" "$CNI_SHA256" "$CNI_TARBALL"

CNI_TMP="$DL_DIR/cni.$$"
rm -rf "$CNI_TMP"
mkdir -p "$CNI_TMP" "$TARGET_DIR/opt/cni/bin"
tar -xzf "$CNI_TARBALL" -C "$CNI_TMP"
for plugin in "$CNI_TMP"/*; do
	[ -f "$plugin" ] || continue
	install -m 0755 "$plugin" "$TARGET_DIR/opt/cni/bin/$(basename -- "$plugin")"
done
rm -rf "$CNI_TMP"
log "CNI plugins v$CNI_VERSION installed in /opt/cni/bin"

# --- cosign public key --------------------------------------------------------
# The private key never leaves the CI secrets.
install -d "$TARGET_DIR/etc/nrzube"
if [ -n "${COSIGN_PUBLIC_KEY_PATH:-}" ] && [ -f "${COSIGN_PUBLIC_KEY_PATH}" ]; then
	install -m 0644 "${COSIGN_PUBLIC_KEY_PATH}" "$TARGET_DIR/etc/nrzube/cosign.pub"
	log "cosign public key installed"
else
	log "WARNING: no cosign public key provided: the agent will refuse every update (fail-closed)"
	cat > "$TARGET_DIR/etc/nrzube/cosign.pub" <<'EOF'
# No cosign public key was embedded at build time.
#
# The agent treats this file as "no trusted key": every OTA update is then
# refused (fail-closed). A new image embedding a real key is required.
EOF
fi

# --- Runtime directories -------------------------------------------------------
install -d "$TARGET_DIR/var/lib/nrzube" "$TARGET_DIR/var/lib/kubelet" \
	"$TARGET_DIR/var/lib/containerd" "$TARGET_DIR/run" "$TARGET_DIR/tmp"

# --- Sanity checks -------------------------------------------------------------
[ -x "$TARGET_DIR/init" ] || die "/init is missing from the rootfs overlay"
[ -x "$TARGET_DIR/usr/bin/nrzube-agent" ] || die "the nrzube-agent binary is missing"

log "post-build done"
