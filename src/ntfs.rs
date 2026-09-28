use std::{
    cell::RefCell,
    collections::HashSet,
    io,
    path::{Component, Path, PathBuf, Prefix},
};

use crate::search::{
    EntryMetadata, IndexIssue, IndexSource, IndexUpdate, JournalCheckpoint, SearchEntry,
};

/// Abstracts the platform-specific volume queries that `NtfsMftSource`
/// depends on. Splitting these behind a trait lets the validation and
/// ordering behavior in `collect_initial` be exercised deterministically in
/// unit tests, independent of any specific drive letter, filesystem, or
/// process privilege level available on the machine running the tests.
pub trait VolumeInspector {
    /// Returns the filesystem name reported for `volume_root` (for example
    /// `"NTFS"` or `"FAT32"`), read from a supported/documented Windows
    /// volume-management API.
    fn filesystem_name(&self, volume_root: &Path) -> io::Result<String>;

    /// Returns `Ok(())` when the current process can open `volume_root` for
    /// raw read access, or the underlying io error otherwise (for example
    /// `PermissionDenied` when the process lacks the elevated privilege that
    /// raw MFT access requires).
    fn check_read_access(&self, volume_root: &Path) -> io::Result<()>;
}

/// Opens a volume for master-file-table enumeration.
///
/// This is the seam around the native `FSCTL_ENUM_USN_DATA` device control:
/// the production implementation ([`WindowsMftEnumerator`]) issues the real
/// control code, while tests supply chunks of synthetic output so that
/// record parsing and continuation are covered without depending on the
/// contents, size, or accessibility of a real volume.
pub trait MftRecordEnumerator {
    /// Opens `volume_root` and returns a cursor over the raw enumeration
    /// output, or the underlying io error when the volume cannot be opened.
    fn open(&self, volume_root: &Path) -> io::Result<Box<dyn MftEnumerationCursor>>;
}

/// Cursor over the raw output chunks produced by `FSCTL_ENUM_USN_DATA`.
pub trait MftEnumerationCursor {
    /// Writes the next output chunk for `start_file_reference` into
    /// `buffer` and returns the number of bytes written, or `Ok(0)` when the
    /// volume has no further records.
    ///
    /// Each chunk begins with the 64-bit little-endian file reference number
    /// the next call should start from, followed by consecutive USN records.
    /// Implementations must never report more bytes than `buffer` can hold.
    fn next_chunk(&mut self, start_file_reference: u64, buffer: &mut [u8]) -> io::Result<usize>;
}

/// Current identity and readable USN range of a volume's change journal.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct JournalState {
    pub journal_id: u64,
    /// First USN that can still be read; older records were purged.
    pub first_usn: i64,
    pub next_usn: i64,
}

/// Reads the USN journal state that must accompany a completed volume
/// snapshot. The journal identifier distinguishes a recreated journal from
/// its predecessor, while `next_usn` is the first change not represented by
/// the checkpoint.
pub trait JournalCheckpointReader {
    fn read_state(&self, volume_root: &Path) -> io::Result<JournalState>;

    /// Writes the `FSCTL_READ_USN_JOURNAL` output starting at `start_usn` into `buffer` and returns its length:
    /// the next USN to read from, followed by version 2 USN records.
    fn read_changes(
        &self,
        volume_root: &Path,
        journal_id: u64,
        start_usn: i64,
        buffer: &mut [u8],
    ) -> io::Result<usize>;
}

/// Size of the output buffer handed to each enumeration chunk request. The
/// documented control code fills as many complete records as fit, so this
/// only trades the number of device-control round trips against memory.
const ENUMERATION_BUFFER_LEN: usize = 64 * 1024;

/// `FSCTL_ENUM_USN_DATA` prefixes every output chunk with the 64-bit file
/// reference number that the following request should start from.
const NEXT_REFERENCE_LEN: usize = 8;

/// Size of the fixed part of a `USN_RECORD_V2`, up to and including the
/// `FileNameOffset` field that precedes the variable-length name.
const USN_RECORD_V2_HEADER_LEN: usize = 60;

/// `MFT_ENUM_DATA_V0` input makes the file system report version 2 records.
const SUPPORTED_RECORD_MAJOR_VERSION: u16 = 2;

/// `USN_REASON_FILE_DELETE`. `FSCTL_ENUM_USN_DATA` reports only records that
/// are currently in use, but a record carrying this reason describes a file
/// that is already gone and must not enter the index.
const USN_REASON_FILE_DELETE: u32 = 0x0000_0200;

const USN_REASON_FILE_CREATE: u32 = 0x0000_0100;
const USN_REASON_RENAME_OLD_NAME: u32 = 0x0000_1000;
const USN_REASON_RENAME_NEW_NAME: u32 = 0x0000_2000;

/// Journal reasons that change which names exist where; content changes do not affect the index.
const JOURNAL_REASON_MASK: u32 = USN_REASON_FILE_CREATE
    | USN_REASON_FILE_DELETE
    | USN_REASON_RENAME_OLD_NAME
    | USN_REASON_RENAME_NEW_NAME;

/// A file reference number packs the MFT record index into its low 48 bits
/// and a reuse sequence number into its high 16 bits. The complete value
/// distinguishes a recently reused MFT record from the file that used the
/// same record index before it; it must therefore be retained in index
/// identifiers and parent links. The mask is only used to recognize the
/// reserved root record index.
const FILE_REFERENCE_INDEX_MASK: u64 = 0x0000_ffff_ffff_ffff;

/// NTFS reserves MFT record index 5 for the volume root directory.
const ROOT_RECORD_INDEX: u64 = 5;

/// `FILE_ATTRIBUTE_DIRECTORY`, used for the volume root anchor when the
/// enumeration itself did not report the root directory record.
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;

/// Initial index source backed by the NTFS Master File Table.
///
/// Before any MFT record is read, [`collect_initial`](IndexSource::collect_initial)
/// validates that the requested volume is NTFS-formatted and that the
/// current process can open it for raw read access, using documented
/// Windows volume APIs (see [`WindowsVolumeInspector`]) or an injected
/// [`VolumeInspector`] for deterministic tests. Records are then enumerated
/// with the documented `FSCTL_ENUM_USN_DATA` control code (see
/// [`WindowsMftEnumerator`]), which reports the volume's active records
/// without walking directories. Every record is parsed defensively: a
/// malformed length, version, or name range is reported as a recoverable
/// [`IndexIssue`] instead of ending the index or panicking.
pub struct NtfsMftSource<
    I = WindowsVolumeInspector,
    E = WindowsMftEnumerator,
    J = WindowsJournalCheckpointReader,
> {
    volume: PathBuf,
    inspector: I,
    enumerator: E,
    journal_reader: J,
}

impl NtfsMftSource<WindowsVolumeInspector, WindowsMftEnumerator, WindowsJournalCheckpointReader> {
    /// Creates a source for `volume` that validates access and enumerates
    /// records using the real Windows APIs.
    pub fn new(volume: impl Into<PathBuf>) -> Self {
        Self::with_parts(volume, WindowsVolumeInspector, WindowsMftEnumerator)
    }
}

impl<I: VolumeInspector, E: MftRecordEnumerator> NtfsMftSource<I, E> {
    /// Creates a source for `volume` that validates access using `inspector`
    /// and reads records through `enumerator`. Intended for tests that need
    /// deterministic filesystem, permission, and record outcomes without
    /// depending on a specific real volume.
    pub fn with_parts(volume: impl Into<PathBuf>, inspector: I, enumerator: E) -> Self {
        Self {
            volume: volume.into(),
            inspector,
            enumerator,
            journal_reader: WindowsJournalCheckpointReader,
        }
    }
}

impl<I, E, J> NtfsMftSource<I, E, J> {
    /// Creates a source with an injected journal reader for deterministic
    /// checkpoint tests.
    pub fn with_parts_and_journal(
        volume: impl Into<PathBuf>,
        inspector: I,
        enumerator: E,
        journal_reader: J,
    ) -> Self {
        Self {
            volume: volume.into(),
            inspector,
            enumerator,
            journal_reader,
        }
    }
}

impl<I: VolumeInspector, E: MftRecordEnumerator, J: JournalCheckpointReader> IndexSource
    for NtfsMftSource<I, E, J>
{
    fn collect_initial(
        &self,
        out: &mut Vec<SearchEntry>,
        issues: &mut Vec<IndexIssue>,
    ) -> io::Result<()> {
        let volume_root = volume_root_path(&self.volume)?;

        let filesystem = self.inspector.filesystem_name(&volume_root)?;
        if !filesystem.eq_ignore_ascii_case("NTFS") {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "Volume {} uses unsupported filesystem \"{filesystem}\"; only NTFS volumes are supported",
                    volume_root.display()
                ),
            ));
        }

        self.inspector.check_read_access(&volume_root)?;

        let mut cursor = self.enumerator.open(&volume_root)?;
        let collected = collect_records(&volume_root, cursor.as_mut(), out, issues)?;
        if collected == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Enumerating the master file table of {} returned no usable records",
                    volume_root.display()
                ),
            ));
        }

        Ok(())
    }

    fn journal_checkpoints(&self) -> io::Result<Vec<JournalCheckpoint>> {
        let volume_root = volume_root_path(&self.volume)?;
        let state = self.journal_reader.read_state(&volume_root)?;
        Ok(vec![checkpoint_at(&volume_root, state)])
    }

    fn collect_updates(
        &self,
        checkpoints: &mut [JournalCheckpoint],
        out: &mut Vec<IndexUpdate>,
        issues: &mut Vec<IndexIssue>,
    ) -> io::Result<()> {
        let volume_root = volume_root_path(&self.volume)?;
        let identity = volume_identity(&volume_root);
        for checkpoint in checkpoints
            .iter_mut()
            .filter(|checkpoint| checkpoint.volume_identity == identity)
        {
            let state = self.journal_reader.read_state(&volume_root)?;
            match stale_reason(checkpoint, &state) {
                None => collect_journal_updates(
                    &volume_root,
                    &self.journal_reader,
                    checkpoint,
                    out,
                    issues,
                )?,
                Some(reason) => {
                    // The state is read before enumerating, so changes made meanwhile are replayed later.
                    let mut entries = Vec::new();
                    self.collect_initial(&mut entries, issues)?;
                    issues.push(IndexIssue {
                        path: volume_root.clone(),
                        message: format!("Rebuilt volume {identity} because {reason}"),
                    });
                    out.push(IndexUpdate::ReplaceVolume {
                        volume_identity: identity.clone(),
                        entries,
                    });
                    *checkpoint = checkpoint_at(&volume_root, state);
                }
            }
        }
        Ok(())
    }
}

fn checkpoint_at(volume_root: &Path, state: JournalState) -> JournalCheckpoint {
    JournalCheckpoint {
        volume_identity: volume_identity(volume_root),
        journal_id: state.journal_id,
        next_usn: state.next_usn,
    }
}

/// Explains why the changes after `checkpoint` can no longer be read completely, if they cannot.
fn stale_reason(checkpoint: &JournalCheckpoint, state: &JournalState) -> Option<String> {
    if state.journal_id != checkpoint.journal_id {
        Some(format!(
            "its USN journal was recreated (identifier {} replaced {})",
            state.journal_id, checkpoint.journal_id
        ))
    } else if checkpoint.next_usn < state.first_usn {
        Some(format!(
            "USN journal records from {} up to {} were purged",
            checkpoint.next_usn, state.first_usn
        ))
    } else if checkpoint.next_usn > state.next_usn {
        Some(format!(
            "its checkpoint USN {} is past the journal's next USN {}",
            checkpoint.next_usn, state.next_usn
        ))
    } else {
        None
    }
}

/// Initial index source that validates and enumerates several NTFS volumes
/// as one snapshot.
///
/// Each selected volume is validated and enumerated independently through
/// its own [`NtfsMftSource`], reusing the same validation and native
/// enumeration seams. A volume that fails validation or enumeration does not
/// discard entries already collected from other volumes: its failure
/// becomes an [`IndexIssue`] tied to that volume's own path, and collection
/// continues with the remaining volumes in selection order. Selected paths
/// that resolve to the same drive root (for example `C:\` and `C:\Users`)
/// are deduplicated so that volume is only validated and enumerated once.
///
/// `collect_initial` only fails when no volume was selected, or when every
/// selected volume failed; whenever it returns `Ok`, `out` holds at least
/// one entry from at least one selected volume.
pub struct NtfsMultiVolumeSource<
    I = WindowsVolumeInspector,
    E = WindowsMftEnumerator,
    J = WindowsJournalCheckpointReader,
> {
    sources: Vec<NtfsMftSource<I, E, J>>,
    journal_checkpoints: RefCell<Vec<JournalCheckpoint>>,
}

impl
    NtfsMultiVolumeSource<
        WindowsVolumeInspector,
        WindowsMftEnumerator,
        WindowsJournalCheckpointReader,
    >
{
    /// Creates a source that validates and enumerates every volume in
    /// `volumes` using the real Windows APIs.
    pub fn new(volumes: impl IntoIterator<Item = impl Into<PathBuf>>) -> Self {
        Self {
            sources: volumes.into_iter().map(NtfsMftSource::new).collect(),
            journal_checkpoints: RefCell::new(Vec::new()),
        }
    }
}

impl<I, E, J> NtfsMultiVolumeSource<I, E, J> {
    /// Creates a source from already-constructed single-volume sources.
    ///
    /// Intended for tests that need deterministic, per-volume validation and
    /// enumeration outcomes (including mixed successes and failures) without
    /// depending on any real drive.
    pub fn with_sources(sources: Vec<NtfsMftSource<I, E, J>>) -> Self {
        Self {
            sources,
            journal_checkpoints: RefCell::new(Vec::new()),
        }
    }
}

impl<I: Default, E: Default, J: Default> Default for NtfsMultiVolumeSource<I, E, J> {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            journal_checkpoints: RefCell::new(Vec::new()),
        }
    }
}

impl<I: VolumeInspector, E: MftRecordEnumerator, J: JournalCheckpointReader> IndexSource
    for NtfsMultiVolumeSource<I, E, J>
{
    fn collect_initial(
        &self,
        out: &mut Vec<SearchEntry>,
        issues: &mut Vec<IndexIssue>,
    ) -> io::Result<()> {
        if self.sources.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "No NTFS volumes were selected for indexing",
            ));
        }

        let mut seen_roots: HashSet<PathBuf> = HashSet::new();
        let mut indexed_volumes = 0_usize;
        let mut failed_volumes: Vec<String> = Vec::new();
        let mut journal_checkpoints = Vec::new();

        for source in &self.sources {
            let is_duplicate_selection = match volume_root_path(&source.volume) {
                Ok(root) => !seen_roots.insert(root),
                Err(_) => false,
            };
            if is_duplicate_selection {
                continue;
            }

            let mut volume_entries: Vec<SearchEntry> = Vec::new();
            let mut volume_issues: Vec<IndexIssue> = Vec::new();

            match source.collect_initial(&mut volume_entries, &mut volume_issues) {
                Ok(()) => match source.journal_checkpoints() {
                    Ok(mut checkpoint) => {
                        out.append(&mut volume_entries);
                        issues.append(&mut volume_issues);
                        journal_checkpoints.append(&mut checkpoint);
                        indexed_volumes += 1;
                    }
                    Err(error) => {
                        failed_volumes.push(source.volume.display().to_string());
                        issues.push(IndexIssue {
                                path: source.volume.clone(),
                                message: format!(
                                    "Volume {} was indexed but its USN journal checkpoint could not be recorded: {error}",
                                    source.volume.display()
                                ),
                            });
                    }
                },
                Err(error) => {
                    failed_volumes.push(source.volume.display().to_string());
                    issues.push(IndexIssue {
                        path: source.volume.clone(),
                        message: format!(
                            "Volume {} is not supported for NTFS indexing: {error}",
                            source.volume.display()
                        ),
                    });
                }
            }
        }

        if indexed_volumes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "None of the selected volumes could be indexed: {}",
                    failed_volumes.join(", ")
                ),
            ));
        }

        self.journal_checkpoints.replace(journal_checkpoints);
        Ok(())
    }

    fn journal_checkpoints(&self) -> io::Result<Vec<JournalCheckpoint>> {
        Ok(self.journal_checkpoints.borrow().clone())
    }

    fn collect_updates(
        &self,
        checkpoints: &mut [JournalCheckpoint],
        out: &mut Vec<IndexUpdate>,
        issues: &mut Vec<IndexIssue>,
    ) -> io::Result<()> {
        let mut seen_roots: HashSet<PathBuf> = HashSet::new();
        for source in &self.sources {
            // Unresolvable or duplicate selections never produced a checkpoint of their own.
            let Ok(root) = volume_root_path(&source.volume) else {
                continue;
            };
            if seen_roots.insert(root) {
                source.collect_updates(checkpoints, out, issues)?;
            }
        }
        Ok(())
    }
}

/// Reads every journal chunk after `checkpoint`, appending the resulting updates in journal order
/// and advancing `checkpoint` until the journal reports no further records.
fn collect_journal_updates<J: JournalCheckpointReader>(
    volume_root: &Path,
    reader: &J,
    checkpoint: &mut JournalCheckpoint,
    out: &mut Vec<IndexUpdate>,
    issues: &mut Vec<IndexIssue>,
) -> io::Result<()> {
    let mut buffer = vec![0_u8; ENUMERATION_BUFFER_LEN];

    loop {
        let written = reader.read_changes(
            volume_root,
            checkpoint.journal_id,
            checkpoint.next_usn,
            &mut buffer,
        )?;
        let Some((next_usn, records)) = buffer
            .get(..written)
            .and_then(|chunk| Some((read_u64(chunk, 0)?, chunk.get(NEXT_REFERENCE_LEN..)?)))
        else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Reading the USN Change Journal of {} returned {written} bytes, too few to report the next USN",
                    volume_root.display()
                ),
            ));
        };

        let parsed = parse_records(records);
        for defect in parsed.defects {
            record_issue(issues, volume_root, defect);
        }
        out.extend(
            parsed
                .records
                .iter()
                .filter_map(|record| journal_update(record, volume_root)),
        );

        // USNs are non-negative, so the reinterpreting cast is lossless.
        let next_usn = next_usn as i64;
        if next_usn <= checkpoint.next_usn {
            break;
        }
        checkpoint.next_usn = next_usn;
    }

    Ok(())
}

/// Translates one journal record into an index change, or `None` when it does not affect names.
fn journal_update(record: &UsnRecord, volume_root: &Path) -> Option<IndexUpdate> {
    if record.reason & USN_REASON_FILE_DELETE != 0 {
        return Some(IndexUpdate::Remove {
            volume_identity: volume_identity(volume_root),
            file_id: u128::from(record.file_reference),
        });
    }

    // An old-name record describes where the file was; its new-name record follows.
    let names_previous_location = record.reason & USN_REASON_RENAME_OLD_NAME != 0
        && record.reason & USN_REASON_RENAME_NEW_NAME == 0;
    let names_current_location =
        record.reason & (USN_REASON_FILE_CREATE | USN_REASON_RENAME_NEW_NAME) != 0;

    (names_current_location && !names_previous_location)
        .then(|| IndexUpdate::Upsert(entry_for(record, volume_root)))
}

/// Reads every enumeration chunk for `volume_root` and appends one entry per
/// usable record to `out`, returning how many entries were appended.
///
/// Chunk boundaries, record lengths, and name ranges are all validated
/// before use; anything unusable is reported through `issues` and skipped so
/// that a single damaged record cannot discard an otherwise complete volume
/// index. The enumeration stops as soon as the file system reports no more
/// data, an unusable chunk, or a start reference that does not advance,
/// which keeps the loop finite even for inconsistent input.
fn collect_records(
    volume_root: &Path,
    cursor: &mut dyn MftEnumerationCursor,
    out: &mut Vec<SearchEntry>,
    issues: &mut Vec<IndexIssue>,
) -> io::Result<usize> {
    let mut buffer = vec![0_u8; ENUMERATION_BUFFER_LEN];
    let mut start_file_reference = 0_u64;
    let mut collected = 0_usize;
    let mut root_collected = false;
    let mut root_file_reference = None;

    loop {
        let written = cursor.next_chunk(start_file_reference, &mut buffer)?;
        if written == 0 {
            break;
        }

        let Some(chunk) = buffer.get(..written) else {
            record_issue(
                issues,
                volume_root,
                format!(
                    "Discarded an enumeration chunk reporting {written} bytes, more than the {ENUMERATION_BUFFER_LEN} byte buffer"
                ),
            );
            break;
        };

        let Some(next_file_reference) = read_u64(chunk, 0) else {
            record_issue(
                issues,
                volume_root,
                format!(
                    "Discarded a {written} byte enumeration chunk that is too short to report the next file reference number"
                ),
            );
            break;
        };

        let parsed = parse_records(&chunk[NEXT_REFERENCE_LEN..]);
        for defect in parsed.defects {
            record_issue(issues, volume_root, defect);
        }

        for record in parsed.records {
            if record.reason & USN_REASON_FILE_DELETE != 0 {
                continue;
            }

            if record.file_reference & FILE_REFERENCE_INDEX_MASK == ROOT_RECORD_INDEX {
                if root_collected {
                    continue;
                }
                root_collected = true;
                root_file_reference = Some(record.file_reference);
                out.push(root_entry(
                    volume_root,
                    record.file_reference,
                    record.attributes,
                ));
            } else {
                if root_file_reference.is_none()
                    && record.parent_reference & FILE_REFERENCE_INDEX_MASK == ROOT_RECORD_INDEX
                {
                    root_file_reference = Some(record.parent_reference);
                }
                out.push(entry_for(&record, volume_root));
            }
            collected += 1;
        }

        if next_file_reference <= start_file_reference {
            break;
        }
        start_file_reference = next_file_reference;
    }

    if collected > 0 && !root_collected {
        out.push(root_entry(
            volume_root,
            root_file_reference.unwrap_or(ROOT_RECORD_INDEX),
            FILE_ATTRIBUTE_DIRECTORY,
        ));
        collected += 1;
    }

    Ok(collected)
}

/// Builds the index entry for an enumerated record. Full paths are left to
/// the shared parent-child resolver, so the entry only has to carry the
/// record's own name until its lineage is known.
fn entry_for(record: &UsnRecord, volume_root: &Path) -> SearchEntry {
    SearchEntry {
        display: record.name.clone(),
        path: record.name.clone(),
        metadata: EntryMetadata {
            file_id: Some(u128::from(record.file_reference)),
            parent_id: Some(u128::from(record.parent_reference)),
            name: record.name.clone(),
            full_path: PathBuf::from(&record.name),
            volume_identity: Some(volume_identity(volume_root)),
            attributes: record.attributes,
        },
    }
}

/// Builds the entry that anchors every other record to `volume_root`.
///
/// The root directory is the one record whose path is known without a
/// parent, so it carries no parent identifier and keeps the validated volume
/// root as its full path. Records that reference it resolve against the same
/// reserved record index. `file_reference` is the complete NTFS file
/// reference number, including its sequence component.
fn root_entry(volume_root: &Path, file_reference: u64, attributes: u32) -> SearchEntry {
    let name = volume_root.display().to_string();

    SearchEntry {
        display: name.clone(),
        path: name.clone(),
        metadata: EntryMetadata {
            file_id: Some(u128::from(file_reference)),
            parent_id: None,
            name,
            full_path: volume_root.to_path_buf(),
            volume_identity: Some(volume_identity(volume_root)),
            attributes,
        },
    }
}

/// Identifies the volume that an entry was indexed from. The validated drive
/// root is stable for the lifetime of an index and keeps entries from
/// different volumes distinguishable once several are indexed together.
fn volume_identity(volume_root: &Path) -> String {
    volume_root.display().to_string()
}

fn record_issue(issues: &mut Vec<IndexIssue>, volume_root: &Path, message: String) {
    issues.push(IndexIssue {
        path: volume_root.to_path_buf(),
        message,
    });
}

/// A defensively parsed version 2 USN record, reduced to the fields the
/// index model needs.
#[derive(Clone, Debug, Eq, PartialEq)]
struct UsnRecord {
    file_reference: u64,
    parent_reference: u64,
    reason: u32,
    attributes: u32,
    name: String,
}

/// Records recovered from one enumeration chunk, plus a message for every
/// record that had to be skipped.
#[derive(Debug, Default, Eq, PartialEq)]
struct ParsedChunk {
    records: Vec<UsnRecord>,
    defects: Vec<String>,
}

/// Parses the consecutive USN records that follow the next-reference prefix
/// of an enumeration chunk.
///
/// Each record declares its own length, which is the only way to reach the
/// record that follows it. A length that is too small to hold a record
/// header, or that reaches past the end of the chunk, therefore makes the
/// rest of the chunk unusable: parsing reports the defect and stops there
/// rather than guessing an offset. Individual records that are intact enough
/// to skip past are reported and skipped on their own.
fn parse_records(records: &[u8]) -> ParsedChunk {
    let mut parsed = ParsedChunk::default();
    let mut offset = 0_usize;

    while let Some(declared_length) = read_u32(records, offset) {
        let remaining = records.len() - offset;
        let record_length = usize::try_from(declared_length).unwrap_or(usize::MAX);

        if record_length < USN_RECORD_V2_HEADER_LEN || record_length > remaining {
            parsed.defects.push(format!(
                "Stopped parsing a USN record chunk at a declared record length of {declared_length} bytes with {remaining} bytes remaining"
            ));
            return parsed;
        }

        let Some(record) = records.get(offset..offset + record_length) else {
            parsed.defects.push(format!(
                "Stopped parsing a USN record chunk at an unreadable {record_length} byte record"
            ));
            return parsed;
        };

        match parse_record(record) {
            Ok(record) => parsed.records.push(record),
            Err(defect) => parsed.defects.push(defect),
        }

        offset += record_length;
    }

    if offset < records.len() {
        parsed.defects.push(format!(
            "Discarded {} trailing bytes that cannot contain a USN record length",
            records.len() - offset
        ));
    }

    parsed
}

/// Parses one `USN_RECORD_V2`, whose bytes have already been bounded by its
/// declared record length.
///
/// Every field is read through a checked accessor and the variable-length
/// name is only decoded once its offset and length are known to describe a
/// range inside the record that does not overlap the record header. Records
/// that fail any of those checks are described in the returned error and
/// skipped by the caller.
fn parse_record(record: &[u8]) -> Result<UsnRecord, String> {
    let field = |offset: usize, name: &str| {
        format!("Skipped a USN record missing its {name} field at offset {offset}")
    };

    let major_version = read_u16(record, 4).ok_or_else(|| field(4, "major version"))?;
    if major_version != SUPPORTED_RECORD_MAJOR_VERSION {
        return Err(format!(
            "Skipped a USN record with unsupported major version {major_version}"
        ));
    }

    let file_reference = read_u64(record, 8).ok_or_else(|| field(8, "file reference number"))?;
    let parent_reference =
        read_u64(record, 16).ok_or_else(|| field(16, "parent file reference number"))?;
    let reason = read_u32(record, 40).ok_or_else(|| field(40, "reason"))?;
    let attributes = read_u32(record, 52).ok_or_else(|| field(52, "file attributes"))?;
    let name_length =
        usize::from(read_u16(record, 56).ok_or_else(|| field(56, "file name length"))?);
    let name_offset =
        usize::from(read_u16(record, 58).ok_or_else(|| field(58, "file name offset"))?);

    if name_length == 0 {
        return Err(format!(
            "Skipped a USN record for file reference number {file_reference} without a file name"
        ));
    }
    if name_length % 2 != 0 {
        return Err(format!(
            "Skipped a USN record with an odd file name length of {name_length} bytes"
        ));
    }
    if name_offset < USN_RECORD_V2_HEADER_LEN {
        return Err(format!(
            "Skipped a USN record with a file name offset of {name_offset} bytes that overlaps its {USN_RECORD_V2_HEADER_LEN} byte header"
        ));
    }

    let name_end = name_offset
        .checked_add(name_length)
        .ok_or_else(|| "Skipped a USN record with an unrepresentable file name range".to_owned())?;
    let name_bytes = record.get(name_offset..name_end).ok_or_else(|| {
        format!(
            "Skipped a USN record whose {name_length} byte file name at offset {name_offset} extends past its {} byte record",
            record.len()
        )
    })?;

    Ok(UsnRecord {
        file_reference,
        parent_reference,
        reason,
        attributes,
        name: decode_utf16(name_bytes),
    })
}

/// Decodes a little-endian UTF-16 file name. NTFS names are sequences of
/// 16-bit code units that are not required to be well-formed UTF-16, so
/// unpaired surrogates are replaced instead of rejecting the record.
fn decode_utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

/// Resolves the root path (for example `C:\`) of the local drive volume that
/// contains `path`. Returns an explicit error for inputs that do not name a
/// local drive, such as relative paths or UNC network shares, which are out
/// of scope for MFT-based indexing.
fn volume_root_path(path: &Path) -> io::Result<PathBuf> {
    match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                Ok(PathBuf::from(format!("{}:\\", letter as char)))
            }
            _ => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("{} does not name a local drive volume", path.display()),
            )),
        },
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{} has no drive letter to identify a volume",
                path.display()
            ),
        )),
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let end = offset.checked_add(2)?;
    let field: [u8; 2] = bytes.get(offset..end)?.try_into().ok()?;
    Some(u16::from_le_bytes(field))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    let field: [u8; 4] = bytes.get(offset..end)?.try_into().ok()?;
    Some(u32::from_le_bytes(field))
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    let end = offset.checked_add(8)?;
    let field: [u8; 8] = bytes.get(offset..end)?.try_into().ok()?;
    Some(u64::from_le_bytes(field))
}

/// [`VolumeInspector`] backed by the documented Win32 volume-management
/// APIs: `GetVolumeInformationW` reports the filesystem name, and opening
/// the raw volume device path (`\\.\<drive>:`) with `CreateFileW` validates
/// read access without reading any file content.
#[cfg(windows)]
#[derive(Default)]
pub struct WindowsVolumeInspector;

#[cfg(windows)]
impl VolumeInspector for WindowsVolumeInspector {
    fn filesystem_name(&self, volume_root: &Path) -> io::Result<String> {
        windows_volume::filesystem_name(volume_root)
    }

    fn check_read_access(&self, volume_root: &Path) -> io::Result<()> {
        windows_volume::check_read_access(volume_root)
    }
}

/// [`MftRecordEnumerator`] backed by the documented `FSCTL_ENUM_USN_DATA`
/// device control, which reports the volume's in-use master-file-table
/// records as USN records without walking directories.
#[cfg(windows)]
#[derive(Default)]
pub struct WindowsMftEnumerator;

#[cfg(windows)]
impl MftRecordEnumerator for WindowsMftEnumerator {
    fn open(&self, volume_root: &Path) -> io::Result<Box<dyn MftEnumerationCursor>> {
        Ok(Box::new(windows_volume::UsnEnumerationCursor::open(
            volume_root,
        )?))
    }
}

/// [`JournalCheckpointReader`] backed by the documented
/// `FSCTL_QUERY_USN_JOURNAL` device control.
#[cfg(windows)]
#[derive(Default)]
pub struct WindowsJournalCheckpointReader;

#[cfg(windows)]
impl JournalCheckpointReader for WindowsJournalCheckpointReader {
    fn read_state(&self, volume_root: &Path) -> io::Result<JournalState> {
        windows_volume::journal_state(volume_root)
    }

    fn read_changes(
        &self,
        volume_root: &Path,
        journal_id: u64,
        start_usn: i64,
        buffer: &mut [u8],
    ) -> io::Result<usize> {
        windows_volume::read_journal_changes(volume_root, journal_id, start_usn, buffer)
    }
}

/// Fallback used when compiling for a non-Windows target, where the Win32
/// volume APIs this source relies on are unavailable. Every query fails
/// explicitly instead of silently reporting success.
#[cfg(not(windows))]
#[derive(Default)]
pub struct WindowsVolumeInspector;

#[cfg(not(windows))]
impl VolumeInspector for WindowsVolumeInspector {
    fn filesystem_name(&self, volume_root: &Path) -> io::Result<String> {
        Err(unsupported_platform_error(volume_root))
    }

    fn check_read_access(&self, volume_root: &Path) -> io::Result<()> {
        Err(unsupported_platform_error(volume_root))
    }
}

/// Fallback used when compiling for a non-Windows target, where
/// `FSCTL_ENUM_USN_DATA` is unavailable. Opening a volume fails explicitly
/// instead of reporting an empty enumeration.
#[cfg(not(windows))]
#[derive(Default)]
pub struct WindowsMftEnumerator;

#[cfg(not(windows))]
impl MftRecordEnumerator for WindowsMftEnumerator {
    fn open(&self, volume_root: &Path) -> io::Result<Box<dyn MftEnumerationCursor>> {
        Err(unsupported_platform_error(volume_root))
    }
}

/// Fallback used when compiling for a non-Windows target, where querying the
/// NTFS USN Change Journal is unavailable.
#[cfg(not(windows))]
#[derive(Default)]
pub struct WindowsJournalCheckpointReader;

#[cfg(not(windows))]
impl JournalCheckpointReader for WindowsJournalCheckpointReader {
    fn read_state(&self, volume_root: &Path) -> io::Result<JournalState> {
        Err(unsupported_platform_error(volume_root))
    }

    fn read_changes(
        &self,
        volume_root: &Path,
        _journal_id: u64,
        _start_usn: i64,
        _buffer: &mut [u8],
    ) -> io::Result<usize> {
        Err(unsupported_platform_error(volume_root))
    }
}

#[cfg(not(windows))]
fn unsupported_platform_error(volume_root: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        format!(
            "NTFS volume access for {} requires the Win32 volume-management APIs, which are unavailable on this platform",
            volume_root.display()
        ),
    )
}

/// Returns the roots of all local fixed drives, such as `C:\`, in drive-letter order.
#[cfg(windows)]
pub fn local_fixed_volumes() -> Vec<PathBuf> {
    windows_volume::local_fixed_volumes()
}

#[cfg(not(windows))]
pub fn local_fixed_volumes() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(windows)]
mod windows_volume {
    use std::{
        io, mem,
        path::{Path, PathBuf},
        ptr,
    };

    use windows::{
        Win32::{
            Foundation::{
                CloseHandle, ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_HANDLE_EOF,
                ERROR_INVALID_DRIVE, ERROR_NOT_READY, ERROR_PATH_NOT_FOUND, GENERIC_READ, HANDLE,
            },
            Storage::FileSystem::{
                CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, GetDriveTypeW, GetLogicalDrives,
                GetVolumeInformationW, OPEN_EXISTING,
            },
            System::{
                IO::DeviceIoControl,
                Ioctl::{
                    FSCTL_ENUM_USN_DATA, FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_USN_JOURNAL,
                    MFT_ENUM_DATA_V0, READ_USN_JOURNAL_DATA_V0, USN_JOURNAL_DATA_V0,
                },
            },
        },
        core::{Error as WindowsError, HRESULT, HSTRING},
    };

    use super::{JOURNAL_REASON_MASK, JournalState, MftEnumerationCursor};

    /// `GetDriveTypeW` result for fixed disks, from `WinBase.h`.
    const DRIVE_FIXED: u32 = 3;

    pub(super) fn local_fixed_volumes() -> Vec<PathBuf> {
        // SAFETY: GetLogicalDrives has no preconditions.
        let drives = unsafe { GetLogicalDrives() };
        (0..26_u8)
            .filter(|bit| drives & (1 << bit) != 0)
            .map(|bit| PathBuf::from(format!("{}:\\", char::from(b'A' + bit))))
            .filter(|root| {
                let root = HSTRING::from(root.as_os_str());
                // SAFETY: `root` is a valid NUL-terminated wide string for the call's duration.
                let drive_type = unsafe { GetDriveTypeW(&root) };
                drive_type == DRIVE_FIXED
            })
            .collect()
    }

    /// Reads the journal records after `start_usn` without waiting for new changes.
    pub(super) fn read_journal_changes(
        volume_root: &Path,
        journal_id: u64,
        start_usn: i64,
        buffer: &mut [u8],
    ) -> io::Result<usize> {
        let handle = open_volume(volume_root)?;
        let request = READ_USN_JOURNAL_DATA_V0 {
            StartUsn: start_usn,
            ReasonMask: JOURNAL_REASON_MASK,
            ReturnOnlyOnClose: 0,
            Timeout: 0,
            BytesToWaitFor: 0,
            UsnJournalID: journal_id,
        };
        let request_len =
            u32::try_from(mem::size_of::<READ_USN_JOURNAL_DATA_V0>()).unwrap_or(u32::MAX);
        let buffer_len = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        let mut returned = 0u32;

        // SAFETY: `handle` is a valid volume handle, `request` outlives the
        // call and is passed with its own size, `buffer` is a live mutable
        // slice passed with its own capacity, and no overlapped I/O is
        // requested, so the call completes synchronously.
        let result = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_READ_USN_JOURNAL,
                Some(ptr::from_ref(&request).cast()),
                request_len,
                Some(buffer.as_mut_ptr().cast()),
                buffer_len,
                Some(&mut returned),
                None,
            )
        };

        // SAFETY: `handle` is owned locally and is not used after this call.
        let _ = unsafe { CloseHandle(handle) };

        result.map_err(|error| {
            to_io_error(
                &format!(
                    "Reading the USN Change Journal of {}",
                    volume_root.display()
                ),
                error,
            )
        })?;
        Ok(usize::try_from(returned).unwrap_or(0).min(buffer.len()))
    }

    /// Reads the current USN journal identity and readable range for `volume_root`.
    pub(super) fn journal_state(volume_root: &Path) -> io::Result<JournalState> {
        let handle = open_volume(volume_root)?;
        let mut journal_data = USN_JOURNAL_DATA_V0::default();
        let mut returned = 0u32;
        let output_len = u32::try_from(mem::size_of::<USN_JOURNAL_DATA_V0>()).unwrap_or(u32::MAX);

        // SAFETY: `handle` is a valid volume handle, `journal_data` is a
        // correctly sized mutable output structure, and synchronous I/O
        // completes before either local variable is dropped.
        let result = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_QUERY_USN_JOURNAL,
                None,
                0,
                Some(ptr::from_mut(&mut journal_data).cast()),
                output_len,
                Some(&mut returned),
                None,
            )
        };

        // SAFETY: `handle` is owned locally and is not used after this call.
        let _ = unsafe { CloseHandle(handle) };

        result.map_err(|error| {
            to_io_error(
                &format!(
                    "Querying the USN Change Journal of {}",
                    volume_root.display()
                ),
                error,
            )
        })?;
        if usize::try_from(returned).unwrap_or(0) != mem::size_of::<USN_JOURNAL_DATA_V0>() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Querying the USN Change Journal of {} returned {returned} bytes instead of {}",
                    volume_root.display(),
                    mem::size_of::<USN_JOURNAL_DATA_V0>()
                ),
            ));
        }

        Ok(JournalState {
            journal_id: journal_data.UsnJournalID,
            first_usn: journal_data.FirstUsn,
            next_usn: journal_data.NextUsn,
        })
    }

    /// Reads the filesystem name Windows reports for `volume_root` using
    /// `GetVolumeInformationW`, the documented API for this query.
    pub(super) fn filesystem_name(volume_root: &Path) -> io::Result<String> {
        let root = HSTRING::from(volume_root);
        let mut filesystem_name_buffer = [0u16; 32];

        // SAFETY: `root` is a valid, NUL-terminated wide string naming a
        // drive root, and `filesystem_name_buffer` is a live, correctly
        // sized `u16` buffer the call may write into; no other output
        // parameter is requested.
        unsafe {
            GetVolumeInformationW(
                &root,
                None,
                None,
                None,
                None,
                Some(&mut filesystem_name_buffer),
            )
        }
        .map_err(|error| {
            to_io_error(
                &format!("Reading volume information for {}", volume_root.display()),
                error,
            )
        })?;

        let end = filesystem_name_buffer
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(filesystem_name_buffer.len());
        Ok(String::from_utf16_lossy(&filesystem_name_buffer[..end]))
    }

    /// Validates read access to `volume_root` by opening its raw device path
    /// (`\\.\<drive>:`) with `CreateFileW`, then immediately closing the
    /// handle. No file content is read; this only proves the current
    /// process holds sufficient privilege to open the volume itself, which
    /// raw MFT access requires.
    pub(super) fn check_read_access(volume_root: &Path) -> io::Result<()> {
        let handle = open_volume(volume_root)?;

        // SAFETY: `handle` was just returned as a valid, owned handle by
        // `open_volume` above and is not used again after this call.
        let _ = unsafe { CloseHandle(handle) };
        Ok(())
    }

    /// Opens the raw volume device (`\\.\<drive>:`) for read access. The
    /// caller owns the returned handle and is responsible for closing it.
    fn open_volume(volume_root: &Path) -> io::Result<HANDLE> {
        let device_path = HSTRING::from(device_path_for(volume_root));

        // SAFETY: `device_path` is a valid, NUL-terminated wide string, no
        // security attributes or template handle are supplied, and the
        // returned handle is owned exclusively by the caller.
        unsafe {
            CreateFileW(
                &device_path,
                GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                Default::default(),
                None,
            )
        }
        .map_err(|error| {
            to_io_error(
                &format!("Opening volume {} for read access", volume_root.display()),
                error,
            )
        })
    }

    /// Cursor over the raw `FSCTL_ENUM_USN_DATA` output of one volume.
    ///
    /// The volume handle is opened once and closed when the cursor is
    /// dropped, so a full enumeration issues a single `CreateFileW` call and
    /// one device control per chunk.
    pub(super) struct UsnEnumerationCursor {
        handle: HANDLE,
        volume_root: PathBuf,
    }

    impl UsnEnumerationCursor {
        pub(super) fn open(volume_root: &Path) -> io::Result<Self> {
            Ok(Self {
                handle: open_volume(volume_root)?,
                volume_root: volume_root.to_path_buf(),
            })
        }
    }

    impl MftEnumerationCursor for UsnEnumerationCursor {
        fn next_chunk(
            &mut self,
            start_file_reference: u64,
            buffer: &mut [u8],
        ) -> io::Result<usize> {
            let request = MFT_ENUM_DATA_V0 {
                StartFileReferenceNumber: start_file_reference,
                LowUsn: 0,
                HighUsn: i64::MAX,
            };
            let request_len = u32::try_from(mem::size_of::<MFT_ENUM_DATA_V0>()).unwrap_or(u32::MAX);
            let buffer_len = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
            let mut returned = 0u32;

            // SAFETY: `self.handle` is a live volume handle owned by this
            // cursor, `request` outlives the call and is passed with its own
            // size, `buffer` is a live mutable slice passed with its own
            // capacity, `returned` is a live `u32`, and no overlapped I/O is
            // requested, so the call completes synchronously.
            let result = unsafe {
                DeviceIoControl(
                    self.handle,
                    FSCTL_ENUM_USN_DATA,
                    Some(ptr::from_ref(&request).cast()),
                    request_len,
                    Some(buffer.as_mut_ptr().cast()),
                    buffer_len,
                    Some(&mut returned),
                    None,
                )
            };

            match result {
                Ok(()) => Ok(usize::try_from(returned).unwrap_or(0).min(buffer.len())),
                // The documented end-of-enumeration signal, not a failure.
                Err(error) if win32_code(error.code()) == Some(ERROR_HANDLE_EOF.0) => Ok(0),
                Err(error) => Err(to_io_error(
                    &format!(
                        "Enumerating master file table records of {}",
                        self.volume_root.display()
                    ),
                    error,
                )),
            }
        }
    }

    impl Drop for UsnEnumerationCursor {
        fn drop(&mut self) {
            // SAFETY: `handle` was returned by `open_volume`, is owned
            // exclusively by this cursor, and is closed exactly once here.
            let _ = unsafe { CloseHandle(self.handle) };
        }
    }

    /// Builds the raw volume device path (for example `\\.\C:`) that
    /// `CreateFileW` expects, from a drive root such as `C:\`.
    fn device_path_for(volume_root: &Path) -> String {
        let root = volume_root.display().to_string();
        let drive = root.trim_end_matches(['\\', '/']);
        format!(r"\\.\{drive}")
    }

    fn to_io_error(context: &str, error: WindowsError) -> io::Error {
        io::Error::new(
            classify(error.code()),
            format!("{context}: {}", error.message().trim()),
        )
    }

    /// Maps well-known Win32 error codes to the closest matching
    /// [`io::ErrorKind`], preserving the original message for diagnostics.
    fn classify(code: HRESULT) -> io::ErrorKind {
        match win32_code(code) {
            Some(code) if code == ERROR_ACCESS_DENIED.0 => io::ErrorKind::PermissionDenied,
            Some(code)
                if code == ERROR_FILE_NOT_FOUND.0
                    || code == ERROR_PATH_NOT_FOUND.0
                    || code == ERROR_INVALID_DRIVE.0
                    || code == ERROR_NOT_READY.0 =>
            {
                io::ErrorKind::NotFound
            }
            _ => io::ErrorKind::Other,
        }
    }

    /// Extracts the original Win32 error code from an `HRESULT` produced by
    /// `HRESULT::from_win32` (the `FACILITY_WIN32` encoding used for errors
    /// surfaced through `GetLastError`), if `code` was constructed that way.
    fn win32_code(code: HRESULT) -> Option<u32> {
        let bits = code.0 as u32;
        if bits & 0xffff_0000 == 0x8007_0000 {
            Some(bits & 0xffff)
        } else {
            None
        }
    }

    #[cfg(test)]
    mod tests {
        use std::io::ErrorKind;

        use windows::Win32::Foundation::{
            ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_HANDLE_EOF,
        };

        use super::{HRESULT, classify, win32_code};

        #[test]
        fn maps_win32_access_denied_to_permission_denied() {
            let code = HRESULT::from_win32(ERROR_ACCESS_DENIED.0);

            assert_eq!(win32_code(code), Some(ERROR_ACCESS_DENIED.0));
            assert_eq!(classify(code), ErrorKind::PermissionDenied);
        }

        #[test]
        fn maps_win32_file_not_found_to_not_found() {
            let code = HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0);

            assert_eq!(classify(code), ErrorKind::NotFound);
        }

        #[test]
        fn recognizes_the_end_of_enumeration_error_code() {
            let code = HRESULT::from_win32(ERROR_HANDLE_EOF.0);

            assert_eq!(win32_code(code), Some(ERROR_HANDLE_EOF.0));
        }

        #[test]
        fn treats_non_win32_hresults_as_other() {
            let code = HRESULT(-1);

            assert_eq!(win32_code(code), None);
            assert_eq!(classify(code), ErrorKind::Other);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        collections::VecDeque,
        fs, io,
        path::{Path, PathBuf},
        rc::Rc,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::search::{
        IndexUpdate, JournalCheckpoint, SearchIndex, WorkspaceSource, resolve_entry_paths,
    };

    use super::{
        IndexIssue, IndexSource, JournalCheckpointReader, JournalState, MftEnumerationCursor,
        MftRecordEnumerator, NtfsMftSource, NtfsMultiVolumeSource, SearchEntry, VolumeInspector,
        volume_root_path,
    };

    /// Fixed part of a `USN_RECORD_V2`, mirrored here so the encoder in
    /// these tests stays independent of the parser under test.
    const HEADER_LEN: usize = 60;
    const DIRECTORY_ATTRIBUTE: u32 = 0x0000_0010;
    const ARCHIVE_ATTRIBUTE: u32 = 0x0000_0020;
    const DELETE_REASON: u32 = 0x0000_0200;
    const DATA_EXTEND_REASON: u32 = 0x0000_0002;
    const CREATE_REASON: u32 = 0x0000_0100;
    const RENAME_OLD_NAME_REASON: u32 = 0x0000_1000;
    const RENAME_NEW_NAME_REASON: u32 = 0x0000_2000;
    const CLOSE_REASON: u32 = 0x8000_0000;

    /// File reference numbers combine a reuse sequence number in the high 16
    /// bits with the MFT record index in the low 48 bits.
    const ROOT_REFERENCE: u64 = 0x0005_0000_0000_0005;
    const PROJECTS_REFERENCE: u64 = 0x0001_0000_0000_002a;
    const NOTES_REFERENCE: u64 = 0x0002_0000_0000_002b;
    const ALPHA_REFERENCE: u64 = 0x0001_0000_0000_0100;
    const NESTED_REFERENCE: u64 = 0x0001_0000_0000_0101;
    const BRAVO_REFERENCE: u64 = 0x0001_0000_0000_0102;
    const ROOT_ID: u128 = ROOT_REFERENCE as u128;
    const PROJECTS_ID: u128 = PROJECTS_REFERENCE as u128;
    const NOTES_ID: u128 = NOTES_REFERENCE as u128;
    const ALPHA_ID: u128 = ALPHA_REFERENCE as u128;

    /// Deterministic [`VolumeInspector`] fake that reports fixed filesystem
    /// and access-check outcomes without touching any real volume, so
    /// `NtfsMftSource` behavior can be tested independent of the host
    /// machine's drives or process privilege level.
    struct FakeVolumeInspector {
        filesystem: io::Result<&'static str>,
        access: io::Result<()>,
        access_checked: Cell<bool>,
    }

    impl FakeVolumeInspector {
        fn new(filesystem: &'static str) -> Self {
            Self {
                filesystem: Ok(filesystem),
                access: Ok(()),
                access_checked: Cell::new(false),
            }
        }

        fn with_access_error(mut self, error: io::Error) -> Self {
            self.access = Err(error);
            self
        }

        fn with_filesystem_error(mut self, error: io::Error) -> Self {
            self.filesystem = Err(error);
            self
        }
    }

    impl VolumeInspector for FakeVolumeInspector {
        fn filesystem_name(&self, _volume_root: &Path) -> io::Result<String> {
            match &self.filesystem {
                Ok(name) => Ok((*name).to_owned()),
                Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
            }
        }

        fn check_read_access(&self, _volume_root: &Path) -> io::Result<()> {
            self.access_checked.set(true);
            match &self.access {
                Ok(()) => Ok(()),
                Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
            }
        }
    }

    enum FakeJournalCheckpointReader {
        Success,
        Failure,
    }

    impl JournalCheckpointReader for FakeJournalCheckpointReader {
        fn read_state(&self, _volume_root: &Path) -> io::Result<JournalState> {
            match self {
                Self::Success => Ok(JournalState {
                    journal_id: 42,
                    first_usn: 0,
                    next_usn: 84,
                }),
                Self::Failure => Err(io::Error::other("journal query failed")),
            }
        }

        fn read_changes(
            &self,
            _volume_root: &Path,
            _journal_id: u64,
            _start_usn: i64,
            _buffer: &mut [u8],
        ) -> io::Result<usize> {
            Err(io::Error::other("no journal changes prepared"))
        }
    }

    /// Replays prepared `FSCTL_READ_USN_JOURNAL` output chunks and records every requested position.
    #[derive(Clone)]
    struct FakeJournal {
        changes: Rc<RefCell<VecDeque<Vec<u8>>>>,
        requests: Rc<RefCell<Vec<(u64, i64)>>>,
        state: Rc<Cell<JournalState>>,
    }

    impl Default for FakeJournal {
        fn default() -> Self {
            Self {
                changes: Rc::default(),
                requests: Rc::default(),
                state: Rc::new(Cell::new(JournalState {
                    journal_id: 42,
                    first_usn: 0,
                    next_usn: 84,
                })),
            }
        }
    }

    impl FakeJournal {
        fn with_changes(chunks: Vec<Vec<u8>>) -> Self {
            let journal = Self::default();
            journal.changes.borrow_mut().extend(chunks);
            journal
        }

        fn set_state(&self, state: JournalState) {
            self.state.set(state);
        }

        fn requests(&self) -> Vec<(u64, i64)> {
            self.requests.borrow().clone()
        }
    }

    impl JournalCheckpointReader for FakeJournal {
        fn read_state(&self, _volume_root: &Path) -> io::Result<JournalState> {
            Ok(self.state.get())
        }

        fn read_changes(
            &self,
            _volume_root: &Path,
            journal_id: u64,
            start_usn: i64,
            buffer: &mut [u8],
        ) -> io::Result<usize> {
            self.requests.borrow_mut().push((journal_id, start_usn));
            // An exhausted journal reports only the unchanged next USN.
            let chunk = self
                .changes
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| start_usn.to_le_bytes().to_vec());
            let written = chunk.len().min(buffer.len());
            buffer[..written].copy_from_slice(&chunk[..written]);
            Ok(written)
        }
    }

    #[derive(Default)]
    struct FakeEnumerationState {
        chunks: VecDeque<io::Result<Vec<u8>>>,
        starts: Vec<u64>,
        opened: usize,
    }

    /// Deterministic [`MftRecordEnumerator`] fake that replays prepared
    /// `FSCTL_ENUM_USN_DATA` output chunks and records the start file
    /// reference number requested for each one.
    #[derive(Clone, Default)]
    struct FakeEnumerator {
        state: Rc<RefCell<FakeEnumerationState>>,
    }

    impl FakeEnumerator {
        fn with_chunks(chunks: Vec<Vec<u8>>) -> Self {
            let enumerator = Self::default();
            enumerator
                .state
                .borrow_mut()
                .chunks
                .extend(chunks.into_iter().map(Ok));
            enumerator
        }

        fn with_error(error: io::Error) -> Self {
            let enumerator = Self::default();
            enumerator.state.borrow_mut().chunks.push_back(Err(error));
            enumerator
        }

        fn starts(&self) -> Vec<u64> {
            self.state.borrow().starts.clone()
        }

        fn opened(&self) -> usize {
            self.state.borrow().opened
        }
    }

    impl MftRecordEnumerator for FakeEnumerator {
        fn open(&self, _volume_root: &Path) -> io::Result<Box<dyn MftEnumerationCursor>> {
            self.state.borrow_mut().opened += 1;
            Ok(Box::new(FakeCursor {
                state: Rc::clone(&self.state),
            }))
        }
    }

    struct FakeCursor {
        state: Rc<RefCell<FakeEnumerationState>>,
    }

    impl MftEnumerationCursor for FakeCursor {
        fn next_chunk(
            &mut self,
            start_file_reference: u64,
            buffer: &mut [u8],
        ) -> io::Result<usize> {
            let mut state = self.state.borrow_mut();
            state.starts.push(start_file_reference);
            match state.chunks.pop_front() {
                None => Ok(0),
                Some(Err(error)) => Err(error),
                Some(Ok(chunk)) => {
                    let written = chunk.len().min(buffer.len());
                    buffer[..written].copy_from_slice(&chunk[..written]);
                    Ok(written)
                }
            }
        }
    }

    /// Builder for a single synthetic `USN_RECORD_V2`, including malformed
    /// variants that the parser has to survive.
    struct FakeRecord {
        file_reference: u64,
        parent_reference: u64,
        attributes: u32,
        reason: u32,
        name: String,
        major_version: u16,
        record_length: Option<u32>,
        name_offset: Option<u16>,
        name_length: Option<u16>,
    }

    impl FakeRecord {
        fn new(file_reference: u64, parent_reference: u64, name: &str) -> Self {
            Self {
                file_reference,
                parent_reference,
                attributes: 0,
                reason: 0,
                name: name.to_owned(),
                major_version: 2,
                record_length: None,
                name_offset: None,
                name_length: None,
            }
        }

        fn with_attributes(mut self, attributes: u32) -> Self {
            self.attributes = attributes;
            self
        }

        fn with_reason(mut self, reason: u32) -> Self {
            self.reason = reason;
            self
        }

        fn with_major_version(mut self, major_version: u16) -> Self {
            self.major_version = major_version;
            self
        }

        fn with_record_length(mut self, record_length: u32) -> Self {
            self.record_length = Some(record_length);
            self
        }

        fn with_name_offset(mut self, name_offset: u16) -> Self {
            self.name_offset = Some(name_offset);
            self
        }

        fn with_name_length(mut self, name_length: u16) -> Self {
            self.name_length = Some(name_length);
            self
        }

        fn encode(&self) -> Vec<u8> {
            let name_units: Vec<u16> = self.name.encode_utf16().collect();
            let name_offset = usize::from(self.name_offset.unwrap_or(HEADER_LEN as u16));
            let name_length = self.name_length.unwrap_or((name_units.len() * 2) as u16);
            let mut bytes = vec![0u8; (name_offset + name_units.len() * 2).max(HEADER_LEN)];
            let record_length = self.record_length.unwrap_or(bytes.len() as u32);

            bytes[0..4].copy_from_slice(&record_length.to_le_bytes());
            bytes[4..6].copy_from_slice(&self.major_version.to_le_bytes());
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

    /// Builds one enumeration chunk: the next start file reference number
    /// followed by the encoded records.
    fn chunk(next_file_reference: u64, records: &[FakeRecord]) -> Vec<u8> {
        let mut bytes = next_file_reference.to_le_bytes().to_vec();
        for record in records {
            bytes.extend_from_slice(&record.encode());
        }
        bytes
    }

    fn ntfs_source(
        volume: &str,
        enumerator: FakeEnumerator,
    ) -> NtfsMftSource<FakeVolumeInspector, FakeEnumerator, FakeJournalCheckpointReader> {
        NtfsMftSource::with_parts_and_journal(
            volume,
            FakeVolumeInspector::new("NTFS"),
            enumerator,
            FakeJournalCheckpointReader::Success,
        )
    }

    fn collect_entries<J: JournalCheckpointReader>(
        source: &NtfsMftSource<FakeVolumeInspector, FakeEnumerator, J>,
    ) -> (Vec<SearchEntry>, Vec<IndexIssue>) {
        let mut out: Vec<SearchEntry> = Vec::new();
        let mut issues: Vec<IndexIssue> = Vec::new();
        source.collect_initial(&mut out, &mut issues).unwrap();
        (out, issues)
    }

    fn collect_initial_error<J: JournalCheckpointReader>(
        source: &NtfsMftSource<FakeVolumeInspector, FakeEnumerator, J>,
    ) -> io::Error {
        let mut out: Vec<SearchEntry> = Vec::new();
        let mut issues: Vec<IndexIssue> = Vec::new();
        source.collect_initial(&mut out, &mut issues).unwrap_err()
    }

    #[test]
    fn records_a_journal_checkpoint_for_an_indexed_volume() {
        let source = NtfsMftSource::with_parts_and_journal(
            r"C:\",
            FakeVolumeInspector::new("NTFS"),
            FakeEnumerator::with_chunks(vec![volume_tree_chunk()]),
            FakeJournalCheckpointReader::Success,
        );
        let index = SearchIndex::from_source(&source)
            .expect("the successfully indexed volume must retain its journal checkpoint");

        assert_eq!(
            index.journal_checkpoints(),
            &[JournalCheckpoint {
                volume_identity: r"C:\".to_owned(),
                journal_id: 42,
                next_usn: 84,
            }]
        );
    }

    #[test]
    fn records_journal_checkpoints_for_each_completed_multi_volume_index() {
        let c_drive = NtfsMftSource::with_parts_and_journal(
            r"C:\",
            FakeVolumeInspector::new("NTFS"),
            FakeEnumerator::with_chunks(vec![volume_tree_chunk()]),
            FakeJournalCheckpointReader::Success,
        );
        let d_drive = NtfsMftSource::with_parts_and_journal(
            r"D:\",
            FakeVolumeInspector::new("NTFS"),
            FakeEnumerator::with_chunks(vec![volume_tree_chunk()]),
            FakeJournalCheckpointReader::Success,
        );
        let source = NtfsMultiVolumeSource::with_sources(vec![c_drive, d_drive]);
        let index = SearchIndex::from_source(&source)
            .expect("each successfully indexed volume must retain a journal checkpoint");

        assert_eq!(
            index.journal_checkpoints(),
            &[
                JournalCheckpoint {
                    volume_identity: r"C:\".to_owned(),
                    journal_id: 42,
                    next_usn: 84,
                },
                JournalCheckpoint {
                    volume_identity: r"D:\".to_owned(),
                    journal_id: 42,
                    next_usn: 84,
                },
            ]
        );
    }

    #[test]
    fn omits_a_volume_when_its_journal_checkpoint_cannot_be_recorded() {
        let c_drive = NtfsMftSource::with_parts_and_journal(
            r"C:\",
            FakeVolumeInspector::new("NTFS"),
            FakeEnumerator::with_chunks(vec![volume_tree_chunk()]),
            FakeJournalCheckpointReader::Success,
        );
        let d_drive = NtfsMftSource::with_parts_and_journal(
            r"D:\",
            FakeVolumeInspector::new("NTFS"),
            FakeEnumerator::with_chunks(vec![volume_tree_chunk()]),
            FakeJournalCheckpointReader::Failure,
        );
        let source = NtfsMultiVolumeSource::with_sources(vec![c_drive, d_drive]);
        let index = SearchIndex::from_source(&source)
            .expect("the volume with a recorded checkpoint must remain available");

        assert_eq!(index.journal_checkpoints().len(), 1);
        assert_eq!(index.indexing_issue_count(), 1);
        assert_eq!(index.len(), 3);
    }

    fn journal_source(
        volume: &str,
        journal: FakeJournal,
    ) -> NtfsMftSource<FakeVolumeInspector, FakeEnumerator, FakeJournal> {
        NtfsMftSource::with_parts_and_journal(
            volume,
            FakeVolumeInspector::new("NTFS"),
            FakeEnumerator::with_chunks(vec![volume_tree_chunk()]),
            journal,
        )
    }

    fn c_drive_checkpoint() -> JournalCheckpoint {
        JournalCheckpoint {
            volume_identity: r"C:\".to_owned(),
            journal_id: 42,
            next_usn: 84,
        }
    }

    fn describe(update: &IndexUpdate) -> String {
        match update {
            IndexUpdate::Upsert(entry) => format!(
                "upsert {} under {:?} on {:?}",
                entry.metadata.name, entry.metadata.parent_id, entry.metadata.volume_identity
            ),
            IndexUpdate::Remove {
                volume_identity,
                file_id,
            } => format!("remove {file_id} on {volume_identity}"),
            IndexUpdate::ReplaceVolume {
                volume_identity,
                entries,
            } => format!("replace {volume_identity} with {} entries", entries.len()),
        }
    }

    fn assert_rebuilt_when_journal_is(state: JournalState, reason: &str) {
        let journal = FakeJournal::default();
        journal.set_state(state);
        let source = journal_source(r"C:\", journal.clone());
        let mut checkpoints = vec![c_drive_checkpoint()];
        let mut updates = Vec::new();
        let mut issues = Vec::new();

        source
            .collect_updates(&mut checkpoints, &mut updates, &mut issues)
            .unwrap();

        assert_eq!(
            updates.iter().map(describe).collect::<Vec<_>>(),
            vec![r"replace C:\ with 3 entries"],
        );
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains(reason), "{}", issues[0].message);
        assert_eq!(
            checkpoints,
            vec![JournalCheckpoint {
                volume_identity: r"C:\".to_owned(),
                journal_id: state.journal_id,
                next_usn: state.next_usn,
            }],
        );
        assert!(journal.requests().is_empty());
    }

    #[test]
    fn rebuilds_a_volume_whose_journal_was_recreated() {
        assert_rebuilt_when_journal_is(
            JournalState {
                journal_id: 43,
                first_usn: 0,
                next_usn: 16,
            },
            "recreated",
        );
    }

    #[test]
    fn rebuilds_a_volume_whose_checkpoint_was_purged_from_the_journal() {
        assert_rebuilt_when_journal_is(
            JournalState {
                journal_id: 42,
                first_usn: 100,
                next_usn: 400,
            },
            "purged",
        );
    }

    #[test]
    fn rebuilds_a_volume_whose_checkpoint_is_past_the_journal_end() {
        assert_rebuilt_when_journal_is(
            JournalState {
                journal_id: 42,
                first_usn: 0,
                next_usn: 50,
            },
            "past",
        );
    }

    #[test]
    fn refresh_replaces_a_stale_volume_with_its_rebuilt_snapshot() {
        let journal = FakeJournal::default();
        let source = NtfsMftSource::with_parts_and_journal(
            r"C:\",
            FakeVolumeInspector::new("NTFS"),
            FakeEnumerator::with_chunks(vec![
                volume_tree_chunk(),
                chunk(
                    0,
                    &[
                        FakeRecord::new(ROOT_REFERENCE, ROOT_REFERENCE, ".")
                            .with_attributes(DIRECTORY_ATTRIBUTE),
                        FakeRecord::new(ALPHA_REFERENCE, ROOT_REFERENCE, "fresh.txt"),
                    ],
                ),
            ]),
            journal.clone(),
        );
        let mut index = SearchIndex::from_source(&source).unwrap();
        journal.set_state(JournalState {
            journal_id: 43,
            first_usn: 0,
            next_usn: 16,
        });

        index.refresh(&source).unwrap();

        assert_eq!(index.search("", 10), vec![r"C:\", r"C:\fresh.txt"]);
        assert_eq!(index.journal_checkpoints()[0].journal_id, 43);
        assert_eq!(index.indexing_issue_count(), 1);
    }

    #[test]
    fn translates_journal_records_into_ordered_updates_and_advances_the_checkpoint() {
        let journal = FakeJournal::with_changes(vec![chunk(
            200,
            &[
                FakeRecord::new(ALPHA_REFERENCE, ROOT_REFERENCE, "alpha.txt")
                    .with_reason(CREATE_REASON),
                FakeRecord::new(ALPHA_REFERENCE, ROOT_REFERENCE, "alpha.txt")
                    .with_reason(RENAME_OLD_NAME_REASON),
                FakeRecord::new(ALPHA_REFERENCE, PROJECTS_REFERENCE, "bravo.txt")
                    .with_reason(RENAME_NEW_NAME_REASON),
                FakeRecord::new(ALPHA_REFERENCE, PROJECTS_REFERENCE, "bravo.txt")
                    .with_reason(DATA_EXTEND_REASON),
                FakeRecord::new(ALPHA_REFERENCE, PROJECTS_REFERENCE, "bravo.txt")
                    .with_reason(DELETE_REASON | CLOSE_REASON),
            ],
        )]);
        let source = journal_source(r"C:\", journal.clone());
        let mut checkpoints = vec![c_drive_checkpoint()];
        let mut updates = Vec::new();
        let mut issues = Vec::new();

        source
            .collect_updates(&mut checkpoints, &mut updates, &mut issues)
            .unwrap();

        assert!(issues.is_empty());
        assert_eq!(
            updates.iter().map(describe).collect::<Vec<_>>(),
            vec![
                format!("upsert alpha.txt under Some({ROOT_ID}) on Some(\"C:\\\\\")"),
                format!("upsert bravo.txt under Some({PROJECTS_ID}) on Some(\"C:\\\\\")"),
                format!("remove {ALPHA_ID} on C:\\"),
            ],
        );
        assert_eq!(journal.requests(), vec![(42, 84), (42, 200)]);
        assert_eq!(checkpoints[0].next_usn, 200);
    }

    #[test]
    fn refreshes_an_ntfs_index_with_moved_created_and_deleted_files() {
        let journal = FakeJournal::with_changes(vec![chunk(
            160,
            &[
                FakeRecord::new(NOTES_REFERENCE, ROOT_REFERENCE, "todo.txt")
                    .with_reason(RENAME_NEW_NAME_REASON),
                FakeRecord::new(ALPHA_REFERENCE, PROJECTS_REFERENCE, "plan.md")
                    .with_reason(CREATE_REASON),
                FakeRecord::new(BRAVO_REFERENCE, ROOT_REFERENCE, "scratch.txt")
                    .with_reason(CREATE_REASON),
                FakeRecord::new(BRAVO_REFERENCE, ROOT_REFERENCE, "scratch.txt")
                    .with_reason(DELETE_REASON),
            ],
        )]);
        let source = journal_source(r"C:\", journal);
        let mut index = SearchIndex::from_source(&source).unwrap();

        let applied = index.refresh(&source).unwrap();

        assert_eq!(applied, 4);
        assert_eq!(
            index.search("", 10),
            vec![
                r"C:\",
                r"C:\projects",
                r"C:\projects\plan.md",
                r"C:\todo.txt"
            ],
        );
        assert_eq!(index.journal_checkpoints()[0].next_usn, 160);
    }

    #[test]
    fn refreshes_each_volume_from_its_own_journal_checkpoint() {
        let c_journal = FakeJournal::default();
        let d_journal = FakeJournal::with_changes(vec![chunk(
            300,
            &[
                FakeRecord::new(ALPHA_REFERENCE, ROOT_REFERENCE, "report.txt")
                    .with_reason(CREATE_REASON),
            ],
        )]);
        let source = NtfsMultiVolumeSource::with_sources(vec![
            journal_source(r"C:\", c_journal.clone()),
            journal_source(r"D:\", d_journal.clone()),
        ]);
        let mut index = SearchIndex::from_source(&source).unwrap();

        index.refresh(&source).unwrap();

        assert_eq!(index.search("report", 10), vec![r"D:\report.txt"]);
        assert_eq!(index.search("notes", 10).len(), 2);
        assert_eq!(c_journal.requests(), vec![(42, 84)]);
        assert_eq!(d_journal.requests(), vec![(42, 84), (42, 300)]);
        assert_eq!(
            index
                .journal_checkpoints()
                .iter()
                .map(|checkpoint| checkpoint.next_usn)
                .collect::<Vec<_>>(),
            vec![84, 300],
        );
    }

    #[test]
    fn rejects_a_journal_chunk_too_short_for_the_next_usn() {
        let source = journal_source(r"C:\", FakeJournal::with_changes(vec![vec![1, 2, 3]]));
        let mut checkpoints = vec![c_drive_checkpoint()];

        let error = source
            .collect_updates(&mut checkpoints, &mut Vec::new(), &mut Vec::new())
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(checkpoints[0].next_usn, 84);
    }

    fn entry<'a>(entries: &'a [SearchEntry], name: &str) -> &'a SearchEntry {
        entries
            .iter()
            .find(|entry| entry.metadata.name == name)
            .unwrap_or_else(|| panic!("no entry named {name}"))
    }

    fn volume_tree_chunk() -> Vec<u8> {
        chunk(
            0,
            &[
                FakeRecord::new(ROOT_REFERENCE, ROOT_REFERENCE, ".")
                    .with_attributes(DIRECTORY_ATTRIBUTE),
                FakeRecord::new(PROJECTS_REFERENCE, ROOT_REFERENCE, "projects")
                    .with_attributes(DIRECTORY_ATTRIBUTE),
                FakeRecord::new(NOTES_REFERENCE, PROJECTS_REFERENCE, "notes.txt")
                    .with_attributes(ARCHIVE_ATTRIBUTE),
            ],
        )
    }

    #[derive(Default)]
    struct TemporaryDirectory {
        path: PathBuf,
    }

    impl TemporaryDirectory {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("the system clock must be after the Unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "everything-lite-mft-comparison-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("the test temporary directory must be created");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TemporaryDirectory {
        fn drop(&mut self) {
            if self.path.is_dir() {
                let _ = fs::remove_dir_all(&self.path);
            }
        }
    }

    #[test]
    fn mft_and_workspace_sources_reconstruct_matching_nested_entries() {
        let temporary_directory = TemporaryDirectory::new();
        let root = temporary_directory.path();
        fs::create_dir_all(root.join("nested"))
            .expect("the nested workspace directory must be created");
        fs::write(root.join("alpha.txt"), "").expect("the workspace root file must be created");
        fs::write(root.join("nested").join("bravo.txt"), "")
            .expect("the nested workspace file must be created");

        let workspace_index = SearchIndex::from_source(&WorkspaceSource::new(root))
            .expect("the workspace tree must be indexed");
        let workspace_entries = workspace_index
            .search("", usize::MAX)
            .into_iter()
            .map(|display| {
                let path = PathBuf::from(&display);
                let name = path
                    .file_name()
                    .expect("workspace displays must name an entry")
                    .to_string_lossy()
                    .into_owned();
                (path, name)
            })
            .collect::<Vec<_>>();

        let volume_root = volume_root_path(root).expect("the temporary tree must be on a drive");
        let relative_root_components = root
            .strip_prefix(&volume_root)
            .expect("the temporary tree must be below its volume root")
            .components()
            .map(|component| component.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(
            !relative_root_components.is_empty(),
            "the temporary tree must not be the volume root"
        );

        let mut records = vec![
            FakeRecord::new(ROOT_REFERENCE, ROOT_REFERENCE, ".")
                .with_attributes(DIRECTORY_ATTRIBUTE),
        ];
        let mut parent_reference = ROOT_REFERENCE;
        for (index, name) in relative_root_components.iter().enumerate() {
            let file_reference = 0x0001_0000_0000_0200 + index as u64;
            records.push(
                FakeRecord::new(file_reference, parent_reference, name)
                    .with_attributes(DIRECTORY_ATTRIBUTE),
            );
            parent_reference = file_reference;
        }
        records.extend([
            FakeRecord::new(ALPHA_REFERENCE, parent_reference, "alpha.txt")
                .with_attributes(ARCHIVE_ATTRIBUTE),
            FakeRecord::new(NESTED_REFERENCE, parent_reference, "nested")
                .with_attributes(DIRECTORY_ATTRIBUTE),
            FakeRecord::new(BRAVO_REFERENCE, NESTED_REFERENCE, "bravo.txt")
                .with_attributes(ARCHIVE_ATTRIBUTE),
        ]);

        let mft_index = SearchIndex::from_source(&ntfs_source(
            &volume_root.display().to_string(),
            FakeEnumerator::with_chunks(vec![chunk(0, &records)]),
        ))
        .expect("the matching synthetic MFT records must be indexed");
        let mft_entries = mft_index
            .search("", usize::MAX)
            .into_iter()
            .filter_map(|display| {
                let path = PathBuf::from(display);
                let relative_path = path.strip_prefix(root).ok()?.to_path_buf();
                (!relative_path.as_os_str().is_empty()).then(|| {
                    let name = relative_path
                        .file_name()
                        .expect("derived MFT paths must name an entry")
                        .to_string_lossy()
                        .into_owned();
                    (relative_path, name)
                })
            })
            .collect::<Vec<_>>();

        let expected = vec![
            (PathBuf::from("alpha.txt"), "alpha.txt".to_owned()),
            (PathBuf::from("nested"), "nested".to_owned()),
            (PathBuf::from(r"nested\bravo.txt"), "bravo.txt".to_owned()),
        ];
        assert_eq!(workspace_entries, expected);
        assert_eq!(mft_entries, expected);
        assert_eq!(mft_entries, workspace_entries);
        assert_eq!(mft_index.indexing_issue_count(), 0);
    }

    #[test]
    fn extracts_the_drive_root_from_an_absolute_path() {
        let root = volume_root_path(&PathBuf::from(r"C:\Users\test\file.txt")).unwrap();

        assert_eq!(root, PathBuf::from(r"C:\"));
    }

    #[test]
    fn new_uses_the_default_windows_volume_components() {
        let source = NtfsMftSource::new(r"C:\");

        assert_eq!(source.volume, PathBuf::from(r"C:\"));
    }

    #[test]
    fn rejects_relative_paths_without_a_drive_letter() {
        let error = volume_root_path(&PathBuf::from("workspace")).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn rejects_unc_network_share_paths() {
        let error = volume_root_path(&PathBuf::from(r"\\server\share\file.txt")).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn reports_an_inaccessible_volume_when_the_filesystem_cannot_be_read() {
        let inspector = FakeVolumeInspector::new("NTFS").with_filesystem_error(io::Error::new(
            io::ErrorKind::NotFound,
            "The system cannot find the path specified",
        ));
        let enumerator = FakeEnumerator::default();
        let source = NtfsMftSource::with_parts(r"E:\", inspector, enumerator.clone());

        let error = collect_initial_error(&source);

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(enumerator.opened(), 0);
    }

    #[test]
    fn reports_unsupported_filesystems_explicitly_without_checking_access() {
        let enumerator = FakeEnumerator::default();
        let source = NtfsMftSource::with_parts(
            r"D:\",
            FakeVolumeInspector::new("FAT32"),
            enumerator.clone(),
        );

        let error = collect_initial_error(&source);

        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert!(error.to_string().contains("FAT32"));
        assert!(!source.inspector.access_checked.get());
        assert_eq!(enumerator.opened(), 0);
    }

    #[test]
    fn reports_an_inaccessible_ntfs_volume_explicitly() {
        let inspector = FakeVolumeInspector::new("NTFS").with_access_error(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Access is denied",
        ));
        let enumerator = FakeEnumerator::default();
        let source = NtfsMftSource::with_parts(r"C:\", inspector, enumerator.clone());

        let error = collect_initial_error(&source);

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(enumerator.opened(), 0);
    }

    #[test]
    fn enumerates_the_volume_only_after_validation_succeeds() {
        let enumerator = FakeEnumerator::with_chunks(vec![volume_tree_chunk()]);
        let source = ntfs_source(r"C:\", enumerator.clone());

        let (entries, issues) = collect_entries(&source);

        assert!(source.inspector.access_checked.get());
        assert_eq!(enumerator.opened(), 1);
        assert_eq!(entries.len(), 3);
        assert!(issues.is_empty());
    }

    #[test]
    fn indexes_active_records_with_identifiers_names_attributes_and_volume_identity() {
        let source = ntfs_source(
            r"C:\",
            FakeEnumerator::with_chunks(vec![volume_tree_chunk()]),
        );

        let (entries, _) = collect_entries(&source);
        let notes = entry(&entries, "notes.txt");

        assert_eq!(notes.metadata.file_id, Some(NOTES_ID));
        assert_eq!(notes.metadata.parent_id, Some(PROJECTS_ID));
        assert_eq!(notes.metadata.attributes, ARCHIVE_ATTRIBUTE);
        assert_eq!(notes.metadata.volume_identity.as_deref(), Some(r"C:\"));
        assert_eq!(notes.display, "notes.txt");
        assert_eq!(
            entry(&entries, "projects").metadata.parent_id,
            Some(ROOT_ID)
        );
    }

    #[test]
    fn keeps_file_reference_sequences_when_resolving_parents() {
        let reused_projects_reference = 0x0003_0000_0000_002a;
        let source = ntfs_source(
            r"C:\",
            FakeEnumerator::with_chunks(vec![chunk(
                0,
                &[
                    FakeRecord::new(PROJECTS_REFERENCE, ROOT_REFERENCE, "projects")
                        .with_attributes(DIRECTORY_ATTRIBUTE),
                    FakeRecord::new(NOTES_REFERENCE, reused_projects_reference, "notes.txt"),
                ],
            )]),
        );

        let (mut entries, _) = collect_entries(&source);
        let issues = resolve_entry_paths(&mut entries);

        assert_eq!(
            entry(&entries, "notes.txt").metadata.parent_id,
            Some(u128::from(reused_projects_reference))
        );
        assert_eq!(
            entry(&entries, "notes.txt").metadata.full_path,
            PathBuf::from("notes.txt")
        );
        assert_eq!(issues.len(), 1);
        assert!(
            issues[0]
                .message
                .contains(&reused_projects_reference.to_string())
        );
    }

    #[test]
    fn anchors_the_enumerated_root_record_to_the_volume_root() {
        let source = ntfs_source(
            r"C:\",
            FakeEnumerator::with_chunks(vec![volume_tree_chunk()]),
        );

        let (entries, _) = collect_entries(&source);
        let root = entry(&entries, r"C:\");

        assert_eq!(root.metadata.file_id, Some(ROOT_ID));
        assert_eq!(root.metadata.parent_id, None);
        assert_eq!(root.metadata.full_path, PathBuf::from(r"C:\"));
        assert_eq!(root.metadata.attributes, DIRECTORY_ATTRIBUTE);
    }

    #[test]
    fn resolves_full_paths_from_the_enumerated_parent_relationships() {
        let source = ntfs_source(
            r"C:\",
            FakeEnumerator::with_chunks(vec![volume_tree_chunk()]),
        );

        let (mut entries, _) = collect_entries(&source);
        let issues = resolve_entry_paths(&mut entries);

        assert!(issues.is_empty(), "unresolved entries: {issues:?}");
        assert_eq!(
            entry(&entries, "notes.txt").metadata.full_path,
            PathBuf::from(r"C:\projects\notes.txt"),
        );
        assert_eq!(entry(&entries, "notes.txt").path, r"C:\projects\notes.txt");
    }

    #[test]
    fn anchors_records_to_the_volume_root_when_no_root_record_is_reported() {
        let source = ntfs_source(
            r"D:\",
            FakeEnumerator::with_chunks(vec![chunk(
                0,
                &[
                    FakeRecord::new(PROJECTS_REFERENCE, ROOT_REFERENCE, "projects")
                        .with_attributes(DIRECTORY_ATTRIBUTE),
                ],
            )]),
        );

        let (mut entries, _) = collect_entries(&source);
        let issues = resolve_entry_paths(&mut entries);

        assert_eq!(entries.len(), 2);
        assert!(issues.is_empty(), "unresolved entries: {issues:?}");
        assert_eq!(entry(&entries, r"D:\").metadata.file_id, Some(ROOT_ID));
        assert_eq!(
            entry(&entries, "projects").metadata.full_path,
            PathBuf::from(r"D:\projects"),
        );
    }

    #[test]
    fn skips_records_reported_as_deleted() {
        let source = ntfs_source(
            r"C:\",
            FakeEnumerator::with_chunks(vec![chunk(
                0,
                &[
                    FakeRecord::new(PROJECTS_REFERENCE, ROOT_REFERENCE, "projects"),
                    FakeRecord::new(NOTES_REFERENCE, PROJECTS_REFERENCE, "removed.txt")
                        .with_reason(DELETE_REASON),
                ],
            )]),
        );

        let (entries, issues) = collect_entries(&source);

        assert!(issues.is_empty());
        assert_eq!(entries.len(), 2);
        assert!(
            entries
                .iter()
                .all(|entry| entry.metadata.name != "removed.txt")
        );
    }

    #[test]
    fn requests_every_chunk_from_the_reported_next_file_reference() {
        let enumerator = FakeEnumerator::with_chunks(vec![
            chunk(
                48,
                &[FakeRecord::new(
                    PROJECTS_REFERENCE,
                    ROOT_REFERENCE,
                    "projects",
                )],
            ),
            chunk(
                96,
                &[FakeRecord::new(
                    NOTES_REFERENCE,
                    PROJECTS_REFERENCE,
                    "notes.txt",
                )],
            ),
        ]);
        let source = ntfs_source(r"C:\", enumerator.clone());

        let (entries, issues) = collect_entries(&source);

        assert_eq!(enumerator.starts(), vec![0, 48, 96]);
        assert_eq!(entries.len(), 3);
        assert!(issues.is_empty());
    }

    #[test]
    fn stops_when_the_next_file_reference_does_not_advance() {
        let enumerator = FakeEnumerator::with_chunks(vec![
            chunk(
                0,
                &[FakeRecord::new(
                    PROJECTS_REFERENCE,
                    ROOT_REFERENCE,
                    "projects",
                )],
            ),
            chunk(
                0,
                &[FakeRecord::new(
                    NOTES_REFERENCE,
                    PROJECTS_REFERENCE,
                    "never-read.txt",
                )],
            ),
        ]);
        let source = ntfs_source(r"C:\", enumerator.clone());

        let (entries, _) = collect_entries(&source);

        assert_eq!(enumerator.starts(), vec![0]);
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn reports_a_record_length_beyond_the_chunk_without_losing_earlier_records() {
        let source = ntfs_source(
            r"C:\",
            FakeEnumerator::with_chunks(vec![chunk(
                0,
                &[
                    FakeRecord::new(PROJECTS_REFERENCE, ROOT_REFERENCE, "projects"),
                    FakeRecord::new(NOTES_REFERENCE, PROJECTS_REFERENCE, "notes.txt")
                        .with_record_length(4096),
                ],
            )]),
        );

        let (entries, issues) = collect_entries(&source);

        assert_eq!(entries.len(), 2);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].path, PathBuf::from(r"C:\"));
        assert!(issues[0].message.contains("4096"));
    }

    #[test]
    fn reports_a_record_length_smaller_than_the_record_header() {
        let source = ntfs_source(
            r"C:\",
            FakeEnumerator::with_chunks(vec![chunk(
                0,
                &[
                    FakeRecord::new(PROJECTS_REFERENCE, ROOT_REFERENCE, "projects"),
                    FakeRecord::new(NOTES_REFERENCE, PROJECTS_REFERENCE, "notes.txt")
                        .with_record_length(8),
                ],
            )]),
        );

        let (entries, issues) = collect_entries(&source);

        assert_eq!(entries.len(), 2);
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn skips_records_with_an_unsupported_version_and_keeps_the_rest() {
        let source = ntfs_source(
            r"C:\",
            FakeEnumerator::with_chunks(vec![chunk(
                0,
                &[
                    FakeRecord::new(NOTES_REFERENCE, PROJECTS_REFERENCE, "future.txt")
                        .with_major_version(4),
                    FakeRecord::new(PROJECTS_REFERENCE, ROOT_REFERENCE, "projects"),
                ],
            )]),
        );

        let (entries, issues) = collect_entries(&source);

        assert_eq!(entries.len(), 2);
        assert!(
            entries
                .iter()
                .all(|entry| entry.metadata.name != "future.txt")
        );
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains('4'));
    }

    #[test]
    fn skips_records_whose_name_extends_past_the_record() {
        let source = ntfs_source(
            r"C:\",
            FakeEnumerator::with_chunks(vec![chunk(
                0,
                &[
                    FakeRecord::new(NOTES_REFERENCE, PROJECTS_REFERENCE, "notes.txt")
                        .with_name_length(512),
                    FakeRecord::new(PROJECTS_REFERENCE, ROOT_REFERENCE, "projects"),
                ],
            )]),
        );

        let (entries, issues) = collect_entries(&source);

        assert_eq!(entries.len(), 2);
        assert!(
            entries
                .iter()
                .all(|entry| entry.metadata.name != "notes.txt")
        );
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn skips_records_whose_name_overlaps_the_record_header() {
        let source = ntfs_source(
            r"C:\",
            FakeEnumerator::with_chunks(vec![chunk(
                0,
                &[
                    FakeRecord::new(NOTES_REFERENCE, PROJECTS_REFERENCE, "notes.txt")
                        .with_name_offset(8),
                    FakeRecord::new(PROJECTS_REFERENCE, ROOT_REFERENCE, "projects"),
                ],
            )]),
        );

        let (entries, issues) = collect_entries(&source);

        assert_eq!(entries.len(), 2);
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn skips_records_with_an_odd_or_empty_name_length() {
        let source = ntfs_source(
            r"C:\",
            FakeEnumerator::with_chunks(vec![chunk(
                0,
                &[
                    FakeRecord::new(NOTES_REFERENCE, PROJECTS_REFERENCE, "notes.txt")
                        .with_name_length(9),
                    FakeRecord::new(NOTES_REFERENCE, PROJECTS_REFERENCE, "").with_name_length(0),
                    FakeRecord::new(PROJECTS_REFERENCE, ROOT_REFERENCE, "projects"),
                ],
            )]),
        );

        let (entries, issues) = collect_entries(&source);

        assert_eq!(entries.len(), 2);
        assert_eq!(issues.len(), 2);
    }

    #[test]
    fn reports_a_chunk_too_short_for_the_next_file_reference() {
        let source = ntfs_source(
            r"C:\",
            FakeEnumerator::with_chunks(vec![
                chunk(
                    48,
                    &[FakeRecord::new(
                        PROJECTS_REFERENCE,
                        ROOT_REFERENCE,
                        "projects",
                    )],
                ),
                vec![0u8; 4],
            ]),
        );

        let (entries, issues) = collect_entries(&source);

        assert_eq!(entries.len(), 2);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("4"));
    }

    #[test]
    fn reports_trailing_bytes_that_cannot_form_a_record_length() {
        let mut malformed_chunk = chunk(
            0,
            &[FakeRecord::new(
                PROJECTS_REFERENCE,
                ROOT_REFERENCE,
                "projects",
            )],
        );
        malformed_chunk.extend_from_slice(&[0xaa, 0xbb, 0xcc]);
        let source = ntfs_source(r"C:\", FakeEnumerator::with_chunks(vec![malformed_chunk]));

        let (entries, issues) = collect_entries(&source);

        assert_eq!(entries.len(), 2);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("trailing"));
    }

    #[test]
    fn fails_instead_of_reporting_an_empty_index_when_no_records_are_enumerated() {
        let source = ntfs_source(r"C:\", FakeEnumerator::default());

        let error = collect_initial_error(&source);

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("no usable records"));
    }

    #[test]
    fn fails_when_every_enumerated_record_is_unusable() {
        let source = ntfs_source(
            r"C:\",
            FakeEnumerator::with_chunks(vec![chunk(
                0,
                &[
                    FakeRecord::new(NOTES_REFERENCE, PROJECTS_REFERENCE, "notes.txt")
                        .with_major_version(4),
                ],
            )]),
        );

        let error = collect_initial_error(&source);

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn propagates_enumeration_failures_from_the_volume() {
        let source = ntfs_source(
            r"C:\",
            FakeEnumerator::with_error(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Access is denied",
            )),
        );

        let error = collect_initial_error(&source);

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn propagates_failures_to_open_the_volume_for_enumeration() {
        struct FailingEnumerator;

        impl MftRecordEnumerator for FailingEnumerator {
            fn open(&self, _volume_root: &Path) -> io::Result<Box<dyn MftEnumerationCursor>> {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Access is denied",
                ))
            }
        }

        let source =
            NtfsMftSource::with_parts(r"C:\", FakeVolumeInspector::new("NTFS"), FailingEnumerator);
        let mut out: Vec<SearchEntry> = Vec::new();
        let mut issues: Vec<IndexIssue> = Vec::new();

        let error = source.collect_initial(&mut out, &mut issues).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(out.is_empty());
    }

    fn unsupported_filesystem_source(
        volume: &str,
        enumerator: FakeEnumerator,
    ) -> NtfsMftSource<FakeVolumeInspector, FakeEnumerator, FakeJournalCheckpointReader> {
        NtfsMftSource::with_parts_and_journal(
            volume,
            FakeVolumeInspector::new("FAT32"),
            enumerator,
            FakeJournalCheckpointReader::Success,
        )
    }

    fn multi_collect(
        source: &NtfsMultiVolumeSource<
            FakeVolumeInspector,
            FakeEnumerator,
            FakeJournalCheckpointReader,
        >,
    ) -> io::Result<(Vec<SearchEntry>, Vec<IndexIssue>)> {
        let mut out: Vec<SearchEntry> = Vec::new();
        let mut issues: Vec<IndexIssue> = Vec::new();
        source.collect_initial(&mut out, &mut issues)?;
        Ok((out, issues))
    }

    #[test]
    fn multi_volume_source_defaults_to_no_selected_volumes() {
        let source: NtfsMultiVolumeSource = NtfsMultiVolumeSource::default();

        assert!(source.sources.is_empty());
    }

    #[test]
    fn multi_volume_source_new_constructs_one_source_per_selected_volume() {
        let source = NtfsMultiVolumeSource::new([r"C:\", r"D:\"]);

        assert_eq!(source.sources.len(), 2);
        assert_eq!(source.sources[0].volume, PathBuf::from(r"C:\"));
        assert_eq!(source.sources[1].volume, PathBuf::from(r"D:\"));
    }

    #[test]
    fn multi_volume_source_fails_when_no_volumes_are_selected() {
        let source: NtfsMultiVolumeSource<
            FakeVolumeInspector,
            FakeEnumerator,
            FakeJournalCheckpointReader,
        > = NtfsMultiVolumeSource::with_sources(Vec::new());

        let error = multi_collect(&source).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn multi_volume_source_indexes_every_selected_volume() {
        let c_drive = ntfs_source(
            r"C:\",
            FakeEnumerator::with_chunks(vec![volume_tree_chunk()]),
        );
        let d_drive = ntfs_source(
            r"D:\",
            FakeEnumerator::with_chunks(vec![chunk(
                0,
                &[
                    FakeRecord::new(ROOT_REFERENCE, ROOT_REFERENCE, ".")
                        .with_attributes(DIRECTORY_ATTRIBUTE),
                    FakeRecord::new(PROJECTS_REFERENCE, ROOT_REFERENCE, "backup.zip")
                        .with_attributes(ARCHIVE_ATTRIBUTE),
                ],
            )]),
        );
        let source = NtfsMultiVolumeSource::with_sources(vec![c_drive, d_drive]);

        let (entries, issues) = multi_collect(&source).unwrap();

        assert!(issues.is_empty());
        assert_eq!(entries.len(), 5);
        assert!(
            entries
                .iter()
                .any(|entry| entry.metadata.name == "notes.txt")
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.metadata.name == "backup.zip")
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.metadata.volume_identity.as_deref() == Some(r"C:\"))
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.metadata.volume_identity.as_deref() == Some(r"D:\"))
        );
    }

    #[test]
    fn multi_volume_source_retains_earlier_successful_volumes_when_a_later_volume_fails() {
        let good_volume = ntfs_source(
            r"C:\",
            FakeEnumerator::with_chunks(vec![volume_tree_chunk()]),
        );
        let bad_volume = unsupported_filesystem_source(r"E:\", FakeEnumerator::default());
        let source = NtfsMultiVolumeSource::with_sources(vec![good_volume, bad_volume]);

        let (entries, issues) = multi_collect(&source).unwrap();

        assert_eq!(entries.len(), 3);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].path, PathBuf::from(r"E:\"));
        assert!(issues[0].message.contains("FAT32"));
    }

    #[test]
    fn multi_volume_source_retains_later_successful_volumes_when_an_earlier_volume_fails() {
        let bad_volume = unsupported_filesystem_source(r"E:\", FakeEnumerator::default());
        let good_volume = ntfs_source(
            r"C:\",
            FakeEnumerator::with_chunks(vec![volume_tree_chunk()]),
        );
        let source = NtfsMultiVolumeSource::with_sources(vec![bad_volume, good_volume]);

        let (entries, issues) = multi_collect(&source).unwrap();

        assert_eq!(entries.len(), 3);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].path, PathBuf::from(r"E:\"));
    }

    #[test]
    fn multi_volume_source_fails_when_every_selected_volume_fails() {
        let first = unsupported_filesystem_source(r"E:\", FakeEnumerator::default());
        let second_inspector = FakeVolumeInspector::new("NTFS").with_access_error(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Access is denied",
        ));
        let second = NtfsMftSource::with_parts_and_journal(
            r"F:\",
            second_inspector,
            FakeEnumerator::default(),
            FakeJournalCheckpointReader::Success,
        );
        let source = NtfsMultiVolumeSource::with_sources(vec![first, second]);

        let mut out: Vec<SearchEntry> = Vec::new();
        let mut issues: Vec<IndexIssue> = Vec::new();
        let error = source.collect_initial(&mut out, &mut issues).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(out.is_empty());
        assert_eq!(issues.len(), 2);
        assert!(error.to_string().contains(r"E:\"));
        assert!(error.to_string().contains(r"F:\"));
    }

    #[test]
    fn multi_volume_source_deduplicates_equivalent_selected_volume_roots() {
        let first_enumerator = FakeEnumerator::with_chunks(vec![volume_tree_chunk()]);
        let second_enumerator = FakeEnumerator::with_chunks(vec![volume_tree_chunk()]);
        let first = ntfs_source(r"C:\", first_enumerator.clone());
        let second = ntfs_source(r"C:\Users\test", second_enumerator.clone());
        let source = NtfsMultiVolumeSource::with_sources(vec![first, second]);

        let (entries, issues) = multi_collect(&source).unwrap();

        assert!(issues.is_empty());
        assert_eq!(entries.len(), 3);
        assert_eq!(first_enumerator.opened(), 1);
        assert_eq!(second_enumerator.opened(), 0);
    }
}
