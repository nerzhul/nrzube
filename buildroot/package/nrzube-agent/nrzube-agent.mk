################################################################################
#
# nrzube-agent
#
# The agent is part of this repository: the source is not downloaded, it is
# taken from the working copy (BR2_EXTERNAL_DIR_PATH/../agent).
#
################################################################################

NRZUBE_AGENT_VERSION = 0.1.0
NRZUBE_AGENT_LICENSE = Apache-2.0
NRZUBE_AGENT_LICENSE_FILES = LICENSE

# host-rust provides cargo and the standard library of the target.
NRZUBE_AGENT_DEPENDENCIES = host-rust

# The sources live in the repository, next to the Buildroot tree.
# Local source: SITE and SOURCE must be empty.
NRZUBE_AGENT_SITE =
NRZUBE_AGENT_SOURCE =
NRZUBE_AGENT_OVERRIDE_SRCDIR = $(BR2_EXTERNAL_NRZUBE_PATH)/../agent
NRZUBE_AGENT_OVERRIDE_SRCDIR_RSYNC_EXCLUSIONS = /target /.git

# The build is not done offline: the crates are downloaded into the Buildroot
# cargo home. The versions are pinned by Cargo.lock (--locked).
define NRZUBE_AGENT_BUILD_CMDS
	cd $(@D) && \
	$(TARGET_MAKE_ENV) \
		$(PKG_CARGO_ENV) \
		cargo build \
			--release \
			--manifest-path Cargo.toml \
			--locked
endef

define NRZUBE_AGENT_INSTALL_TARGET_CMDS
	$(INSTALL) -D -m 0755 \
		$(@D)/target/$(RUSTC_TARGET_NAME)/release/nrzube-agent \
		$(TARGET_DIR)/usr/bin/nrzube-agent
endef

$(eval $(generic-package))
