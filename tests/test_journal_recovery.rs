#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use oxifind::{
    ntfs::{
        JournalCheckpointReader, JournalState, MftEnumerationCursor, MftRecordEnumerator,
        NtfsMftSource, NtfsMultiVolumeSource, VolumeInspector,
    },
    persistence::{load_snapshot, save_snapshot},
    search::{IndexSource, IndexUpdate, SearchIndex, UpdateBatch},
};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    fs, io,
    path::Path,
    rc::Rc,
};

const HEADER_LEN: usize = 60;
const DIRECTORY_ATTRIBUTE: u32 = 0x0000_0010;
const ARCHIVE_ATTRIBUTE: u32 = 0x0000_0020;
const USN_REASON_FILE_CREATE: u32 = 0x0000_0100;
const USN_REASON_CLOSE: u32 = 0x8000_0000;

const ROOT_REFERENCE: u64 = 0x0005_0000_0000_0005;
const PROJECTS_REFERENCE: u64 = 0x0001_0000_0000_002a;
const DOC_REFERENCE: u64 = 0x0001_0000_0000_0100;

#[derive(Clone)]
struct TestRecord {
    file_reference: u64,
    parent_reference: u64,
    attributes: u32,
    reason: u32,
    name: String,
}

impl TestRecord {
    fn new(file_reference: u64, parent_reference: u64, name: &str) -> Self {
        Self {
            file_reference,
            parent_reference,
            attributes: 0,
            reason: 0,
            name: name.to_owned(),
        }
    }

    fn directory(mut self) -> Self {
        self.attributes |= DIRECTORY_ATTRIBUTE;
        self
    }

    fn file(mut self) -> Self {
        self.attributes |= ARCHIVE_ATTRIBUTE;
        self
    }

    fn with_reason(mut self, reason: u32) -> Self {
        self.reason = reason;
        self
    }

    fn encode(&self) -> Vec<u8> {
        let name_units: Vec<u16> = self.name.encode_utf16().collect();
        let name_offset = HEADER_LEN;
        let name_length = (name_units.len() * 2) as u16;
        let mut bytes = vec![0u8; name_offset + name_units.len() * 2];
        let record_length = bytes.len() as u32;

        bytes[0..4].copy_from_slice(&record_length.to_le_bytes());
        bytes[4..6].copy_from_slice(&2u16.to_le_bytes()); // major version 2
        bytes[8..16].copy_from_slice(&self.file_reference.to_le_bytes());
        bytes[16..24].copy_from_slice(&self.parent_reference.to_le_bytes());
        bytes[40..44].copy_from_slice(&self.reason.to_le_bytes());
        bytes[52..56].copy_from_slice(&self.attributes.to_le_bytes());
        bytes[56..58].copy_from_slice(&name_length.to_le_bytes());
        bytes[58..60].copy_from_slice(&(name_offset as u16).to_le_bytes());
        for (index, unit) in name_units.iter().enumerate() {
            let start = name_offset + index * 2;
            bytes[start..start + 2].copy_from_slice(&unit.to_le_bytes());
        }

        bytes
    }
}

fn build_mft_chunk(next_file_reference: u64, records: &[TestRecord]) -> Vec<u8> {
    let mut bytes = next_file_reference.to_le_bytes().to_vec();
    for record in records {
        bytes.extend_from_slice(&record.encode());
    }
    bytes
}

fn standard_volume_chunk() -> Vec<u8> {
    build_mft_chunk(
        0,
        &[
            TestRecord::new(ROOT_REFERENCE, ROOT_REFERENCE, ".").directory(),
            TestRecord::new(PROJECTS_REFERENCE, ROOT_REFERENCE, "projects").directory(),
            TestRecord::new(DOC_REFERENCE, PROJECTS_REFERENCE, "document.docx").file(),
        ],
    )
}

struct MockVolumeInspector {
    filesystem: String,
    access_result: io::Result<()>,
    access_checked: Cell<bool>,
}

impl MockVolumeInspector {
    fn new(filesystem: &str) -> Self {
        Self {
            filesystem: filesystem.to_owned(),
            access_result: Ok(()),
            access_checked: Cell::new(false),
        }
    }

    fn with_access_denied() -> Self {
        Self {
            filesystem: "NTFS".to_owned(),
            access_result: Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Access is denied (elevation required)",
            )),
            access_checked: Cell::new(false),
        }
    }
}

impl VolumeInspector for MockVolumeInspector {
    fn filesystem_name(&self, _volume_root: &Path) -> io::Result<String> {
        Ok(self.filesystem.clone())
    }

    fn check_read_access(&self, _volume_root: &Path) -> io::Result<()> {
        self.access_checked.set(true);
        match &self.access_result {
            Ok(()) => Ok(()),
            Err(e) => Err(io::Error::new(e.kind(), e.to_string())),
        }
    }
}

#[derive(Clone, Default)]
struct MockEnumerator {
    chunks: Rc<RefCell<VecDeque<Vec<u8>>>>,
    opened_count: Rc<Cell<usize>>,
}

impl MockEnumerator {
    fn with_chunks(chunks: Vec<Vec<u8>>) -> Self {
        Self {
            chunks: Rc::new(RefCell::new(VecDeque::from(chunks))),
            opened_count: Rc::new(Cell::new(0)),
        }
    }
}

impl MftRecordEnumerator for MockEnumerator {
    fn open(&self, _volume_root: &Path) -> io::Result<Box<dyn MftEnumerationCursor>> {
        self.opened_count.set(self.opened_count.get() + 1);
        Ok(Box::new(MockCursor {
            chunks: Rc::clone(&self.chunks),
        }))
    }
}

struct MockCursor {
    chunks: Rc<RefCell<VecDeque<Vec<u8>>>>,
}

impl MftEnumerationCursor for MockCursor {
    fn next_chunk(&mut self, _start_file_reference: u64, buffer: &mut [u8]) -> io::Result<usize> {
        match self.chunks.borrow_mut().pop_front() {
            Some(chunk) => {
                let len = chunk.len().min(buffer.len());
                buffer[..len].copy_from_slice(&chunk[..len]);
                Ok(len)
            }
            None => Ok(0),
        }
    }
}

#[derive(Clone)]
struct MockJournalReader {
    state: Rc<Cell<JournalState>>,
    changes: Rc<RefCell<VecDeque<Vec<u8>>>>,
}

impl MockJournalReader {
    fn new(state: JournalState) -> Self {
        Self {
            state: Rc::new(Cell::new(state)),
            changes: Rc::new(RefCell::new(VecDeque::new())),
        }
    }

    fn with_change_chunk(self, chunk: Vec<u8>) -> Self {
        self.changes.borrow_mut().push_back(chunk);
        self
    }
}

impl JournalCheckpointReader for MockJournalReader {
    fn read_state(&self, _volume_root: &Path) -> io::Result<JournalState> {
        Ok(self.state.get())
    }

    fn read_changes(
        &self,
        _volume_root: &Path,
        _journal_id: u64,
        start_usn: i64,
        buffer: &mut [u8],
    ) -> io::Result<usize> {
        match self.changes.borrow_mut().pop_front() {
            Some(chunk) => {
                let len = chunk.len().min(buffer.len());
                buffer[..len].copy_from_slice(&chunk[..len]);
                Ok(len)
            }
            None => {
                let bytes = start_usn.to_le_bytes();
                buffer[..8].copy_from_slice(&bytes);
                Ok(8)
            }
        }
    }
}

#[test]
fn test_first_run_snapshot_and_cold_start_recovery() {
    let inspector = MockVolumeInspector::new("NTFS");
    let enumerator = MockEnumerator::with_chunks(vec![standard_volume_chunk()]);
    let journal = MockJournalReader::new(JournalState {
        journal_id: 100,
        first_usn: 0,
        next_usn: 500,
    });

    let source = NtfsMftSource::with_parts_and_journal(r"C:\", inspector, enumerator, journal);
    let index = SearchIndex::from_source(&source).expect("First-run snapshot must succeed");

    assert_eq!(index.len(), 3);
    assert_eq!(index.indexing_issue_count(), 0);
    assert_eq!(index.journal_checkpoints().len(), 1);
    assert_eq!(index.journal_checkpoints()[0].journal_id, 100);
    assert_eq!(index.journal_checkpoints()[0].next_usn, 500);

    let results = index.search_results("document", 10);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].display, r"C:\projects\document.docx");

    let temp_dir = std::env::temp_dir().join(format!("oxifind-firstrun-{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let snapshot_path = temp_dir.join("index.snapshot");

    save_snapshot(&snapshot_path, index.journal_checkpoints(), index.entries())
        .expect("Persisting snapshot must succeed");

    let loaded = load_snapshot(&snapshot_path)
        .expect("Loading snapshot must succeed")
        .expect("Snapshot must exist");

    let restored_index = SearchIndex::from_persisted(loaded.entries, loaded.checkpoints);
    assert_eq!(restored_index.len(), 3);
    let restored_results = restored_index.search_results("document", 10);
    assert_eq!(restored_results.len(), 1);
    assert_eq!(restored_results[0].display, r"C:\projects\document.docx");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_continuous_journal_update_advances_checkpoint() {
    let inspector = MockVolumeInspector::new("NTFS");
    let enumerator = MockEnumerator::with_chunks(vec![standard_volume_chunk()]);

    let new_file_record = TestRecord::new(
        0x0001_0000_0000_0102,
        PROJECTS_REFERENCE,
        "spreadsheet.xlsx",
    )
    .file()
    .with_reason(USN_REASON_FILE_CREATE | USN_REASON_CLOSE);

    let next_usn_header = 650i64.to_le_bytes();
    let mut journal_chunk = next_usn_header.to_vec();
    journal_chunk.extend_from_slice(&new_file_record.encode());

    let journal = MockJournalReader::new(JournalState {
        journal_id: 100,
        first_usn: 0,
        next_usn: 650,
    })
    .with_change_chunk(journal_chunk);

    let source = NtfsMftSource::with_parts_and_journal(r"C:\", inspector, enumerator, journal);

    let mut index = SearchIndex::from_source(&source).unwrap();
    assert_eq!(index.len(), 3);

    let batch = UpdateBatch::collect(&source, index.journal_checkpoints()).unwrap();
    assert_eq!(batch.checkpoints[0].next_usn, 650);

    let applied = index.apply(batch);
    assert_eq!(applied, 1);
    assert_eq!(index.len(), 4);

    let results = index.search_results("spreadsheet", 10);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].display, r"C:\projects\spreadsheet.xlsx");
}

#[test]
fn test_journal_wrap_purged_records_triggers_full_volume_rebuild() {
    let inspector = MockVolumeInspector::new("NTFS");
    // Provide chunks for initial load AND subsequent re-enumeration on rebuild
    let enumerator =
        MockEnumerator::with_chunks(vec![standard_volume_chunk(), standard_volume_chunk()]);

    let journal = MockJournalReader::new(JournalState {
        journal_id: 100,
        first_usn: 0,
        next_usn: 100,
    });

    let source = NtfsMftSource::with_parts_and_journal(
        r"C:\",
        inspector,
        enumerator.clone(),
        journal.clone(),
    );

    let mut index = SearchIndex::from_source(&source).unwrap();
    assert_eq!(index.journal_checkpoints()[0].next_usn, 100);

    // Simulate journal wrap/purge: first_usn jumps past checkpoint
    journal.state.set(JournalState {
        journal_id: 100,
        first_usn: 500,
        next_usn: 800,
    });

    let batch = UpdateBatch::collect(&source, index.journal_checkpoints()).unwrap();

    assert_eq!(batch.updates.len(), 1);
    match &batch.updates[0] {
        IndexUpdate::ReplaceVolume {
            volume_identity,
            entries,
        } => {
            assert_eq!(volume_identity, r"C:\");
            assert_eq!(entries.len(), 3);
        }
        other => panic!("Expected ReplaceVolume, got {other:?}"),
    }

    assert_eq!(batch.issues.len(), 1);
    assert!(
        batch.issues[0].message.contains("purged"),
        "Issue must explain purged records: {}",
        batch.issues[0].message
    );
    assert_eq!(batch.checkpoints[0].next_usn, 800);

    index.apply(batch);
    assert_eq!(index.len(), 3);
    assert_eq!(index.journal_checkpoints()[0].next_usn, 800);
}

#[test]
fn test_journal_recreation_triggers_rebuild() {
    let inspector = MockVolumeInspector::new("NTFS");
    let enumerator =
        MockEnumerator::with_chunks(vec![standard_volume_chunk(), standard_volume_chunk()]);

    let journal = MockJournalReader::new(JournalState {
        journal_id: 100,
        first_usn: 0,
        next_usn: 100,
    });

    let source =
        NtfsMftSource::with_parts_and_journal(r"C:\", inspector, enumerator, journal.clone());

    let mut index = SearchIndex::from_source(&source).unwrap();
    assert_eq!(index.journal_checkpoints()[0].journal_id, 100);

    // Simulate journal recreation with a new identifier
    journal.state.set(JournalState {
        journal_id: 200,
        first_usn: 0,
        next_usn: 50,
    });

    let batch = UpdateBatch::collect(&source, index.journal_checkpoints()).unwrap();

    assert_eq!(batch.updates.len(), 1);
    assert!(matches!(
        batch.updates[0],
        IndexUpdate::ReplaceVolume { .. }
    ));
    assert_eq!(batch.issues.len(), 1);
    assert!(
        batch.issues[0].message.contains("recreated"),
        "Issue must explain recreation: {}",
        batch.issues[0].message
    );
    assert_eq!(batch.checkpoints[0].journal_id, 200);
    assert_eq!(batch.checkpoints[0].next_usn, 50);

    index.apply(batch);
    assert_eq!(index.journal_checkpoints()[0].journal_id, 200);
}

#[test]
fn test_journal_checkpoint_past_end_corruption_triggers_rebuild() {
    let inspector = MockVolumeInspector::new("NTFS");
    let enumerator =
        MockEnumerator::with_chunks(vec![standard_volume_chunk(), standard_volume_chunk()]);

    let journal = MockJournalReader::new(JournalState {
        journal_id: 100,
        first_usn: 0,
        next_usn: 50,
    });

    let source =
        NtfsMftSource::with_parts_and_journal(r"C:\", inspector, enumerator, journal.clone());

    let index = SearchIndex::from_source(&source).unwrap();

    // Checkpoint becomes corrupt/ahead of actual journal end
    journal.state.set(JournalState {
        journal_id: 100,
        first_usn: 0,
        next_usn: 30, // less than checkpoint's 50
    });

    let batch = UpdateBatch::collect(&source, index.journal_checkpoints()).unwrap();

    assert_eq!(batch.updates.len(), 1);
    assert!(matches!(
        batch.updates[0],
        IndexUpdate::ReplaceVolume { .. }
    ));
    assert!(
        batch.issues[0]
            .message
            .contains("is past the journal's next USN"),
        "Issue message: {}",
        batch.issues[0].message
    );
}

#[test]
fn test_unsupported_volume_filesystem_explicit_error() {
    let inspector = MockVolumeInspector::new("FAT32");
    let enumerator = MockEnumerator::with_chunks(vec![standard_volume_chunk()]);
    let journal = MockJournalReader::new(JournalState::default());

    let source =
        NtfsMftSource::with_parts_and_journal(r"D:\", inspector, enumerator.clone(), journal);

    let mut entries = Vec::new();
    let mut issues = Vec::new();
    let err = source
        .collect_initial(&mut entries, &mut issues)
        .unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::Unsupported);
    assert!(
        err.to_string().contains("FAT32"),
        "Error message must specify unsupported filesystem: {err}"
    );
    assert!(
        err.to_string().contains("only NTFS volumes are supported"),
        "Error message: {err}"
    );
    assert_eq!(
        enumerator.opened_count.get(),
        0,
        "Enumerator must never be opened"
    );
}

#[test]
fn test_inaccessible_volume_permission_denied() {
    let inspector = MockVolumeInspector::with_access_denied();
    let enumerator = MockEnumerator::with_chunks(vec![standard_volume_chunk()]);
    let journal = MockJournalReader::new(JournalState::default());

    let source =
        NtfsMftSource::with_parts_and_journal(r"C:\", inspector, enumerator.clone(), journal);

    let mut entries = Vec::new();
    let mut issues = Vec::new();
    let err = source
        .collect_initial(&mut entries, &mut issues)
        .unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(
        enumerator.opened_count.get(),
        0,
        "Enumerator must never be opened"
    );
}

#[test]
fn test_multi_volume_mixed_resilience() {
    let good_volume = NtfsMftSource::with_parts_and_journal(
        r"C:\",
        MockVolumeInspector::new("NTFS"),
        MockEnumerator::with_chunks(vec![standard_volume_chunk()]),
        MockJournalReader::new(JournalState {
            journal_id: 1,
            first_usn: 0,
            next_usn: 10,
        }),
    );

    let fat32_volume = NtfsMftSource::with_parts_and_journal(
        r"D:\",
        MockVolumeInspector::new("FAT32"),
        MockEnumerator::default(),
        MockJournalReader::new(JournalState::default()),
    );

    let denied_volume = NtfsMftSource::with_parts_and_journal(
        r"E:\",
        MockVolumeInspector::with_access_denied(),
        MockEnumerator::default(),
        MockJournalReader::new(JournalState::default()),
    );

    let multi_source =
        NtfsMultiVolumeSource::with_sources(vec![good_volume, fat32_volume, denied_volume]);

    let mut entries = Vec::new();
    let mut issues = Vec::new();
    let res = multi_source.collect_initial(&mut entries, &mut issues);

    assert!(
        res.is_ok(),
        "Multi volume collection should succeed if at least one volume is valid"
    );
    assert_eq!(entries.len(), 3, "Valid volume entries must be retained");
    assert_eq!(
        issues.len(),
        2,
        "Issues must be recorded for failed volumes"
    );

    assert!(issues[0].message.contains("FAT32"));
    assert!(
        issues[1].message.contains("PermissionDenied")
            || issues[1].message.contains("Access is denied")
    );
}

#[test]
fn test_multi_volume_all_failing_reports_explicit_error() {
    let fat32_volume = NtfsMftSource::with_parts_and_journal(
        r"D:\",
        MockVolumeInspector::new("FAT32"),
        MockEnumerator::default(),
        MockJournalReader::new(JournalState::default()),
    );

    let denied_volume = NtfsMftSource::with_parts_and_journal(
        r"E:\",
        MockVolumeInspector::with_access_denied(),
        MockEnumerator::default(),
        MockJournalReader::new(JournalState::default()),
    );

    let multi_source = NtfsMultiVolumeSource::with_sources(vec![fat32_volume, denied_volume]);

    let mut entries = Vec::new();
    let mut issues = Vec::new();
    let err = multi_source
        .collect_initial(&mut entries, &mut issues)
        .unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    assert!(err.to_string().contains(r"D:\"));
    assert!(err.to_string().contains(r"E:\"));
    assert_eq!(issues.len(), 2);
}
