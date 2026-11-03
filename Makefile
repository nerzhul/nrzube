# nrzube — minimal OS image for Raspberry Pi 5 (Kubernetes worker)
#
# This repository builds a bootable disk image for the RPi 5 that:
#   - starts a DHCP + IPv6 SLAAC client (dhcpcd);
#   - runs containerd + kubelet and joins an existing Kubernetes control plane;
#   - exposes a Rust agent (health + metrics on 127.0.0.1:12249);
#   - applies signed OTA updates (cosign) with automatic A/B rollback.
#
# No distro: Buildroot (a build system, not a distro) + compiled glibc,
# s6-linux as supervisor, U-Boot chainloaded by the RPi firmware.
#
# The reference plan (source of truth) lives in
# .kilo/plans/1793628833208-nrzube-rpi5-k8s-node-image.md

SHELL := /bin/bash
.SHELLFLAGS := -eu -o pipefail -c
.DEFAULT_GOAL := help

# Repository root
ROOT := $(patsubst %/,%,$(dir $(abspath $(lastword $(MAKEFILE_LIST)))))

# Buildroot tree
BR2_EXTERNAL := $(ROOT)/buildroot
BR2_DIR      := $(ROOT)/build/buildroot
OUT_DIR      := $(ROOT)/build/out
DIST_DIR     := $(ROOT)/dist

# Versions (single source of truth: versions.lock)
include $(ROOT)/versions.mk

# Container engine used by `make image`
CONTAINER_ENGINE ?= $(shell command -v podman >/dev/null 2>&1 && echo podman || echo docker)

# Build image (Debian + cross toolchain + Rust)
BUILD_IMAGE ?= ghcr.io/nrzube/buildroot-builder:2026.08.1

# Target device for `make flash` (e.g. /dev/sdb) — never guessed
FLASH_DEVICE ?=

.PHONY: help
help: ## Show this help
	@echo "nrzube — available targets:"
	@grep -hE '^[a-zA-Z0-9_.-]+:.*?## .*$$' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-22s\033[0m %s\n", $$1, $$2}'
	@echo ""
	@echo "Useful variables: CONTAINER_ENGINE=$(CONTAINER_ENGINE) FLASH_DEVICE=<device>"

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------

.PHONY: image
image: ## Build the complete image (rootfs + initramfs + GPT) inside a container
	$(CONTAINER_ENGINE) run --rm \
		--privileged \
		-v "$(ROOT):/src" \
		-v "$(ROOT)/build:/build" \
		-e "BR2_EXTERNAL=$(BR2_EXTERNAL)" \
		-e "OUT_DIR=$(OUT_DIR)" \
		-e "DIST_DIR=$(DIST_DIR)" \
		-e "NRZUBE_VERSION=$(NRZUBE_VERSION)" \
		-e "COSIGN_PUBLIC_KEY_PATH=$(COSIGN_PUBLIC_KEY_PATH)" \
		$(BUILD_IMAGE) \
		/scripts/build-container.sh

.PHONY: boot-gate
boot-gate: ## Build the boot gate image (kernel + U-Boot + stage 1 initramfs)
	./scripts/native-build.sh boot-gate

.PHONY: native-build
native-build: ## Build the complete image on the host (no container)
	./scripts/native-build.sh full

.PHONY: rootfs
rootfs: ## Build only the squashfs rootfs (Buildroot)
	$(CONTAINER_ENGINE) run --rm \
		-v "$(ROOT):/src" \
		-v "$(ROOT)/build:/build" \
		-e "BR2_EXTERNAL=$(BR2_EXTERNAL)" \
		-e "OUT_DIR=$(OUT_DIR)" \
		$(BUILD_IMAGE) \
		/scripts/build-rootfs.sh

.PHONY: initramfs
initramfs: ## Build only the initramfs (offline, slot selection)
	$(ROOT)/scripts/build-initramfs.sh "$(OUT_DIR)"

.PHONY: agent
agent: ## Cross-compile the Rust agent for aarch64 (static musl)
	cd $(ROOT)/agent && cargo build --release --target aarch64-unknown-linux-musl

.PHONY: clean
clean: ## Remove build artifacts
	rm -rf $(ROOT)/build $(ROOT)/dist $(ROOT)/agent/target

.PHONY: distclean
distclean: clean ## Also remove the Buildroot download cache
	rm -rf $(BR2_DIR)/dl

# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

.PHONY: test
test: test-agent test-scripts ## Run the whole local test suite

.PHONY: test-agent
test-agent: ## Unit tests of the Rust agent (incl. real cosign fixtures)
	cd $(ROOT)/agent && cargo test --workspace --all-targets

.PHONY: test-scripts
test-scripts: ## Check shell script syntax and repository consistency
	@set -e; \
	for f in $(ROOT)/scripts/*.sh $(ROOT)/buildroot/board/nrzube/*.sh $(ROOT)/buildroot/board/nrzube/initramfs/init; do \
		[ -f "$$f" ] || continue; \
		sh -n "$$f" || { echo "invalid syntax: $$f"; exit 1; }; \
	done; \
	echo "scripts OK"

.PHONY: fixtures
fixtures: ## Regenerate the cosign test fixtures (requires the cosign binary)
	$(ROOT)/scripts/gen-cosign-fixtures.sh

# ---------------------------------------------------------------------------
# Versions / lock
# ---------------------------------------------------------------------------

.PHONY: versions-lock
versions-lock: ## Re-resolve COMMITs from TAGs and verify the SHA-256 digests
	$(ROOT)/scripts/refresh-versions-lock.sh

# ---------------------------------------------------------------------------
# Flash / diagnostics
# ---------------------------------------------------------------------------

.PHONY: flash
flash: ## Write the image to an SD card (FLASH_DEVICE=/dev/sdX)
	@if [ -z "$(FLASH_DEVICE)" ]; then \
		echo "ERROR: set FLASH_DEVICE (e.g. make flash FLASH_DEVICE=/dev/sdb)"; exit 1; \
	fi
	@test -f "$(DIST_DIR)/nrzube.img" || { echo "ERROR: $(DIST_DIR)/nrzube.img missing - run 'make image' first"; exit 1; }
	@read -r -p "Overwrite $(FLASH_DEVICE)? [y/N] " ans; \
	 case "$$ans" in [yY]*) ;; *) echo "aborted"; exit 1;; esac
	sudo dd if=$(DIST_DIR)/nrzube.img of=$(FLASH_DEVICE) bs=4M conv=fsync status=progress
	sync

.PHONY: sha256
sha256: ## Show the SHA-256 of the produced artifacts
	cd $(DIST_DIR) && sha256sum *.img *.json *.bundle 2>/dev/null || true
