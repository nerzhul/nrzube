//! Small filesystem and time helpers.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Write a file atomically: temporary file, fsync, rename, fsync of the
/// directory.
///
/// The rename is the atomicity guarantee the whole A/B mechanism relies on
/// (plan §6.3): a power cut either leaves the old content or the new one,
/// never a truncated file.
pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .context("the path has no parent directory")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("creating {}", parent.display()))?;

    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "nrzube".to_string());
    let tmp: PathBuf = parent.join(format!(".{file_name}.tmp.{}", std::process::id()));

    {
        use std::io::Write;
        let mut file = std::fs::File::create(&tmp)
            .with_context(|| format!("creating {}", tmp.display()))?;
        file.write_all(contents)
            .with_context(|| format!("writing {}", tmp.display()))?;
        file.sync_all()
            .with_context(|| format!("syncing {}", tmp.display()))?;
    }

    std::fs::rename(&tmp, path).with_context(|| {
        format!("renaming {} to {}", tmp.display(), path.display())
    })?;

    // Make the rename durable.
    if let Ok(dir) = std::fs::File::open(parent) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Read a file and trim the surrounding blanks.
pub fn read_trimmed(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|content| content.trim().to_string())
        .filter(|content| !content.is_empty())
}

/// Write a file atomically, with a trailing newline.
pub fn write_atomic_str(path: &Path, value: &str) -> Result<()> {
    write_atomic(path, format!("{value}\n").as_bytes())
}

/// Seconds since the UNIX epoch.
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Format a size in bytes for the logs.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Mount a filesystem.
///
/// Thin wrapper around `mount(2)`: the agent mounts the partitions to
/// identify them and to update the boot partition.
pub fn mount(
    source: &Path,
    target: &Path,
    fstype: &str,
    flags: libc::c_ulong,
    data: Option<&str>,
) -> std::io::Result<()> {
    use std::ffi::CString;

    let source = CString::new(source.as_os_str().as_encoded_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let target = CString::new(target.as_os_str().as_encoded_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let fstype = CString::new(fstype)
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let data = match data {
        Some(value) => Some(
            CString::new(value)
                .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?,
        ),
        None => None,
    };

    // SAFETY: all the pointers are valid C strings owned by this scope and
    // the call only borrows them for the duration of the syscall.
    let result = unsafe {
        libc::mount(
            source.as_ptr(),
            target.as_ptr(),
            fstype.as_ptr(),
            flags,
            data.as_ref().map_or(std::ptr::null(), |d| d.as_ptr() as *const libc::c_void),
        )
    };

    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Unmount a filesystem.
pub fn umount(target: &Path) -> std::io::Result<()> {
    use std::ffi::CString;

    let target = CString::new(target.as_os_str().as_encoded_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    // SAFETY: the pointer is a valid NUL terminated C string.
    let result = unsafe { libc::umount2(target.as_ptr(), 0) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Mount flags used for a read-only mount.
pub const MS_RDONLY: libc::c_ulong = libc::MS_RDONLY;
/// Mount flags used for a read-write mount.
pub const MS_RW: libc::c_ulong = 0;

/// Reboot the system.
pub fn reboot() -> std::io::Result<()> {
    // SAFETY: reboot(2) does not return on success.
    unsafe {
        libc::sync();
        libc::reboot(libc::LINUX_REBOOT_CMD_RESTART);
    }
    Err(std::io::Error::last_os_error())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sizes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KiB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MiB");
    }

    #[test]
    fn writes_atomically() {
        let dir = tempfile::tempdir().expect("temp dir");
        let target = dir.path().join("sub").join("value");
        write_atomic_str(&target, "hello").expect("write");
        assert_eq!(read_trimmed(&target).as_deref(), Some("hello"));

        // No leftover temporary file.
        let leftovers: Vec<_> = std::fs::read_dir(target.parent().unwrap())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "temporary files left behind");
    }
}
