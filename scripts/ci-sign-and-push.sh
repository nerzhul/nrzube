#!/bin/sh
# =============================================================================
# nrzube — CI helper: sign the release manifest and push to GHCR
#
# This script is designed to be used in CI (GitHub Actions) or locally.
# It performs:
#   1. Sign the release manifest with cosign
#   2. Push all artifacts to GHCR using ORAS
#   3. (Optional) Create a GitHub Release
#
# usage: ci-sign-and-push.sh <dist-dir> <version> [--github-release]
#
# Environment variables:
#   COSIGN_PRIVATE_KEY:   Base64-encoded PEM private key for signing
#   COSIGN_PASSWORD:      Password for the private key (optional)
#   GHCR_TOKEN:           GitHub Container Registry token (for ORAS push)
#   GITHUB_REPOSITORY:    Repository name (e.g., "nerzhul/nrzube")
# =============================================================================
set -eu

DIST_DIR="${1:?usage: $0 <dist-dir> <version> [--github-release]}"
VERSION="${2:?usage: $0 <dist-dir> <version> [--github-release]}"
DO_GITHUB_RELEASE="${3:-}"

REPO_DIR="$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)"
MANIFEST_NAME="release.json"
SIGNATURE_NAME="release.json.sig"
IMAGE_NAME="nrzube"

# artifact type for ORAS
ARTIFACT_TYPE="application/vnd.nrzube.image.v1"

# -----------------------------------------------------------------------------
# 1. Verify required files exist
# -----------------------------------------------------------------------------
echo "=== Verifying artifacts ==="
for file in "$DIST_DIR/nrzube.img" "$DIST_DIR/$MANIFEST_NAME" "$DIST_DIR/SHA256SUMS"; do
	[ -f "$file" ] || { echo "Missing file: $file"; exit 1; }
done
echo "All required artifacts present"

# -----------------------------------------------------------------------------
# 2. Sign the release manifest with cosign
# -----------------------------------------------------------------------------
echo "=== Signing release manifest ==="

# Decode the private key
COSIGN_KEY_FILE="$(mktemp).key"
trap "rm -f '$COSIGN_KEY_FILE'" EXIT

if [ -n "${COSIGN_PRIVATE_KEY:-}" ]; then
	echo "$COSIGN_PRIVATE_KEY" | base64 -d > "$COSIGN_KEY_FILE"
	chmod 600 "$COSIGN_KEY_FILE"
else
	echo "COSIGN_PRIVATE_KEY environment variable not set" >&2
	exit 1
fi

# Build cosign arguments
COSIGN_ARGS="--key $COSIGN_KEY_FILE --bundle $DIST_DIR/$SIGNATURE_NAME"
if [ -n "${COSIGN_PASSWORD:-}" ]; then
	COSIGN_ARGS="$COSIGN_ARGS --password $COSIGN_PASSWORD"
fi

# Sign the manifest
cosign sign-blob $COSIGN_ARGS "$DIST_DIR/$MANIFEST_NAME"

# Verify the signature
cosign verify-blob --key "$COSIGN_KEY_FILE" --bundle "$DIST_DIR/$SIGNATURE_NAME" "$DIST_DIR/$MANIFEST_NAME"

echo "Manifest signed and verified"

# -----------------------------------------------------------------------------
# 3. Push to GHCR using ORAS
# -----------------------------------------------------------------------------
echo "=== Pushing to GHCR ==="

# Check for GHCR token
if [ -z "${GHCR_TOKEN:-}" ]; then
	echo "GHCR_TOKEN environment variable not set, skipping ORAS push" >&2
	exit 0
fi

# Configure Docker/ORAS to use GHCR
export ORAS_AUTH_PROVIDER="ghcr"
export ORAS_USERNAME="${GITHUB_ACTOR:-$(whoami)}"
export ORAS_PASSWORD="$GHCR_TOKEN"

# Login to GHCR
echo "$GHCR_TOKEN" | oras login ghcr.io --username "$ORAS_USERNAME" --password-stdin

# Push with version tag
ORAS_REF="ghcr.io/${GITHUB_REPOSITORY:-${REPO_DIR#*/}}/$(basename "$REPO_DIR")/$IMAGE_NAME:$VERSION"

echo "Pushing to $ORAS_REF"
oras push "$ORAS_REF" \
	--artifact-type "$ARTIFACT_TYPE" \
	"$DIST_DIR/nrzube.img:$ARTIFACT_TYPE" \
	"$DIST_DIR/$MANIFEST_NAME:application/json" \
	"$DIST_DIR/$SIGNATURE_NAME:application/vnd.cosign.bundle"

# Also push with latest tag if this is a stable version
if echo "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$'; then
	ORAS_LATEST_REF="ghcr.io/${GITHUB_REPOSITORY:-${REPO_DIR#*/}}/$(basename "$REPO_DIR")/$IMAGE_NAME:latest"
	echo "Pushing to $ORAS_LATEST_REF"
	oras push "$ORAS_LATEST_REF" \
		--artifact-type "$ARTIFACT_TYPE" \
		"$DIST_DIR/nrzube.img:$ARTIFACT_TYPE" \
		"$DIST_DIR/$MANIFEST_NAME:application/json" \
		"$DIST_DIR/$SIGNATURE_NAME:application/vnd.cosign.bundle"
fi

echo "OCI artifacts pushed successfully"

# -----------------------------------------------------------------------------
# 4. (Optional) Create GitHub Release
# -----------------------------------------------------------------------------
if [ "$DO_GITHUB_RELEASE" = "--github-release" ]; then
	echo "=== Creating GitHub Release ==="
	
	# This step is typically done by the GitHub Actions workflow itself
	# using softprops/action-gh-release, but we provide the option for local use
	if [ -z "${GITHUB_TOKEN:-}" ]; then
		echo "GITHUB_TOKEN not set, cannot create GitHub Release" >&2
		exit 0
	fi
	
	# Extract image SHA for the release body
	IMAGE_SHA="$(sha256sum "$DIST_DIR/nrzube.img" | cut -d' ' -f1)"
	
	# Create release using GitHub CLI (gh)
	if command -v gh >/dev/null 2>&1; then
		gh release create "$VERSION" \
			--repo "$GITHUB_REPOSITORY" \
			--title "nrzube $VERSION" \
			--notes "# nrzube Release $VERSION

**Image SHA-256:** \`$IMAGE_SHA\`

## Artifacts
- Disk Image: \`nrzube.img\`
- Release Manifest: \`release.json\`
- Signature Bundle: \`release.json.sig\`
- Checksums: \`SHA256SUMS\`

## OCI Registry
\`ghcr.io/${GITHUB_REPOSITORY}/$IMAGE_NAME:$VERSION\`
\`ghcr.io/${GITHUB_REPOSITORY}/$IMAGE_NAME:latest\`" \
			"$DIST_DIR/nrzube.img" \
			"$DIST_DIR/$MANIFEST_NAME" \
			"$DIST_DIR/$SIGNATURE_NAME" \
			"$DIST_DIR/SHA256SUMS"
	else
		echo "GitHub CLI (gh) not installed, skipping GitHub Release creation" >&2
	fi
fi

echo "=== Sign and push complete ==="
