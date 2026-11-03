# =============================================================================
# nrzube — external tree make fragment.
#
# Buildroot includes this file with the rest of its make logic: it is the
# place where the packages of the external tree are declared.
# =============================================================================

include $(sort $(wildcard $(BR2_EXTERNAL_NRZUBE_PATH)/package/*/*.mk))

# The kernel is fetched from a git repository: Buildroot generates the
# archive itself, so there is no upstream hash to compare it with. The pinned
# commit is verified after the checkout instead (scripts/build-rootfs.sh),
# which is a stronger guarantee than an archive digest.
NRZUBE_KERNEL_TAG ?= $(KERNEL_TAG)
ifneq ($(NRZUBE_KERNEL_TAG),)
BR_NO_CHECK_HASH_FOR += linux-$(NRZUBE_KERNEL_TAG)$(BR_FMT_VERSION_git).tar.gz
endif

# -----------------------------------------------------------------------------
# Workaround for Buildroot 2026.08: the staging sysroot of an external
# toolchain is populated AFTER the "lib" symlinks are created, but the
# compiler wrapper already points to it. The compiler therefore cannot find
# libc.a and the installation fails with
#   "ln: failed to create symbolic link .../sysroot/usr/lib".
#
# The sysroot is copied as soon as the toolchain is installed, so that the
# staging directory is consistent when the symlinks are created.
# -----------------------------------------------------------------------------
ifeq ($(BR2_TOOLCHAIN_EXTERNAL_BOOTLIN),y)

define NRZUBE_BOOTLIN_SYSROOT_SETUP
	mkdir -p $(STAGING_DIR) $(TARGET_DIR)/usr ; \
	if [ -d "$(HOST_DIR)/opt/ext-toolchain/$(GNU_TARGET_NAME)/sysroot" ] ; then \
		rsync -a --exclude 'locale/' \
			"$(HOST_DIR)/opt/ext-toolchain/$(GNU_TARGET_NAME)/sysroot/" \
			"$(STAGING_DIR)/" ; \
	fi ; \
	true
endef

define TOOLCHAIN_EXTERNAL_BOOTLIN_INSTALL_STAGING_CMDS
	$(call NRZUBE_BOOTLIN_SYSROOT_SETUP)
	$(call TOOLCHAIN_WRAPPER_INSTALL)
	$(call TOOLCHAIN_EXTERNAL_CREATE_STAGING_LIB_SYMLINK)
	$(call TOOLCHAIN_EXTERNAL_INSTALL_SYSROOT_LIBS)
	$(call TOOLCHAIN_EXTERNAL_GLIBC_NO_LIBCRYPT)
	$(call TOOLCHAIN_EXTERNAL_INSTALL_WRAPPER)
	$(call TOOLCHAIN_EXTERNAL_INSTALL_GDBINIT)
	$(call TOOLCHAIN_EXTERNAL_FIXUP_PRETTY_PRINTER_LOADER)
endef

define TOOLCHAIN_EXTERNAL_BOOTLIN_INSTALL_TARGET_CMDS
	$(call NRZUBE_BOOTLIN_SYSROOT_SETUP)
	$(call TOOLCHAIN_EXTERNAL_CREATE_TARGET_LIB_SYMLINK)
	$(call TOOLCHAIN_EXTERNAL_INSTALL_TARGET_LIBS)
	$(call TOOLCHAIN_EXTERNAL_INSTALL_TARGET_GDBSERVER)
	$(call TOOLCHAIN_EXTERNAL_FIXUP_UCLIBCNG_LDSO)
	$(call TOOLCHAIN_EXTERNAL_INSTALL_TARGET_LDD)
endef

endif
