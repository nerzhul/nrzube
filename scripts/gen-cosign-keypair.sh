#!/bin/sh
# =============================================================================
# nrzube — Generate cosign key pair for development/testing
#
# Generates a new ECDSA P-256 key pair for signing release manifests.
# The public key is saved to etc/nrzube/cosign.pub (to be embedded in the image),
# and the private key is saved to cosign.key (to be used in CI secrets).
#
# usage: gen-cosign-keypair.sh <output-dir>
#
# Example:
#   ./gen-cosign-keypair.sh .
#   # This creates:
#   # - cosign.key (private key, PEM format)
#   # - cosign.pub (public key, PEM format)
#   # - etc/nrzube/cosign.pub (public key for embedding in the image)
# =============================================================================
set -eu

OUTPUT_DIR="${1:?usage: $0 <output-dir>}"
KEY_NAME="cosign"

# Create output directories
mkdir -p "$OUTPUT_DIR/etc/nrzube"

# Generate the key pair (ECDSA P-256)
echo "Generating ECDSA P-256 key pair..."
cosign generate-key-pair --kms none \
	--algorithm "ecdsa-sha256" \
	--out "$OUTPUT_DIR/$KEY_NAME"

# The files are generated as cosign and cosign.pub
PRIVATE_KEY="$OUTPUT_DIR/$KEY_NAME"
PUBLIC_KEY="$OUTPUT_DIR/$KEY_NAME.pub"

# Verify the files exist
[ -f "$PRIVATE_KEY" ] || { echo "Private key generation failed"; exit 1; }
[ -f "$PUBLIC_KEY" ] || { echo "Public key generation failed"; exit 1; }

# Copy the public key to the embedding location
cp "$PUBLIC_KEY" "$OUTPUT_DIR/etc/nrzube/cosign.pub"

# Verify the key pair works
echo "Testing key pair..."
echo "test data" | cosign sign-blob --key "$PRIVATE_KEY" --bundle test.sig
cosign verify-blob --key "$PUBLIC_KEY" --bundle test.sig test.data || {
	echo "Key pair verification failed";
	rm -f test.sig test.data
	exit 1
}
rm -f test.sig test.data

echo "Key pair generated successfully:"
echo "  Private key: $PRIVATE_KEY"
echo "  Public key:  $PUBLIC_KEY"
echo "  Embedded public key: $OUTPUT_DIR/etc/nrzube/cosign.pub"
echo ""
echo "To use in CI:"
echo "  1. Base64 encode the private key:"
echo "     base64 -w0 $PRIVATE_KEY"
echo "  2. Add it as a GitHub Actions secret: COSIGN_PRIVATE_KEY"
echo ""
echo "To embed in the image:"
echo "  - The public key at $OUTPUT_DIR/etc/nrzube/cosign.pub will be included"
echo "    in the rootfs overlay (Buildroot will copy it to /etc/nrzube/cosign.pub)"
