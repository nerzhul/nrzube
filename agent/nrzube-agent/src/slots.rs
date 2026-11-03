//! Slot discovery and slot state management.
//!
//! The slots are identified by their *content*: each rootfs carries
//! `/nrzube-slot-id` ("A" or "B") at its root. The initramfs does the same,
//! so the identification does not depend on the partition order.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::util;

/// The name of a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    A,
    B,
}

impl Slot {
    /// Parse a slot name.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "A" | "a" => Some(Slot::A),
            "B" | "b" => Some(Slot::B),
            _ => None,
        }
    }

    /// The name of the slot.
    pub fn as_str(&self) -> &'static str {
        match self {
            Slot::A => "A",
            Slot::B => "B",
        }
    }

    /// The other slot.
    pub fn other(&self) -> Slot {
        match self {
            Slot::A => Slot::B,
            Slot::B => Slot::A,
        }
    }
}

impl std::fmt::Display for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The marker file carried by every rootfs.
pub const SLOT_ID_FILE: &str = "nrzube-slot-id";

/// The block devices that may hold the partitions of the node.
pub fn partition_devices() -> Vec<PathBuf> {
    let mut devices = Vec::new();
    let entries = match std::fs::read_dir("/dev") {
        Ok(entries) => entries,
        Err(_) => return devices,
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let is_partition = (name.starts_with("mmcblk") && name.contains('p'))
            || (name.starts_with("nvme") && name.contains('p'))
            || ((name.starts_with("sd") || name.starts_with("vd")) && name.len() > 2)
            || name.starts_with("mmcblk")
            || name.starts_with("loop");
        if !is_partition {
            continue;
        }
        let path = PathBuf::from("/dev").join(&name);
        // Only keep the block devices: /dev/mmcblk0p1 and friends.
        if let Ok(metadata) = std::fs::metadata(&path) {
            use std::os::unix::fs::FileTypeExt;
            if metadata.file_type().is_block_device() {
                devices.push(path);
            }
        }
    }
    devices.sort();
    devices
}

/// The partitions of the node, identified by their content.
#[derive(Debug, Default)]
pub struct SlotMap {
    /// Partition holding the rootfs of the slot A.
    pub a: Option<PathBuf>,
    /// Partition holding the rootfs of the slot B.
    pub b: Option<PathBuf>,
    /// FAT partition holding the bootloader, the kernel and the initramfs.
    pub boot: Option<PathBuf>,
}

impl SlotMap {
    /// The partition of a slot.
    pub fn device(&self, slot: Slot) -> Option<&PathBuf> {
        match slot {
            Slot::A => self.a.as_ref(),
            Slot::B => self.b.as_ref(),
        }
    }
}

/// A temporary mount point.
struct MountGuard {
    path: PathBuf,
}

impl MountGuard {
    fn new(path: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&path);
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for MountGuard {
    fn drop(&mut self) {
        let _ = util::umount(&self.path);
    }
}

/// Identify the partitions of the node by mounting them read-only and looking
/// at their content.
pub fn discover() -> Result<SlotMap> {
    let mut map = SlotMap::default();
    let probe_root = std::env::temp_dir().join(format!("nrzube-probe.{}", std::process::id()));
    let _ = std::fs::create_dir_all(&probe_root);

    for device in partition_devices() {
        // The state partition is already mounted on /var: mounting it again
        // would fail, which is harmless.
        let guard = MountGuard::new(probe_root.join(
            device.file_name().unwrap_or_default().to_string_lossy().to_string(),
        ));

        if util::mount(&device, guard.path(), "squashfs", util::MS_RDONLY, None).is_ok() {
            let marker = guard.path().join(SLOT_ID_FILE);
            if let Some(slot) = util::read_trimmed(&marker).and_then(|v| Slot::parse(&v)) {
                match slot {
                    Slot::A => map.a = Some(device.clone()),
                    Slot::B => map.b = Some(device.clone()),
                }
            }
            continue;
        }

        if util::mount(&device, guard.path(), "vfat", util::MS_RDONLY, None).is_ok() {
            // The boot partition holds the kernel and the bootloader.
            if guard.path().join("Image").exists() && guard.path().join("config.txt").exists() {
                map.boot = Some(device.clone());
            }
        }
    }

    let _ = std::fs::remove_dir_all(&probe_root);
    Ok(map)
}

/// The slot the node is currently running from.
pub fn current_slot() -> Option<Slot> {
    util::read_trimmed(Path::new("/nrzube-slot-id")).and_then(|v| Slot::parse(&v))
}

/// The slot the initramfs selected.
pub fn active_slot(paths: &crate::config::Paths) -> Slot {
    util::read_trimmed(&paths.active_slot)
        .and_then(|v| Slot::parse(&v))
        .unwrap_or(Slot::A)
}

/// The slot waiting for its health confirmation.
pub fn pending_slot(paths: &crate::config::Paths) -> Option<Slot> {
    util::read_trimmed(&paths.pending_slot).and_then(|v| Slot::parse(&v))
}

/// Write the active slot atomically.
pub fn set_active_slot(paths: &crate::config::Paths, slot: Slot) -> Result<()> {
    util::write_atomic_str(&paths.active_slot, slot.as_str())
        .with_context(|| format!("writing {}", paths.active_slot.display()))
}

/// Write the pending slot atomically.
pub fn set_pending_slot(paths: &crate::config::Paths, slot: Slot) -> Result<()> {
    util::write_atomic_str(&paths.pending_slot, slot.as_str())
        .with_context(|| format!("writing {}", paths.pending_slot.display()))
}

/// Remove the pending slot marker.
pub fn clear_pending_slot(paths: &crate::config::Paths) {
    let _ = std::fs::remove_file(&paths.pending_slot);
}

/// Mark a slot as confirmed healthy.
pub fn set_boot_ok(paths: &crate::config::Paths, slot: Slot) -> Result<()> {
    util::write_atomic_str(&paths.boot_ok, slot.as_str())
        .with_context(|| format!("writing {}", paths.boot_ok.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_slot_names() {
        assert_eq!(Slot::parse("A"), Some(Slot::A));
        assert_eq!(Slot::parse("b"), Some(Slot::B));
        assert_eq!(Slot::parse("C"), None);
        assert_eq!(Slot::parse(""), None);
    }

    #[test]
    fn computes_the_other_slot() {
        assert_eq!(Slot::A.other(), Slot::B);
        assert_eq!(Slot::B.other(), Slot::A);
    }
}
