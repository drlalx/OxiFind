//! Mount point detection and system path exclusions for multiplatform support.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MountEntry {
    pub spec: String,
    pub mount_point: PathBuf,
    pub fs_type: String,
}

/// Unescapes octal escape sequences used by Linux `/proc/mounts` (e.g. `\040` for space).
pub fn unescape_octal(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let mut octal_digits = String::new();
            for _ in 0..3 {
                if let Some(&next) = chars.peek() {
                    if next.is_ascii_digit() && ('0'..='7').contains(&next) {
                        octal_digits.push(next);
                        chars.next();
                    } else {
                        break;
                    }
                }
            }
            if let (3, Ok(byte_val)) = (octal_digits.len(), u8::from_str_radix(&octal_digits, 8)) {
                result.push(byte_val as char);
                continue;
            }
            result.push('\\');
            result.push_str(&octal_digits);
        } else {
            result.push(ch);
        }
    }

    result
}

/// Parses mount table entries from `/proc/mounts` or `/etc/mtab` content.
pub fn parse_mount_entries(content: &str) -> Vec<MountEntry> {
    content
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return None;
            }
            let mut parts = trimmed.split_whitespace();
            let spec = parts.next()?;
            let mount_path_raw = parts.next()?;
            let fs_type = parts.next()?;

            let unescaped_path = unescape_octal(mount_path_raw);
            Some(MountEntry {
                spec: spec.to_owned(),
                mount_point: PathBuf::from(unescaped_path),
                fs_type: fs_type.to_owned(),
            })
        })
        .collect()
}

/// Determines whether a filesystem type and device specifier represent a local storage device.
pub fn is_local_filesystem(spec: &str, fs_type: &str) -> bool {
    let pseudo_fs = matches!(
        fs_type,
        "proc"
            | "sysfs"
            | "devtmpfs"
            | "devpts"
            | "tmpfs"
            | "cgroup"
            | "cgroup2"
            | "pstore"
            | "efivarfs"
            | "bpf"
            | "autofs"
            | "mqueue"
            | "debugfs"
            | "hugetlbfs"
            | "tracefs"
            | "fusectl"
            | "configfs"
            | "ramfs"
            | "squashfs"
            | "overlay"
            | "securityfs"
            | "nsfs"
            | "binfmt_misc"
    );
    if pseudo_fs {
        return false;
    }

    // Network and virtual FUSE file systems
    if fs_type.starts_with("fuse.")
        || matches!(fs_type, "nfs" | "nfs4" | "cifs" | "smbfs" | "davfs")
    {
        return false;
    }

    // Real disk devices on Linux start with /dev/
    spec.starts_with("/dev/")
}

/// Discovers mounted local filesystems from `/proc/mounts` on Linux.
#[cfg(target_os = "linux")]
pub fn discover_linux_mounts() -> Vec<PathBuf> {
    std::fs::read_to_string("/proc/mounts").map_or_else(
        |_| vec![PathBuf::from("/")],
        |content| {
            let entries = parse_mount_entries(&content);
            let mut mounts: Vec<PathBuf> = entries
                .into_iter()
                .filter(|entry| is_local_filesystem(&entry.spec, &entry.fs_type))
                .map(|entry| entry.mount_point)
                .collect();

            if mounts.is_empty() {
                mounts.push(PathBuf::from("/"));
            }
            mounts.sort();
            mounts.dedup();
            mounts
        },
    )
}

/// Returns the default system exclusions for file crawling on the host platform.
pub fn default_system_exclusions() -> Vec<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        vec![
            PathBuf::from("/proc"),
            PathBuf::from("/sys"),
            PathBuf::from("/dev"),
            PathBuf::from("/run"),
            PathBuf::from("/tmp"),
            PathBuf::from("/var/tmp"),
            PathBuf::from("/snap"),
        ]
    }
    #[cfg(target_os = "macos")]
    {
        vec![
            PathBuf::from("/System/Volumes/Data"),
            PathBuf::from("/.Spotlight-V100"),
            PathBuf::from("/.fseventsd"),
            PathBuf::from("/.DocumentRevisions-V100"),
            PathBuf::from("/.PKInstallSandboxManager"),
            PathBuf::from("/.Trashes"),
        ]
    }
    #[cfg(windows)]
    {
        vec![
            PathBuf::from(r"$Recycle.Bin"),
            PathBuf::from(r"System Volume Information"),
        ]
    }
    #[cfg(all(not(target_os = "linux"), not(target_os = "macos"), not(windows)))]
    {
        Vec::new()
    }
}

/// Returns the default root paths to index when no specific path is given.
pub fn default_local_roots() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        crate::ntfs::local_fixed_volumes()
    }
    #[cfg(target_os = "linux")]
    {
        vec![PathBuf::from("/")]
    }
    #[cfg(target_os = "macos")]
    {
        vec![PathBuf::from("/")]
    }
    #[cfg(all(not(windows), not(target_os = "linux"), not(target_os = "macos")))]
    {
        vec![PathBuf::from("/")]
    }
}

/// Checks whether a given path starts with any of the excluded path prefixes.
pub fn is_path_excluded(path: &Path, exclusions: &[PathBuf]) -> bool {
    exclusions
        .iter()
        .any(|exclusion| path.starts_with(exclusion))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{
        default_local_roots, default_system_exclusions, is_local_filesystem, is_path_excluded,
        parse_mount_entries, unescape_octal,
    };

    #[test]
    fn unescapes_octal_spaces_and_special_characters() {
        assert_eq!(unescape_octal(r"My\040Files"), "My Files");
        assert_eq!(
            unescape_octal(r"/media/user/USB\040Drive\0401"),
            "/media/user/USB Drive 1"
        );
        assert_eq!(unescape_octal("normal_path"), "normal_path");
    }

    #[test]
    fn parses_linux_mount_entries_correctly() {
        let sample = r#"
sysfs /sys sysfs rw,nosuid,nodev,noexec 0 0
proc /proc proc rw,nosuid,nodev,noexec 0 0
/dev/nvme0n1p2 / ext4 rw,relatime 0 0
/dev/sda1 /mnt/data\040disk ext4 rw,relatime 0 0
tmpfs /run tmpfs rw,nosuid,nodev 0 0
fusectl /sys/fs/fuse/connections fusectl rw 0 0
"#;
        let entries = parse_mount_entries(sample);
        assert_eq!(entries.len(), 6);
        assert_eq!(entries[2].spec, "/dev/nvme0n1p2");
        assert_eq!(entries[2].mount_point, PathBuf::from("/"));
        assert_eq!(entries[2].fs_type, "ext4");

        assert_eq!(entries[3].spec, "/dev/sda1");
        assert_eq!(entries[3].mount_point, PathBuf::from("/mnt/data disk"));
    }

    #[test]
    fn distinguishes_local_filesystems_from_pseudo_and_network_fs() {
        assert!(is_local_filesystem("/dev/nvme0n1p2", "ext4"));
        assert!(is_local_filesystem("/dev/sda1", "btrfs"));
        assert!(is_local_filesystem("/dev/mapper/root", "xfs"));
        assert!(is_local_filesystem("/dev/sdb1", "vfat"));

        assert!(!is_local_filesystem("proc", "proc"));
        assert!(!is_local_filesystem("sysfs", "sysfs"));
        assert!(!is_local_filesystem("tmpfs", "tmpfs"));
        assert!(!is_local_filesystem("cgroup2", "cgroup2"));
        assert!(!is_local_filesystem("/dev/loop0", "squashfs"));
        assert!(!is_local_filesystem("192.168.1.100:/share", "nfs"));
        assert!(!is_local_filesystem("//server/share", "cifs"));
    }

    #[test]
    fn checks_path_exclusions_accurately() {
        let exclusions = vec![PathBuf::from("/proc"), PathBuf::from("/sys")];

        assert!(is_path_excluded(Path::new("/proc/cpuinfo"), &exclusions));
        assert!(is_path_excluded(Path::new("/sys/devices"), &exclusions));
        assert!(!is_path_excluded(
            Path::new("/home/user/document"),
            &exclusions
        ));
        assert!(!is_path_excluded(Path::new("/usr/bin/ls"), &exclusions));
    }

    #[test]
    fn provides_valid_default_exclusions_and_roots() {
        let exclusions = default_system_exclusions();
        #[cfg(target_os = "linux")]
        {
            assert!(exclusions.contains(&PathBuf::from("/proc")));
            assert!(exclusions.contains(&PathBuf::from("/sys")));
        }
        #[cfg(windows)]
        {
            assert!(exclusions.contains(&PathBuf::from(r"$Recycle.Bin")));
        }

        let roots = default_local_roots();
        assert!(!roots.is_empty());
    }
}
