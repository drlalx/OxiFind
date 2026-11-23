use crate::search::{JournalCheckpoint, SearchEntry};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

pub const SNAPSHOT_MAGIC: &[u8; 8] = b"OXIFIND1";
pub const CURRENT_VERSION: u32 = 1;
pub const HEADER_SIZE: usize = 24;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PersistedSnapshot {
    pub version: u32,
    pub timestamp_secs: u64,
    pub checkpoints: Vec<JournalCheckpoint>,
    pub entries: Vec<SearchEntry>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AppSettings {
    pub indexed_paths: Vec<PathBuf>,
    pub excluded_patterns: Vec<String>,
    pub max_results: usize,
    pub match_case: bool,
    pub match_path: bool,
    pub match_regex: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            indexed_paths: Vec::new(),
            excluded_patterns: vec![
                "target".to_owned(),
                ".git".to_owned(),
                "node_modules".to_owned(),
                ".cache".to_owned(),
                ".stversions".to_owned(),
                "__pycache__".to_owned(),
                ".venv".to_owned(),
            ],
            max_results: 1000,
            match_case: false,
            match_path: false,
            match_regex: false,
        }
    }
}

#[derive(Debug)]
pub enum PersistenceError {
    Io(io::Error),
    Serialization(String),
    InvalidMagic,
    VersionMismatch { expected: u32, found: u32 },
    ChecksumMismatch { expected: u32, found: u32 },
    CorruptData(String),
}

impl std::fmt::Display for PersistenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Serialization(e) => write!(f, "Serialization error: {e}"),
            Self::InvalidMagic => write!(f, "Invalid snapshot magic header"),
            Self::VersionMismatch { expected, found } => {
                write!(
                    f,
                    "Snapshot version mismatch: expected {expected}, found {found}"
                )
            }
            Self::ChecksumMismatch { expected, found } => {
                write!(
                    f,
                    "Snapshot CRC32 checksum mismatch: expected {expected:#010x}, found {found:#010x}"
                )
            }
            Self::CorruptData(e) => write!(f, "Corrupt snapshot data: {e}"),
        }
    }
}

impl std::error::Error for PersistenceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for PersistenceError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut crc = i as u32;
        let mut bit = 0;
        while bit < 8 {
            let mask = (crc & 1).wrapping_neg() & 0xEDB8_8320;
            crc = (crc >> 1) ^ mask;
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

/// Computes the standard IEEE 802.3 CRC32 checksum over the provided data.
pub fn compute_crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &byte in data {
        let idx = ((crc ^ byte as u32) & 0xFF) as usize;
        crc = (crc >> 8) ^ CRC32_TABLE[idx];
    }
    !crc
}

/// Atomically persists a search snapshot and its journal checkpoints to disk.
pub fn save_snapshot(
    path: &Path,
    checkpoints: &[JournalCheckpoint],
    entries: &[SearchEntry],
) -> Result<(), PersistenceError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let timestamp_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());

    let snapshot = PersistedSnapshot {
        version: CURRENT_VERSION,
        timestamp_secs,
        checkpoints: checkpoints.to_vec(),
        entries: entries.to_vec(),
    };

    let payload = serde_json::to_vec(&snapshot)
        .map_err(|e| PersistenceError::Serialization(e.to_string()))?;

    let crc = compute_crc32(&payload);

    let mut header = Vec::with_capacity(HEADER_SIZE);
    header.extend_from_slice(SNAPSHOT_MAGIC);
    header.extend_from_slice(&CURRENT_VERSION.to_le_bytes());
    header.extend_from_slice(&crc.to_le_bytes());
    header.extend_from_slice(&(payload.len() as u64).to_le_bytes());

    let pid = std::process::id();
    let tmp_path = path.with_extension(format!("tmp.{pid}.{timestamp_secs}"));

    let write_res = (|| -> Result<(), io::Error> {
        let mut file = fs::File::create(&tmp_path)?;
        file.write_all(&header)?;
        file.write_all(&payload)?;
        file.sync_all()?;
        Ok(())
    })();

    if let Err(e) = write_res {
        let _ = fs::remove_file(&tmp_path);
        return Err(PersistenceError::Io(e));
    }

    if let Err(e) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(PersistenceError::Io(e));
    }

    Ok(())
}

/// Loads and validates an atomically persisted snapshot from disk.
///
/// Returns `Ok(None)` if the snapshot file does not exist.
pub fn load_snapshot(path: &Path) -> Result<Option<PersistedSnapshot>, PersistenceError> {
    if !path.exists() {
        return Ok(None);
    }

    let bytes = fs::read(path)?;
    if bytes.len() < HEADER_SIZE {
        return Err(PersistenceError::CorruptData(
            "Snapshot file is smaller than header size".to_owned(),
        ));
    }

    let magic = &bytes[0..8];
    if magic != SNAPSHOT_MAGIC {
        return Err(PersistenceError::InvalidMagic);
    }

    let version = u32::from_le_bytes(
        bytes[8..12]
            .try_into()
            .map_err(|_| PersistenceError::CorruptData("Failed to read version".to_owned()))?,
    );
    if version != CURRENT_VERSION {
        return Err(PersistenceError::VersionMismatch {
            expected: CURRENT_VERSION,
            found: version,
        });
    }

    let expected_crc = u32::from_le_bytes(
        bytes[12..16]
            .try_into()
            .map_err(|_| PersistenceError::CorruptData("Failed to read crc".to_owned()))?,
    );

    let payload_len = u64::from_le_bytes(
        bytes[16..24]
            .try_into()
            .map_err(|_| PersistenceError::CorruptData("Failed to read length".to_owned()))?,
    ) as usize;

    let payload = &bytes[24..];
    if payload.len() != payload_len {
        return Err(PersistenceError::CorruptData(format!(
            "Payload length mismatch: expected {payload_len}, found {}",
            payload.len()
        )));
    }

    let actual_crc = compute_crc32(payload);
    if actual_crc != expected_crc {
        return Err(PersistenceError::ChecksumMismatch {
            expected: expected_crc,
            found: actual_crc,
        });
    }

    let snapshot: PersistedSnapshot = serde_json::from_slice(payload)
        .map_err(|e| PersistenceError::CorruptData(e.to_string()))?;

    Ok(Some(snapshot))
}

/// Atomically saves application settings to disk.
pub fn save_settings(path: &Path, settings: &AppSettings) -> Result<(), PersistenceError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let payload = serde_json::to_vec_pretty(settings)
        .map_err(|e| PersistenceError::Serialization(e.to_string()))?;

    let pid = std::process::id();
    let tmp_path = path.with_extension(format!("tmp.{pid}"));

    let write_res = (|| -> Result<(), io::Error> {
        let mut file = fs::File::create(&tmp_path)?;
        file.write_all(&payload)?;
        file.sync_all()?;
        Ok(())
    })();

    if let Err(e) = write_res {
        let _ = fs::remove_file(&tmp_path);
        return Err(PersistenceError::Io(e));
    }

    if let Err(e) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(PersistenceError::Io(e));
    }

    Ok(())
}

/// Loads application settings from disk, falling back to defaults if not found.
pub fn load_settings(path: &Path) -> Result<AppSettings, PersistenceError> {
    if !path.exists() {
        return Ok(AppSettings::default());
    }

    let bytes = fs::read(path)?;
    let settings: AppSettings =
        serde_json::from_slice(&bytes).map_err(|e| PersistenceError::CorruptData(e.to_string()))?;

    Ok(settings)
}

/// Returns the standard platform-specific data directory for OxiFind.
pub fn default_data_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata).join("OxiFind");
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("OxiFind");
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Ok(data_home) = std::env::var("XDG_DATA_HOME") {
            return PathBuf::from(data_home).join("oxifind");
        }
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("oxifind");
        }
    }
    std::env::temp_dir().join("oxifind")
}

/// Returns the default path for the index snapshot file.
pub fn default_snapshot_path() -> PathBuf {
    default_data_dir().join("index.snapshot")
}

/// Returns the default path for the application settings file.
pub fn default_settings_path() -> PathBuf {
    default_data_dir().join("settings.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crc32_standard_vector() {
        // Standard check value for "123456789"
        assert_eq!(compute_crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(compute_crc32(b""), 0x0000_0000);
    }

    #[test]
    fn default_settings_include_common_cache_and_sync_exclusions() {
        let defaults = AppSettings::default();
        for pattern in [
            "target",
            ".git",
            "node_modules",
            ".cache",
            ".stversions",
            "__pycache__",
            ".venv",
        ] {
            assert!(
                defaults.excluded_patterns.iter().any(|p| p == pattern),
                "missing default excluded pattern: {pattern}"
            );
        }
    }
}
