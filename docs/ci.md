# CI/CD

## Overview

nrzube uses **GitHub Actions** for CI/CD with native ARM64 runners (`ubuntu-24.04-arm`).
The workflow is split into two parts:

- **CI (Continuous Integration)**: Build and verify on every PR/push to `main`
- **Release**: Full build, sign, and publish on tag pushes (`v*`)

## Workflows

### CI Workflow (`ci.yml`)

**Trigger**: Pull requests to `main`, pushes to `main`

**Steps**:
1. Checkout the repository
2. Install system dependencies (bc, build-essential, genimage, mtools, etc.)
3. Build the nrzube image (configurable: `full` or `boot-gate`)
4. Verify build outputs (file existence, SHA-256 checksums, JSON validity)
5. Run Rust agent unit tests (if applicable)
6. Upload artifacts for debugging

**Usage**:
```yaml
# Manual trigger with specific target
workflow_dispatch:
  inputs:
    target:
      description: 'Build target'
      required: true
      default: 'full'
      type: choice
      options:
        - full
        - boot-gate
```

### Release Workflow (`release.yml`)

**Trigger**: Tag pushes matching `v*` (e.g., `v1.2.3`)

**Steps**:
1. Checkout the repository
2. Install system dependencies
3. Install cosign and ORAS
4. Full build of nrzube image
5. Generate SHA-256 checksums and release manifest
6. **Sign the release manifest with cosign**
7. **Push to GHCR** (OCI artifacts)
8. **Create GitHub Release** with all artifacts

## Setup

### Prerequisites

1. **ARM64 Runner Access**: Ensure your GitHub organization has access to `ubuntu-24.04-arm` runners.
   - Free for public repositories
   - Requires GitHub Enterprise for private repositories (or self-hosted ARM64 runners)

2. **Repository Secrets**:
   
   | Secret | Purpose | Required |
   |--------|---------|----------|
   | `COSIGN_PRIVATE_KEY` | Base64-encoded PEM private key for signing manifests | ✅ |
   | `COSIGN_PASSWORD` | Password for the private key (optional) | ❌ |

### Generating Cosign Keys

For production use, generate keys on a **secure, air-gapped machine**:

```bash
# 1. Generate the key pair (ECDSA P-256)
cosign generate-key-pair --kms none --algorithm ecdsa-sha256

# This creates:
# - cosign.key (private key, PEM format)
# - cosign.pub (public key, PEM format)

# 2. Prepare for GitHub Secrets
#    The private key must be base64-encoded:
base64 -w0 cosign.key > cosign.key.b64

#    Copy the content of cosign.key.b64 and add it as a GitHub Actions secret:
#    Settings > Secrets > Actions > New repository secret
#    Name: COSIGN_PRIVATE_KEY
#    Value: <content of cosign.key.b64>

# 3. Embed the public key in the image
#    Copy cosign.pub to the Buildroot overlay:
mkdir -p buildroot/board/nrzube/rootfs-overlay/etc/nrzube
cp cosign.pub buildroot/board/nrzube/rootfs-overlay/etc/nrzube/cosign.pub

# 4. Clean up (VERY IMPORTANT)
shred -u cosign.key cosign.key.b64
```

**⚠️ WARNING**: Never commit `cosign.key` to the repository. The private key **must** remain secret.

### For Development/Testing

Use the helper script to generate a test key pair:

```bash
./scripts/gen-cosign-keypair.sh .

# This creates:
# - cosign.key (private key)
# - cosign.pub (public key)
# - etc/nrzube/cosign.pub (for embedding)

# Add to GitHub Secrets:
base64 -w0 cosign.key | pbcopy  # macOS
# Or: base64 -w0 cosign.key | xclip -selection clipboard  # Linux
```

## Release Process

### Automatic (Recommended)

1. Update versions in `versions.lock`:
   ```bash
   # Manual update
   vim versions.lock
   
   # OR use Renovate (recommended)
   # Renovate will automatically create PRs for version updates
   ```

2. Update changelog (optional but recommended):
   ```bash
   vim CHANGELOG.md
   ```

3. Commit and tag:
   ```bash
   git add versions.lock CHANGELOG.md
   git commit -m "Prepare release v1.2.3"
   git tag v1.2.3
   git push origin v1.2.3
   ```

4. The release workflow runs automatically:
   - Builds the full image
   - Signs the manifest
   - Pushes to GHCR
   - Creates a GitHub Release

### Manual

For testing or local releases:

```bash
# 1. Build the image
./scripts/native-build.sh

# 2. Set environment variables
export COSIGN_PRIVATE_KEY="$(base64 -w0 path/to/cosign.key)"
export COSIGN_PASSWORD="your-password"  # if applicable
export GHCR_TOKEN="$(gh auth token)"
export GITHUB_REPOSITORY="owner/repo"

# 3. Sign and push
tag="v$(date +%Y.%m.%d)"
./scripts/ci-sign-and-push.sh dist/ "$tag" --github-release
```

## Artifacts

### OCI Artifacts (GHCR)

Each release pushes to GHCR as OCI artifacts:

```
ghcr.io/owner/repo/nrzube:v1.2.3
├── nrzube.img              (type: application/vnd.nrzube.image.v1)
├── release.json            (type: application/json)
└── release.json.sig        (type: application/vnd.cosign.bundle)
```

**Pulling artifacts**:

```bash
# Pull all artifacts for a specific version
oras pull ghcr.io/owner/repo/nrzube:v1.2.3 -o nrzube-v1.2.3

# Pull just the image
oras pull ghcr.io/owner/repo/nrzube:v1.2.3:application/vnd.nrzube.image.v1 -o nrzube.img
```

### GitHub Release Assets

Each GitHub Release includes:

- `nrzube.img` - The disk image (GPT layout with boot, slot A/B, state partitions)
- `release.json` - Release manifest with component SHA-256 hashes
- `release.json.sig` - Cosign signature bundle
- `SHA256SUMS` - Checksums of all artifacts

## Verification

### Verify Image Integrity

```bash
# Download the release artifacts
wget https://github.com/owner/repo/releases/download/v1.2.3/nrzube.img
wget https://github.com/owner/repo/releases/download/v1.2.3/SHA256SUMS

# Verify the hash
sha256sum -c SHA256SUMS
```

### Verify Manifest Signature

```bash
# Download the manifest and signature
wget https://github.com/owner/repo/releases/download/v1.2.3/release.json
wget https://github.com/owner/repo/releases/download/v1.2.3/release.json.sig

# With the embedded public key (from the image)
cosign verify-blob --key /etc/nrzube/cosign.pub --bundle release.json.sig release.json

# With a local public key
cosign verify-blob --key cosign.pub --bundle release.json.sig release.json
```

### Verify OCI Artifacts

```bash
# Pull the manifest and signature
oras pull ghcr.io/owner/repo/nrzube:v1.2.3 -o temp
cd temp

# Verify
cosign verify-blob --key cosign.pub --bundle release.json.sig release.json
```

## Troubleshooting

### Common Issues

#### Build Fails with Missing Dependencies

**Error**: `build-image: host tool 'genimage' not found`

**Solution**: Install the required packages:

```bash
sudo apt update
sudo apt install -y bc build-essential fakeroot genimage mtools squashfs-tools u-boot-tools
```

#### Cosign Not Found

**Error**: `cosign: command not found`

**Solution** (for local testing):

```bash
# Install cosign
curl -LO https://github.com/sigstore/cosign/releases/latest/download/cosign-linux-amd64
sudo mv cosign-linux-amd64 /usr/local/bin/cosign
chmod +x /usr/local/bin/cosign
```

#### ORAS Not Found

**Error**: `oras: command not found`

**Solution**:

```bash
curl -LO https://github.com/oras-project/oras/releases/latest/download/oras_*_linux_arm64.tar.gz
sudo tar -xzf oras_*.tar.gz -C /usr/local/bin oras
rm oras_*.tar.gz
```

#### ARM64 Runner Not Available

**Error**: `Error: No available runners...`

**Solution**:
- For public repositories: ARM64 runners are available by default
- For private repositories: You need GitHub Enterprise or self-hosted ARM64 runners

### Debugging Builds

To debug a failed build:

1. **Download artifacts** from the workflow run (GitHub Actions UI)
2. **Reproduce locally**:
   ```bash
   # Clean build
   rm -rf build/ output/ dist/
   
   # Run the same command as CI
   ./scripts/build-all.sh $(pwd) build/buildroot output dist src full
   ```
3. **Check logs**: The workflow logs show exactly which command failed

## Security Considerations

### Private Key Protection

- The private key is **only available in CI secrets** (encrypted at rest)
- The private key is **never logged** or exposed in artifacts
- The private key is **only used for signing** the release manifest
- The private key is **not included in the image**

### Public Key Distribution

- The public key is **embedded in the image** at `/etc/nrzube/cosign.pub`
- Nodes verify updates using **only the embedded public key**
- The public key can be rotated by releasing a new image with the new public key

### Trust Chain

The trust chain is:

```
Embedded Public Key
    ↓ (verifies)
Release Manifest Signature
    ↓ (contains)
Component SHA-256 Hashes
    ↓ (verifies)
Downloaded Components (kernel, rootfs, initrd, DTB)
```

This ensures:
1. **Offline verification**: Nodes don't need network access to verify updates
2. **Fail-closed**: If verification fails, the update is rejected
3. **No external dependencies**: No need for Fulcio/Rekor/TUF on the node

## Performance

### Build Times

| Target | Estimated Time | Notes |
|--------|---------------|-------|
| boot-gate | 15-20 min | Minimal build for hardware validation |
| full | 30-60 min | Complete image with all components |

### Optimization Tips

1. **Cache Buildroot downloads**: GitHub Actions automatically caches `/usr/local` and `~/.cache`
2. **Use ccache**: Can be added to speed up rebuilds
3. **Parallel builds**: The workflow already uses multiple cores

## Customization

### Changing the Artifact Type

The OCI artifact type is defined in the workflow:

```yaml
env:
  ARTIFACT_TYPE: "application/vnd.nrzube.image.v1"
```

To change it, update this value in both `release.yml` and `ci-sign-and-push.sh`.

### Adding More Components

To add more components to the release:

1. Add the component to `build-image.sh` (generate SHA-256 and add to manifest)
2. Update the ORAS push command in `release.yml`
3. Update the verification logic in the agent

### Changing the Signing Algorithm

The workflow uses ECDSA P-256. To change:

1. Generate new keys with the desired algorithm:
   ```bash
   cosign generate-key-pair --kms none --algorithm <algorithm>
   ```
2. Update the verification code in the agent
3. Update the public key embedding location

Supported algorithms: `ecdsa-sha256`, `ed25519`, `rsa-sha256`

## References

- [GitHub Actions Documentation](https://docs.github.com/en/actions)
- [Cosign Documentation](https://docs.sigstore.dev/cosign/overview/)
- [ORAS Documentation](https://oras.land/docs/)
- [OCI Artifacts](https://github.com/opencontainers/artifacts)
