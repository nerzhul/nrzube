# nrzube CI/CD

This directory contains GitHub Actions workflows for building, testing, signing, and releasing nrzube.

## Workflows

| File | Trigger | Purpose |
|------|---------|---------|
| [`ci.yml`](./workflows/ci.yml) | PR to main, push to main | Build and verify (no signing/publishing) |
| [`release.yml`](./workflows/release.yml) | Tag push (`v*`) | Full build, sign, push to GHCR, create GitHub Release |

## Setup

### Prerequisites

1. **GitHub Actions ARM64 Runner**: The workflows require the `ubuntu-24.04-arm` runner.
   - GitHub provides this as a standard runner (no self-hosting needed).

2. **Repository Secrets**: Configure these in `Settings > Secrets > Actions`:
   
   | Secret | Description | Required |
   |--------|-------------|----------|
   | `COSIGN_PRIVATE_KEY` | Base64-encoded PEM private key for signing | ✅ (release only) |
   | `COSIGN_PASSWORD` | Password for the private key (empty if none) | ❌ |

### Generating Cosign Keys

To generate a key pair for development/testing:

```bash
# Generate the key pair
./scripts/gen-cosign-keypair.sh .

# The private key (cosign.key) needs to be base64-encoded for the secret:
base64 -w0 cosign.key | pbcopy  # macOS
base64 -w0 cosign.key | xclip -selection clipboard  # Linux

# Then paste it as the COSIGN_PRIVATE_KEY secret in GitHub.

# The public key (etc/nrzube/cosign.pub) must be added to the Buildroot overlay:
cp etc/nrzube/cosign.pub buildroot/board/nrzube/rootfs-overlay/etc/nrzube/cosign.pub
```

**Important**: For production, generate the key pair on a secure machine and never commit the private key to the repository.

### GitHub Container Registry (GHCR)

The workflows automatically push to GHCR using the `GITHUB_TOKEN` (no additional secrets needed for public repositories). For private repositories, ensure the token has `write:packages` permissions.

## Local Testing

### Full Build

```bash
# Native build (requires all dependencies)
./scripts/native-build.sh

# OR container build (using Docker/Podman)
./scripts/local-build.sh
```

### Sign and Push (Manual)

```bash
# Build the image first
./scripts/native-build.sh

# Then sign and push
export COSIGN_PRIVATE_KEY="$(base64 -w0 path/to/cosign.key)"
export COSIGN_PASSWORD="your-password-if-any"
export GHCR_TOKEN="$(gh auth token)"
export GITHUB_REPOSITORY="$(gh repo view --json name -q '.name')"

./scripts/ci-sign-and-push.sh dist/ "$(date +%Y.%m.%d)" --github-release
```

## Release Process

1. **Bump versions** in `versions.lock` (or let Renovate do it)
2. **Update changelog** in `CHANGELOG.md` (if applicable)
3. **Commit and tag**:
   ```bash
   git add versions.lock CHANGELOG.md
   git commit -m "Release v1.2.3"
   git tag v1.2.3
   git push origin v1.2.3
   ```
4. **The release workflow** will automatically:
   - Build the image
   - Generate SHA-256 checksums
   - Sign the manifest with cosign
   - Push to GHCR
   - Create a GitHub Release with all artifacts

## Troubleshooting

### Common Issues

| Issue | Solution |
|-------|----------|
| `ubuntu-24.04-arm runner not available` | Ensure your organization has access to ARM64 runners. Free for public repos. |
| `cosign: command not found` | The workflow installs it automatically. For local testing: `go install github.com/sigstore/cosign/v3@latest` |
| `oras: command not found` | The workflow installs it automatically. For local testing: see [ORAS installation](https://oras.land/docs/installation) |
| Build fails with missing dependencies | Install them: `sudo apt install bc build-essential fakeroot genimage mtools squashfs-tools u-boot-tools` |

### Debugging Builds

To debug a failed CI run:

1. Download the artifacts from the workflow run
2. Inspect the logs for the exact failure
3. Reproduce locally:
   ```bash
   # Clone the PR branch
   git checkout the-pr-branch
   
   # Run the same build command as CI
   ./scripts/build-all.sh $(pwd) build/buildroot output dist src full
   ```

### Clean Build

To force a clean build (remove all build artifacts):

```bash
rm -rf build/ output/ dist/
```

## Artifacts

### OCI Artifacts (GHCR)

The release workflow pushes the following to GHCR:

```
ghcr.io/owner/repo/nrzube:v1.2.3
  ├── nrzube.img              (application/vnd.nrzube.image.v1)
  ├── release.json            (application/json)
  └── release.json.sig        (application/vnd.cosign.bundle)
```

To pull:

```bash
oras pull ghcr.io/owner/repo/nrzube:v1.2.3 -o nrzube.img
```

### GitHub Release Assets

Each release includes:

- `nrzube.img` - The disk image (GPT layout)
- `release.json` - Release manifest with component SHA-256 hashes
- `release.json.sig` - Cosign signature bundle
- `SHA256SUMS` - Checksums of all artifacts

## Verification

### Verify Image Integrity

```bash
# Check the SHA-256 hash
sha256sum nrzube.img
# Compare with SHA256SUMS
```

### Verify Manifest Signature

```bash
# With the embedded public key
cosign verify-blob --key /etc/nrzube/cosign.pub --bundle release.json.sig release.json

# With a specific public key
cosign verify-blob --key cosign.pub --bundle release.json.sig release.json
```

## Security

- **Private keys** are never committed to the repository
- **Signing happens in CI** using GitHub Actions secrets
- **Verification is offline** - nodes only need the embedded public key
- **No network required for boot** - nodes use persisted config

## Performance

The full build takes approximately **30-60 minutes** on ARM64 runners, depending on:
- GitHub Actions runner performance
- Network speed (for downloading dependencies)
- Buildroot configuration

The boot-gate build is faster (~15-20 minutes) as it only builds the minimal boot chain.
