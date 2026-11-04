#!/bin/sh
# =============================================================================
# nrzube — CI Validation Script
#
# Validates that all CI prerequisites are met before pushing.
# Run this locally to catch configuration issues early.
#
# usage: ci-validate.sh [--fix]
#
# Checks:
#   - Required files exist
#   - Workflow files are valid YAML
#   - cosign is available (or can be installed)
#   - ORAS is available (or can be installed)
#   - GitHub Secrets are properly configured (if running in CI)
# =============================================================================
set -eu

REPO_DIR="$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)"
FIX_MODE="${1:-}"

ERRORS=0
WARNINGS=0
TOTAL=0

# Color codes
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

pass() {
	echo -e "${GREEN}✓${NC} $1"
}

warn() {
	echo -e "${YELLOW}⚠${NC} $1"
	WARNINGS=$((WARNINGS + 1))
}

fail() {
	echo -e "${RED}✗${NC} $1"
	ERRORS=$((ERRORS + 1))
}

# -----------------------------------------------------------------------------
# 1. Check required files
# -----------------------------------------------------------------------------
echo "=== Checking required files ==="

REQUIRED_FILES="
.github/workflows/ci.yml
.github/workflows/release.yml
scripts/ci-sign-and-push.sh
scripts/build-all.sh
scripts/build-image.sh
versions.lock
"

for file in $REQUIRED_FILES; do
	if [ -f "$REPO_DIR/$file" ]; then
		pass "$file exists"
	else
		fail "Missing: $file"
	fi
done

# -----------------------------------------------------------------------------
# 2. Check workflow YAML syntax
# -----------------------------------------------------------------------------
echo ""
echo "=== Checking workflow YAML syntax ==="

for wf in .github/workflows/*.yml; do
	if command -v yamllint >/dev/null 2>&1; then
		if yamllint "$REPO_DIR/$wf" >/dev/null 2>&1; then
			pass "$wf: valid YAML"
		else
			fail "$wf: invalid YAML (run yamllint for details)"
		fi
	else
		# Fallback: use Python
		if python3 -c "import yaml; yaml.safe_load(open('$REPO_DIR/$wf'))" 2>/dev/null; then
			pass "$wf: valid YAML"
		else
			fail "$wf: invalid YAML"
		fi
	fi
done

# -----------------------------------------------------------------------------
# 3. Check cosign availability
# -----------------------------------------------------------------------------
echo ""
echo "=== Checking cosign ==="

if command -v cosign >/dev/null 2>&1; then
	VERSION=$(cosign version 2>/dev/null | head -1 || echo "unknown")
	pass "cosign installed ($VERSION)"
else
	warn "cosign not found in PATH"
	warn "  Install with: go install github.com/sigstore/cosign/v3@latest"
	
	if [ "$FIX_MODE" = "--fix" ]; then
		echo "  Attempting to install cosign..."
		if command -v go >/dev/null 2>&1; then
			go install github.com/sigstore/cosign/v3@latest 2>/dev/null || true
			if command -v cosign >/dev/null 2>&1; then
				pass "cosign installed via go"
			else
				warn "Failed to install cosign (go available but install failed)"
			fi
		else
			warn "cosign not installed and go not available"
		fi
	fi
fi

# -----------------------------------------------------------------------------
# 4. Check ORAS availability
# -----------------------------------------------------------------------------
echo ""
echo "=== Checking ORAS ==="

if command -v oras >/dev/null 2>&1; then
	VERSION=$(oras version 2>/dev/null | head -1 || echo "unknown")
	pass "ORAS installed ($VERSION)"
else
	warn "ORAS not found in PATH"
	warn "  Install from: https://oras.land/docs/installation"
	
	if [ "$FIX_MODE" = "--fix" ]; then
		echo "  Attempting to install ORAS..."
		if command -v curl >/dev/null 2>&1; then
			curl -LO https://github.com/oras-project/oras/releases/latest/download/oras_linux_arm64 2>/dev/null || \
			curl -LO https://github.com/oras-project/oras/releases/latest/download/oras_linux_amd64 2>/dev/null || true
			if [ -f oras_linux_* ]; then
				chmod +x oras_linux_*
				sudo mv oras_linux_* /usr/local/bin/oras
				rm -f oras_linux_*
				pass "ORAS installed"
			else
				warn "Failed to download ORAS"
			fi
		else
			warn "ORAS not installed and curl not available"
		fi
	fi
fi

# -----------------------------------------------------------------------------
# 5. Check GitHub CLI (for manual releases)
# -----------------------------------------------------------------------------
echo ""
echo "=== Checking GitHub CLI ==="

if command -v gh >/dev/null 2>&1; then
	VERSION=$(gh version 2>/dev/null | head -1 || echo "unknown")
	pass "GitHub CLI installed ($VERSION)"
else
	warn "GitHub CLI (gh) not installed"
	warn "  Needed for manual releases. Install from: https://cli.github.com"
fi

# -----------------------------------------------------------------------------
# 6. Check Docker/Podman (for container builds)
# -----------------------------------------------------------------------------
echo ""
echo "=== Checking container runtime ==="

HAS_CONTAINER=0
if command -v docker >/dev/null 2>&1; then
	pass "Docker installed"
	HAS_CONTAINER=1
fi

if command -v podman >/dev/null 2>&1; then
	pass "Podman installed"
	HAS_CONTAINER=1
fi

if [ "$HAS_CONTAINER" = "0" ]; then
	warn "No container runtime (Docker/Podman) found"
	warn "  Container builds (local-build.sh) will not work"
fi

# -----------------------------------------------------------------------------
# 7. Check build dependencies
# -----------------------------------------------------------------------------
echo ""
echo "=== Checking build dependencies ==="

BUILD_DEPS="bc build-essential fakeroot genimage mtools squashfs-tools u-boot-tools"
MISSING_DEPS=""

for dep in $BUILD_DEPS; do
	if command -v "$dep" >/dev/null 2>&1 || dpkg -l "$dep" >/dev/null 2>&1; then
		pass "$dep installed"
	else
		MISSING_DEPS="$MISSING_DEPS $dep"
	fi
done

if [ -n "$MISSING_DEPS" ]; then
	warn "Missing build dependencies: $MISSING_DEPS"
	warn "  Install with: sudo apt install$MISSING_DEPS"
fi

# -----------------------------------------------------------------------------
# 8. Check versions.lock consistency
# -----------------------------------------------------------------------------
echo ""
echo "=== Checking versions.lock ==="

if [ -f "$REPO_DIR/versions.lock" ]; then
	# Check for duplicate keys
	DUPLICATES=$(grep -oE '^[A-Z_]+=' "$REPO_DIR/versions.lock" | cut -d'=' -f1 | sort | uniq -d)
	if [ -z "$DUPLICATES" ]; then
		pass "versions.lock: no duplicate keys"
	else
		fail "versions.lock: duplicate keys found: $DUPLICATES"
	fi
	
	# Check for required keys
	REQUIRED_KEYS="BUILDROOT_VERSION KERNEL_VERSION CONTAINERD_VERSION RUNC_VERSION"
	for key in $REQUIRED_KEYS; do
		if grep -q "^${key}=" "$REPO_DIR/versions.lock"; then
			pass "versions.lock: $key defined"
		else
			warn "versions.lock: $key not found"
		fi
	done
else
	fail "versions.lock not found"
fi

# -----------------------------------------------------------------------------
# 9. Check GitHub Secrets (if in CI)
# -----------------------------------------------------------------------------
echo ""
echo "=== Checking GitHub environment ==="

if [ -n "${GITHUB_ACTIONS:-}" ]; then
	# Running in GitHub Actions
	if [ -z "${COSIGN_PRIVATE_KEY:-}" ]; then
		fail "COSIGN_PRIVATE_KEY secret not set (required for release)"
	else
		pass "COSIGN_PRIVATE_KEY secret is set"
	fi
	
	if [ -z "${GITHUB_TOKEN:-}" ]; then
		warn "GITHUB_TOKEN not available (needed for releases)"
	else
		pass "GITHUB_TOKEN is available"
	fi
else
	warn "Not running in GitHub Actions (skipping secret checks)"
fi

# -----------------------------------------------------------------------------
# 10. Check workflow triggers
# -----------------------------------------------------------------------------
echo ""
echo "=== Checking workflow triggers ==="

for wf in .github/workflows/*.yml; do
	WF_PATH="$REPO_DIR/$wf"
	WF_NAME=$(basename "$wf")
	
	# Check for on: push triggers
	if grep -q "on:" "$WF_PATH" && grep -q "push:" "$WF_PATH"; then
		pass "$WF_NAME: has push trigger"
	else
		warn "$WF_NAME: no push trigger found"
	fi
	
	# Check for on: pull_request triggers (for CI workflow)
	if [ "$WF_NAME" = "ci.yml" ] || [ "$WF_NAME" = "ci.yaml" ]; then
		if grep -q "pull_request:" "$WF_PATH"; then
			pass "$WF_NAME: has pull_request trigger"
		else
			fail "$WF_NAME: missing pull_request trigger"
		fi
	fi
	
	# Check for on: push tags trigger (for release workflow)
	if [ "$WF_NAME" = "release.yml" ] || [ "$WF_NAME" = "release.yaml" ]; then
		if grep -q "tags:" "$WF_PATH"; then
			pass "$WF_NAME: has tags trigger"
		else
			fail "$WF_NAME: missing tags trigger"
		fi
	fi
done

# -----------------------------------------------------------------------------
# Summary
# -----------------------------------------------------------------------------
echo ""
echo "=== Summary ==="
echo "Warnings: $WARNINGS"
echo "Errors: $ERRORS"

if [ "$ERRORS" -gt 0 ]; then
	echo ""
	echo -e "${RED}CI validation FAILED${NC}"
	echo "Please fix the errors above before pushing."
	exit 1
elif [ "$WARNINGS" -gt 0 ]; then
	echo ""
	echo -e "${YELLOW}CI validation PASSED with warnings${NC}"
	echo "Consider addressing the warnings above."
	exit 0
else
	echo ""
	echo -e "${GREEN}CI validation PASSED${NC}"
	echo "All checks passed! You can safely push."
	exit 0
fi
