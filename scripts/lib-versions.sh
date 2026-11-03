# =============================================================================
# nrzube — versions for the shell scripts.
#
# This file is sourced by the build scripts. It loads versions.lock and
# derives the URLs from the templates it contains, so that the shell and the
# make build always agree.
#
# usage: . scripts/lib-versions.sh
# =============================================================================

NRZUBE_LIB_DIR="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")" && pwd)"
NRZUBE_REPO_DIR="$(CDPATH='' cd -- "$NRZUBE_LIB_DIR/.." && pwd)"

set -a
# shellcheck disable=SC1091
. "$NRZUBE_REPO_DIR/versions.lock"
set +a

# substitute_version <template> <version>
substitute_version() {
	printf '%s\n' "$1" | sed "s|@VERSION@|$2|g"
}

BUILDROOT_URL="$(substitute_version "$BUILDROOT_URL_TEMPLATE" "$BUILDROOT_VERSION")"
UBOOT_URL="$(substitute_version "$UBOOT_URL_TEMPLATE" "$UBOOT_TAG")"
FIRMWARE_URL="$(substitute_version "$FIRMWARE_URL_TEMPLATE" "$FIRMWARE_TAG")"
CONTAINERD_URL="$(substitute_version "$CONTAINERD_URL_TEMPLATE" "$CONTAINERD_VERSION")"
RUNC_URL="$(substitute_version "$RUNC_URL_TEMPLATE" "$RUNC_VERSION")"
KUBERNETES_BASE_URL="$(substitute_version "$KUBERNETES_URL_TEMPLATE" "$KUBERNETES_VERSION")"
KUBELET_URL="$KUBERNETES_BASE_URL/kubelet"
KUBECTL_URL="$KUBERNETES_BASE_URL/kubectl"
CNI_URL="$(substitute_version "$CNI_URL_TEMPLATE" "$CNI_VERSION")"
COSIGN_URL="$(substitute_version "$COSIGN_URL_TEMPLATE" "$COSIGN_VERSION")"
DHCPCD_URL="$(substitute_version "$DHCPCD_URL_TEMPLATE" "$DHCPCD_VERSION")"

RUNC_MIN_VERSION="${RUNC_MIN_VERSION:-1.5.2}"
export RUNC_MIN_VERSION
