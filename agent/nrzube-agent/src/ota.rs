//! The OTA workflow: download, verify, install and switch slot.
//!
//! The trust chain is:
//!   embedded cosign key -> signature of the release manifest -> SHA-256 of
//!   every component.
//!
//! Nothing is written in a slot before the signature and the digests have
//! been checked, and the switch itself is atomic (plan §6.3).

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use crate::config::Paths;
use crate::cosign::TrustedKey;
use crate::metrics::Metrics;
use crate::registry::Registry;
use crate::slots::{self, Slot};
use crate::util;

/// Media type of the signed release manifest.
pub const RELEASE_MANIFEST_MEDIA_TYPE: &str = "application/vnd.nrzube.release.v1+json";
/// Media type of the cosign bundle.
pub const BUNDLE_MEDIA_TYPE: &str = "application/vnd.dev.sigstore.bundle.v0.3+json";
/// Media type of the rootfs.
pub const ROOTFS_MEDIA_TYPE: &str = "application/vnd.nrzube.rootfs.v1";
/// Media type of the kernel.
pub const KERNEL_MEDIA_TYPE: &str = "application/vnd.nrzube.kernel.v1";
/// Media type of the initramfs.
pub const INITRD_MEDIA_TYPE: &str = "application/vnd.nrzube.initrd.v1";
/// Media type of the device tree blob.
pub const DTB_MEDIA_TYPE: &str = "application/vnd.nrzube.dtb.v1";

/// The signed release manifest.
#[derive(Debug, Clone, Deserialize)]
pub struct ReleaseManifest {
    #[serde(rename = "schemaVersion", default)]
    pub schema_version: u32,
    pub version: String,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub components: Vec<Component>,
}

/// A component of the release.
#[derive(Debug, Clone, Deserialize)]
pub struct Component {
    pub name: String,
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
    #[serde(rename = "ociDigest", default)]
    pub oci_digest: Option<String>,
}

/// What the OTA check did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing to do: the running version is the latest one.
    UpToDate,
    /// A new version has been installed and the node rebooted.
    Updated {
        from: String,
        to: String,
    },
    /// The pending slot has been confirmed.
    Confirmed,
    /// The pending slot was broken: the node rolled back.
    RolledBack,
}

/// The OTA agent.
pub struct Ota {
    paths: Paths,
    registry: Registry,
    key: TrustedKey,
    channel: String,
    metrics: Arc<Metrics>,
    /// How long the pending slot is given to become healthy.
    pub validation_timeout: Duration,
}

impl Ota {
    /// Build the OTA agent.
    pub fn new(
        paths: Paths,
        registry: Registry,
        key: TrustedKey,
        channel: String,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            paths,
            registry,
            key,
            channel,
            metrics,
            validation_timeout: Duration::from_secs(300),
        }
    }

    /// The version currently installed on the node.
    pub fn installed_version(&self) -> Option<String> {
        util::read_trimmed(&self.paths.state_dir.join("version"))
    }

    /// Look for an update and install it.
    pub async fn check_and_apply(&self) -> Result<Outcome> {
        let tag = format!("nrzube-{}", self.channel);
        log::info!("checking the registry for {tag}");

        let manifest = self.registry.manifest(&tag).await.with_context(|| {
            format!(
                "fetching the manifest of {tag} (the registry may be unreachable: this is not fatal)"
            )
        })?;

        // The signed objects: the release manifest and its cosign bundle.
        let release_descriptor = manifest
            .layer(RELEASE_MANIFEST_MEDIA_TYPE)
            .context("the image has no release manifest layer")?
            .clone();
        let bundle_descriptor = manifest
            .layer(BUNDLE_MEDIA_TYPE)
            .context("the image has no cosign bundle layer")?
            .clone();

        let release_bytes = self.registry.blob(&release_descriptor.digest).await?;
        let bundle_bytes = self.registry.blob(&bundle_descriptor.digest).await?;

        // --- Verification, before anything is written -------------------------
        self.key
            .verify_bytes(&release_bytes, &bundle_bytes)
            .context("the release manifest signature is not valid")?;
        log::info!("the release manifest signature is valid");

        let release: ReleaseManifest =
            serde_json::from_slice(&release_bytes).context("decoding the release manifest")?;
        if release.schema_version != 1 {
            bail!("unsupported release manifest schema: {}", release.schema_version);
        }

        let installed = self.installed_version().unwrap_or_default();
        if !installed.is_empty() && release.version.as_str() <= installed.as_str() {
            log::info!("the node is up to date (version {installed})");
            return Ok(Outcome::UpToDate);
        }

        // --- Download and check every component ------------------------------
        std::fs::create_dir_all(&self.paths.staging_dir)
            .with_context(|| format!("creating {}", self.paths.staging_dir.display()))?;

        let mut staged: Vec<(Component, PathBuf)> = Vec::new();
        for component in &release.components {
            let descriptor = manifest
                .layer(&component.media_type)
                .with_context(|| {
                    format!(
                        "the image has no layer for the component {} ({})",
                        component.name, component.media_type
                    )
                })?
                .clone();

            let destination = self.paths.staging_dir.join(format!(
                "{}-{}",
                component.name,
                component.sha256
            ));
            let written = self
                .registry
                .download_blob(&descriptor.digest, &destination)
                .await
                .with_context(|| format!("downloading the component {}", component.name))?;

            let digest = sha256_file(&destination)?;
            if digest != component.sha256 {
                bail!(
                    "SHA-256 mismatch for the component {}: expected {}, got {digest}",
                    component.name,
                    component.sha256
                );
            }
            if component.size > 0 && written != component.size {
                bail!(
                    "size mismatch for the component {}: expected {}, got {written}",
                    component.name,
                    component.size
                );
            }
            log::info!(
                "component {} verified ({} bytes)",
                component.name,
                util::human_size(written)
            );
            staged.push((component.clone(), destination));
        }

        // --- Install ------------------------------------------------------------
        self.install(&release, &staged)?;

        log::info!(
            "updating from {} to {}: rebooting",
            if installed.is_empty() { "<none>" } else { &installed },
            release.version
        );
        self.metrics.updates_applied.fetch_add(1, Ordering::Relaxed);
        self.metrics
            .last_update_timestamp
            .store(util::unix_now(), Ordering::Relaxed);

        // Record the new version before the reboot: the pending slot is
        // confirmed later.
        util::write_atomic_str(&self.paths.state_dir.join("version"), &release.version)?;

        self.reboot()
            .context("rebooting into the new slot")?;
        Ok(Outcome::Updated {
            from: installed,
            to: release.version,
        })
    }

    /// Install the verified components: rootfs in the inactive slot, kernel,
    /// initramfs and DTB in the boot partition, then switch the slot.
    fn install(&self, release: &ReleaseManifest, staged: &[(Component, PathBuf)]) -> Result<()> {
        let current = slots::current_slot().unwrap_or(Slot::A);
        let target = current.other();
        log::info!("installing the release {} in the slot {target}", release.version);

        let map = slots::discover()?;
        let rootfs_component = staged
            .iter()
            .find(|(component, _)| component.media_type == ROOTFS_MEDIA_TYPE)
            .context("the release has no rootfs component")?;
        let rootfs_path = &rootfs_component.1;

        // --- The rootfs goes to the inactive slot ------------------------------
        let device = map
            .device(target)
            .cloned()
            .with_context(|| format!("the partition of the slot {target} was not found"))?;

        let image_size = std::fs::metadata(rootfs_path)?.len();
        if let Some(partition_size) = device_size(&device) {
            if image_size > partition_size {
                bail!(
                    "the new rootfs ({image_size} bytes) does not fit in the slot partition ({partition_size} bytes)"
                );
            }
        }

        log::info!("writing the new rootfs to the slot {target} ({device:?})");
        write_to_device(rootfs_path, &device)?;

        // --- The kernel, the initramfs and the DTB go to the boot partition -----
        let boot_device = map
            .boot
            .clone()
            .context("the boot partition was not found")?;
        self.update_boot_partition(&boot_device, staged)?;

        // --- Switch the slot ----------------------------------------------------
        // The pending marker is written first: the initramfs uses it to know
        // that the slot must be confirmed, and it is what allows the rollback.
        slots::set_pending_slot(&self.paths, target)?;
        slots::set_active_slot(&self.paths, target)?;
        log::info!("slot switched to {target} (previous slot: {current})");
        Ok(())
    }

    /// Replace Image, initrd.img and the DTB in the boot partition.
    ///
    /// The files are written to a temporary name and renamed: the rename is
    /// atomic on FAT, so a power cut leaves either the old or the new file.
    fn update_boot_partition(
        &self,
        device: &Path,
        staged: &[(Component, PathBuf)],
    ) -> Result<()> {
        let mount_point = std::env::temp_dir().join(format!("nrzube-boot.{}", std::process::id()));
        std::fs::create_dir_all(&mount_point)?;
        util::mount(device, &mount_point, "vfat", util::MS_RW, None)
            .with_context(|| format!("mounting the boot partition {device:?}"))?;

        let result = (|| -> Result<()> {
            for (component, path) in staged {
                let name = match component.media_type.as_str() {
                    KERNEL_MEDIA_TYPE => "Image",
                    INITRD_MEDIA_TYPE => "initrd.img",
                    DTB_MEDIA_TYPE => "bcm2712-rpi-5-b.dtb",
                    _ => continue,
                };
                let destination = mount_point.join(name);
                let temporary = mount_point.join(format!(".{name}.new"));
                std::fs::copy(path, &temporary)
                    .with_context(|| format!("copying {}", path.display()))?;
                std::fs::rename(&temporary, &destination)
                    .with_context(|| format!("installing {name}"))?;
                log::info!("boot partition: {name} updated");
            }
            Ok(())
        })();

        // Sync before unmounting: the FAT driver flushes on unmount.
        let _ = util::umount(&mount_point);
        let _ = std::fs::remove_dir_all(&mount_point);
        result
    }

    /// Confirm or roll back the pending slot after a reboot.
    pub async fn validate_pending(&self) -> Result<Outcome> {
        let Some(pending) = slots::pending_slot(&self.paths) else {
            return Ok(Outcome::UpToDate);
        };
        let active = slots::active_slot(&self.paths);
        if pending != active {
            // The initramfs already rolled back: nothing to do.
            slots::clear_pending_slot(&self.paths);
            return Ok(Outcome::UpToDate);
        }

        log::info!("validating the pending slot {pending}");
        let deadline = std::time::Instant::now() + self.validation_timeout;
        let mut healthy = false;

        while std::time::Instant::now() < deadline {
            if self.metrics.containerd_healthy.load(Ordering::Relaxed)
                && self.metrics.kubelet_healthy.load(Ordering::Relaxed)
            {
                healthy = true;
                break;
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }

        if healthy {
            slots::set_boot_ok(&self.paths, pending)?;
            slots::clear_pending_slot(&self.paths);
            log::info!("the slot {pending} is confirmed");
            return Ok(Outcome::Confirmed);
        }

        // The slot is broken: go back to the previous one.
        log::error!("the slot {pending} is not healthy: rolling back to {}", pending.other());
        self.metrics.rollbacks.fetch_add(1, Ordering::Relaxed);
        slots::set_active_slot(&self.paths, pending.other())?;
        slots::clear_pending_slot(&self.paths);
        let _ = std::fs::remove_file(&self.paths.boot_ok);
        self.reboot().context("rebooting into the previous slot")?;
        Ok(Outcome::RolledBack)
    }

    fn reboot(&self) -> Result<()> {
        // Make sure everything is written before the reboot.
        let _ = std::process::Command::new("sync").status();
        util::reboot().context("rebooting")
    }
}

/// The SHA-256 of a file, hexadecimal.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Write a file to a block device.
fn write_to_device(source: &Path, device: &Path) -> Result<()> {
    let mut input = std::fs::File::open(source)
        .with_context(|| format!("opening {}", source.display()))?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .open(device)
        .with_context(|| format!("opening the device {}", device.display()))?;
    output.seek(SeekFrom::Start(0))?;

    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
    }
    output.flush()?;
    output.sync_all()?;
    Ok(())
}

/// The size of a block device, in bytes.
fn device_size(device: &Path) -> Option<u64> {
    let name = device.file_name()?.to_str()?;
    let sysfs = PathBuf::from("/sys/class/block").join(name).join("size");
    let sectors: u64 = std::fs::read_to_string(sysfs).ok()?.trim().parse().ok()?;
    Some(sectors * 512)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_release_manifest() {
        let json = r#"{
            "schemaVersion": 1,
            "version": "2026.11.02",
            "channel": "stable",
            "components": [
                { "name": "rootfs", "mediaType": "application/vnd.nrzube.rootfs.v1",
                  "sha256": "aa", "size": 42, "ociDigest": "sha256:aa" }
            ]
        }"#;
        let release: ReleaseManifest = serde_json::from_str(json).unwrap();
        assert_eq!(release.schema_version, 1);
        assert_eq!(release.version, "2026.11.02");
        assert_eq!(release.components.len(), 1);
        assert_eq!(release.components[0].media_type, ROOTFS_MEDIA_TYPE);
        assert_eq!(release.components[0].oci_digest.as_deref(), Some("sha256:aa"));
    }

    #[test]
    fn hashes_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("payload");
        std::fs::write(&path, b"nrzube").unwrap();

        let mut hasher = Sha256::new();
        hasher.update(b"nrzube");
        let expected = hex::encode(hasher.finalize());

        assert_eq!(sha256_file(&path).unwrap(), expected);
    }
}
