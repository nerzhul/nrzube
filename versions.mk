# nrzube — variables derived from versions.lock
#
# Both versions AND their URLs are derived here: a Renovate version bump can
# therefore never leave a stale URL behind.

include $(ROOT)/versions.lock

# Version of the produced image (override: make image NRZUBE_VERSION=2026.11.02)
NRZUBE_VERSION ?= $(shell date -u +%Y.%m.%d)

# OTA update channel
NRZUBE_CHANNEL ?= stable

# cosign public key (PEM) embedded in the image. The private key only ever
# lives in the CI secrets: never in the image, never in the repository.
COSIGN_PUBLIC_KEY_PATH ?= $(ROOT)/keys/cosign.pub

# --- Artifact URLs (built from the versions) ---------------------------------

BUILDROOT_URL := $(subst @VERSION@,$(BUILDROOT_VERSION),$(BUILDROOT_URL_TEMPLATE))

UBOOT_URL := $(subst @VERSION@,$(UBOOT_TAG),$(UBOOT_URL_TEMPLATE))

FIRMWARE_URL := $(subst @VERSION@,$(FIRMWARE_TAG),$(FIRMWARE_URL_TEMPLATE))

CONTAINERD_URL := $(subst @VERSION@,$(CONTAINERD_VERSION),$(CONTAINERD_URL_TEMPLATE))

RUNC_URL := $(subst @VERSION@,$(RUNC_VERSION),$(RUNC_URL_TEMPLATE))

KUBERNETES_BASE_URL := $(subst @VERSION@,$(KUBERNETES_VERSION),$(KUBERNETES_URL_TEMPLATE))
KUBELET_URL := $(KUBERNETES_BASE_URL)/kubelet
KUBECTL_URL := $(KUBERNETES_BASE_URL)/kubectl

CNI_URL := $(subst @VERSION@,$(CNI_VERSION),$(CNI_URL_TEMPLATE))

COSIGN_URL := $(subst @VERSION@,$(COSIGN_VERSION),$(COSIGN_URL_TEMPLATE))

DHCPCD_URL := $(subst @VERSION@,$(DHCPCD_VERSION),$(DHCPCD_URL_TEMPLATE))

CHRONY_URL := $(subst @VERSION@,$(CHRONY_VERSION),$(CHRONY_URL_TEMPLATE))

# --- Kernel tree -------------------------------------------------------------

KERNEL_REPO_URL := https://github.com/$(KERNEL_REPO).git
FIRMWARE_REPO_URL := https://github.com/$(FIRMWARE_REPO).git

# --- Guard rails (see plan §3.3) ---------------------------------------------
# runc < 1.5.2 is CORRUPTED on kernels >= 6.17: refuse to build.
RUNC_MIN_VERSION := 1.5.2
