#!/bin/sh
# =============================================================================
# nrzube — generate the cosign test fixtures.
#
# The fixtures are produced by the REAL cosign binary: the bundle format is
# never assumed, it is asserted by the tests.
#
# Produced files (agent/nrzube-agent/tests/fixtures/):
#   cosign.pub                  public key of the fixture key pair
#   manifest.json               the signed payload (a release manifest)
#   manifest.json.bundle        the cosign bundle (valid signature)
#   manifest.json.tampered      a payload altered after the signature
#   manifest.json.corrupt.bundle  a bundle with a corrupted signature
#
# usage: gen-cosign-fixtures.sh [path-to-cosign]
# =============================================================================
set -eu

REPO_DIR="$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)"
FIXTURES_DIR="$REPO_DIR/agent/nrzube-agent/tests/fixtures"
COSIGN="${1:-${COSIGN:-cosign}}"

command -v "$COSIGN" >/dev/null 2>&1 || {
	echo "gen-cosign-fixtures: the cosign binary was not found ($COSIGN)" >&2
	echo "  install it with: go install github.com/sigstore/cosign/cmd/cosign@latest" >&2
	exit 1
}

mkdir -p "$FIXTURES_DIR"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# cosign v3 refuses to sign without an explicit signing configuration: the
# transparency log is disabled, the signature stays fully offline.
"$COSIGN" signing-config create > "$WORK/signing-config.json"

export COSIGN_PASSWORD=""
"$COSIGN" generate-key-pair --output-key-prefix "$WORK/cosign" >/dev/null

# The signed payload: a release manifest.
cat > "$FIXTURES_DIR/manifest.json" <<'EOF'
{
  "schemaVersion": 1,
  "version": "2026.11.02",
  "channel": "stable",
  "components": [
    { "name": "rootfs", "mediaType": "application/vnd.nrzube.rootfs.v1",
      "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
      "size": 268435456, "ociDigest": "sha256:0000000000000000000000000000000000000000000000000000000000000000" }
  ]
}
EOF

"$COSIGN" sign-blob \
	--key "$WORK/cosign.key" \
	--bundle "$FIXTURES_DIR/manifest.json.bundle" \
	--signing-config "$WORK/signing-config.json" \
	--yes \
	"$FIXTURES_DIR/manifest.json" >/dev/null

# The tampered payload: the signature must not verify it.
sed 's/2026.11.02/2026.11.03/' "$FIXTURES_DIR/manifest.json" \
	> "$FIXTURES_DIR/manifest.json.tampered"

# The corrupted bundle: the signature bytes are altered.
python3 - "$FIXTURES_DIR/manifest.json.bundle" "$FIXTURES_DIR/manifest.json.corrupt.bundle" <<'EOF'
import json
import sys

src, dst = sys.argv[1], sys.argv[2]
with open(src, encoding="utf-8") as handle:
    bundle = json.load(handle)

signature = bundle["messageSignature"]["signature"]
flipped = ("B" if signature[0] != "B" else "C") + signature[1:]
bundle["messageSignature"]["signature"] = flipped

with open(dst, "w", encoding="utf-8") as handle:
    json.dump(bundle, handle)
EOF

cp "$WORK/cosign.pub" "$FIXTURES_DIR/cosign.pub"
chmod 0644 "$FIXTURES_DIR"/*

echo "gen-cosign-fixtures: fixtures written to $FIXTURES_DIR"
ls -l "$FIXTURES_DIR"
