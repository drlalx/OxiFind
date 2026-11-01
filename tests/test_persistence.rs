use oxifind::{
    persistence::{
        AppSettings, PersistenceError, load_settings, load_snapshot, save_settings, save_snapshot,
    },
    search::{EntryMetadata, JournalCheckpoint, SearchEntry, SearchIndex},
};
use std::{fs, path::PathBuf};

fn sample_checkpoint() -> JournalCheckpoint {
    JournalCheckpoint {
        volume_identity: "VOL_C".to_owned(),
        journal_id: 123456789,
        next_usn: 987654321,
    }
}

fn sample_entries() -> Vec<SearchEntry> {
    vec![
        SearchEntry {
            display: "alpha.txt".to_owned(),
            path: r"C:\data\alpha.txt".to_owned(),
            path_resolved: true,
            metadata: EntryMetadata {
                file_id: Some(101),
                parent_id: Some(100),
                name: "alpha.txt".to_owned(),
                full_path: PathBuf::from(r"C:\data\alpha.txt"),
                volume_identity: Some("VOL_C".to_owned()),
                attributes: 0x20, // Archive
            },
        },
        SearchEntry {
            display: "beta.pdf".to_owned(),
            path: r"C:\data\docs\beta.pdf".to_owned(),
            path_resolved: true,
            metadata: EntryMetadata {
                file_id: Some(102),
                parent_id: Some(105),
                name: "beta.pdf".to_owned(),
                full_path: PathBuf::from(r"C:\data\docs\beta.pdf"),
                volume_identity: Some("VOL_C".to_owned()),
                attributes: 0x20,
            },
        },
    ]
}

#[test]
fn test_atomic_save_and_load_snapshot_roundtrip() {
    let temp_dir = std::env::temp_dir().join(format!("oxifind-pers-test-{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let snapshot_file = temp_dir.join("index.snapshot");

    let checkpoints = vec![sample_checkpoint()];
    let entries = sample_entries();

    let save_res = save_snapshot(&snapshot_file, &checkpoints, &entries);
    assert!(save_res.is_ok(), "Saving snapshot must succeed");

    let load_res = load_snapshot(&snapshot_file);
    assert!(load_res.is_ok(), "Loading snapshot must succeed");

    let loaded = load_res.unwrap();
    assert!(loaded.is_some(), "Snapshot must be present");
    let snapshot = loaded.unwrap();

    assert_eq!(snapshot.checkpoints, checkpoints);
    assert_eq!(snapshot.entries, entries);

    // Verify index search on restored entries
    let index = SearchIndex::from_persisted(snapshot.entries, snapshot.checkpoints);
    let results = index.search_results("alpha", 10);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].display, "alpha.txt");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_load_non_existent_snapshot_returns_none() {
    let temp_dir = std::env::temp_dir().join(format!("oxifind-pers-none-{}", std::process::id()));
    let missing_file = temp_dir.join("missing.snapshot");

    let res = load_snapshot(&missing_file);
    assert!(res.is_ok(), "Missing snapshot should return Ok(None)");
    assert!(res.unwrap().is_none());
}

#[test]
fn test_version_mismatch_detected_safely() {
    let temp_dir = std::env::temp_dir().join(format!("oxifind-pers-ver-{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let snapshot_file = temp_dir.join("version_mismatch.snapshot");

    // Save normal snapshot
    save_snapshot(&snapshot_file, &[sample_checkpoint()], &sample_entries()).unwrap();

    // Tamper version in header (offset 8..12 contains u32 version)
    let mut bytes = fs::read(&snapshot_file).unwrap();
    assert!(bytes.len() > 16);
    bytes[8] = 99; // Set version to 99
    fs::write(&snapshot_file, bytes).unwrap();

    let res = load_snapshot(&snapshot_file);
    match res {
        Err(PersistenceError::VersionMismatch { expected, found }) => {
            assert_eq!(expected, 1);
            assert_eq!(found, 99);
        }
        other => panic!("Expected VersionMismatch, got {:?}", other),
    }

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_corrupt_data_detected_safely() {
    let temp_dir =
        std::env::temp_dir().join(format!("oxifind-pers-corrupt-{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let snapshot_file = temp_dir.join("corrupt.snapshot");

    save_snapshot(&snapshot_file, &[sample_checkpoint()], &sample_entries()).unwrap();

    // Corrupt payload bytes near the end
    let mut bytes = fs::read(&snapshot_file).unwrap();
    let len = bytes.len();
    bytes[len - 5] ^= 0xFF; // Flip bits
    fs::write(&snapshot_file, bytes).unwrap();

    let res = load_snapshot(&snapshot_file);
    match res {
        Err(PersistenceError::ChecksumMismatch { .. }) | Err(PersistenceError::CorruptData(_)) => {
            // Success: safely detected without crashing
        }
        other => panic!("Expected ChecksumMismatch or CorruptData, got {:?}", other),
    }

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_settings_save_and_load_roundtrip() {
    let temp_dir =
        std::env::temp_dir().join(format!("oxifind-pers-settings-{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let settings_file = temp_dir.join("settings.json");

    // Default when file does not exist
    let default_settings = load_settings(&settings_file).unwrap();
    assert_eq!(default_settings.max_results, 1000);
    assert!(!default_settings.match_case);

    // Save custom settings
    let custom = AppSettings {
        indexed_paths: vec![PathBuf::from(r"C:\CustomPath")],
        excluded_patterns: vec!["target".to_owned(), "cache".to_owned()],
        max_results: 500,
        match_case: true,
        match_path: true,
        match_regex: false,
    };

    save_settings(&settings_file, &custom).unwrap();

    let reloaded = load_settings(&settings_file).unwrap();
    assert_eq!(reloaded, custom);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_worker_cold_start_and_recovery_workflow() {
    use oxifind::engine::{
        IndexScope, WorkerCommand, WorkerResponse, spawn_search_worker_with_snapshot,
    };
    use std::sync::mpsc;

    let temp_dir = std::env::temp_dir().join(format!("oxifind-cold-test-{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let sample_file = temp_dir.join("first_run.txt");
    fs::write(&sample_file, "indexed content").unwrap();

    let snapshot_file = temp_dir.join("index.snapshot");
    assert!(!snapshot_file.exists());

    // Run 1: First run without snapshot
    let (cmd_tx1, cmd_rx1) = mpsc::channel();
    let (resp_tx1, resp_rx1) = mpsc::channel();

    spawn_search_worker_with_snapshot(
        IndexScope::Root(temp_dir.clone()),
        cmd_tx1,
        cmd_rx1,
        resp_tx1,
        Some(snapshot_file.clone()),
    );

    let mut ready1 = false;
    while let Ok(resp) = resp_rx1.recv() {
        if let WorkerResponse::IndexReady { indexed_items, .. } = resp {
            assert!(indexed_items >= 1);
            ready1 = true;
            break;
        }
    }
    assert!(ready1);
    assert!(
        snapshot_file.exists(),
        "Snapshot must be persisted after first run"
    );

    // Run 2: Restart from persisted snapshot
    let (cmd_tx2, cmd_rx2) = mpsc::channel();
    let (resp_tx2, resp_rx2) = mpsc::channel();

    spawn_search_worker_with_snapshot(
        IndexScope::Root(temp_dir.clone()),
        cmd_tx2,
        cmd_rx2,
        resp_tx2,
        Some(snapshot_file.clone()),
    );

    let mut ready2 = false;
    while let Ok(resp) = resp_rx2.recv() {
        if let WorkerResponse::IndexReady { indexed_items, .. } = resp {
            assert!(indexed_items >= 1);
            ready2 = true;
            break;
        }
    }
    assert!(ready2);

    // Run 3: Corrupt snapshot and verify safe fallback
    let mut bytes = fs::read(&snapshot_file).unwrap();
    let len = bytes.len();
    bytes[len - 2] ^= 0xFF; // Corrupt payload
    fs::write(&snapshot_file, bytes).unwrap();

    let (cmd_tx3, cmd_rx3) = mpsc::channel();
    let (resp_tx3, resp_rx3) = mpsc::channel();

    spawn_search_worker_with_snapshot(
        IndexScope::Root(temp_dir.clone()),
        cmd_tx3,
        cmd_rx3,
        resp_tx3,
        Some(snapshot_file.clone()),
    );

    let mut ready3 = false;
    while let Ok(resp) = resp_rx3.recv() {
        if let WorkerResponse::IndexReady { indexed_items, .. } = resp {
            assert!(indexed_items >= 1);
            ready3 = true;
            break;
        }
    }
    assert!(ready3, "Worker must recover safely from corrupt snapshot");

    let _ = fs::remove_dir_all(&temp_dir);
}
