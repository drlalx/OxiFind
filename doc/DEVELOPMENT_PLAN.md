# Development Plan

## Goal

Build a Windows desktop file-name search tool in Rust that provides fast,
interactive results from a maintained local index. The implementation should
remain independent and should not reuse code or proprietary data from
external search tools.

## Principles

- Keep indexing, searching, and presentation separate.
- Prefer correct, measurable behavior over premature optimization.
- Keep the UI responsive by moving blocking work off the UI thread.
- Treat inaccessible volumes, files, and journal records as recoverable errors.
- Add deterministic tests for every non-trivial indexing and search behavior.

## Milestone 1: Stabilize the Prototype

**Objective:** Make the current workspace-based search reliable and testable.

1. [x] Define `SearchEntry` invariants and add unit tests for sorting, matching,
   ranking, result limits, empty queries, and inaccessible directories.
2. [x] Replace recursive traversal with an iterator that handles per-entry I/O
   failures without losing the complete index.
3. [x] Display the indexed root, indexing duration, and recoverable indexing
   errors in the UI.
4. [x] Add a command-line option for the index root instead of implicitly using
   the current directory.

**Progress:** Initial indexing now runs without blocking the UI. Workspace
traversal, relative paths, build-metadata exclusions, and inaccessible child
directories have filesystem-level coverage.

**Exit criteria:** A selected directory can be indexed without UI blocking, and
search behavior is covered by automated tests.

## Milestone 2: Establish the Index Model

**Objective:** Support richer Windows file metadata independently of its source.

1. [x] Extend indexed entries with a stable file identifier, parent identifier,
   name, full path, volume identity, and relevant file attributes.
2. [x] Define an `IndexSource` contract that supports an initial snapshot and
   incremental updates.
3. [x] Build paths from parent-child relationships and handle orphaned or
   deleted records safely.
4. [x] Introduce benchmark fixtures for small, medium, and large synthetic
   indexes.

**Exit criteria:** The search index can be constructed from either the existing
workspace walker or a Windows-specific metadata source without changing the UI.

## Milestone 3: Implement NTFS Initial Indexing

**Objective:** Populate the index quickly from NTFS metadata.

1. [x] Implement `NtfsMftSource` using supported Windows APIs or documented NTFS
   access methods, with explicit permission and volume-format checks.
2. [x] Enumerate active MFT records, extract names and parent relationships,
   and build a consistent volume index.
3. [x] Support multiple selected NTFS volumes and report unsupported volumes
   clearly.
4. [x] Compare MFT-derived results with the workspace walker on test directories.

**Progress:** `NtfsMftSource` validates the selected volume before any MFT
access is attempted: it resolves the volume's drive root, reads its reported
filesystem name via `GetVolumeInformationW`, and confirms the current process
can open the volume for raw read access via `CreateFileW`, using the `windows`
crate. Non-NTFS filesystems, inaccessible volumes, and paths without a local
drive letter (relative paths, UNC shares) each return an explicit `io::Error`
with no silent fallback. Validated volumes are then enumerated with the
documented `FSCTL_ENUM_USN_DATA` device control, behind an injectable
record-enumeration seam that lets parsing and chunk continuation be tested
without a real volume. Every chunk is parsed defensively: record lengths,
record versions, and file-name ranges are bounds-checked, unusable records
become recoverable indexing issues instead of panics, records reported as
deleted are skipped, and enumeration stops when the reported next file
reference number does not advance. Each active record becomes an entry
carrying its complete NTFS file reference number as file identifier, its
parent file reference number,
its name, its file attributes, and the volume identity; the root directory
record anchors them to the validated volume root so the shared parent-child
resolver rebuilds full paths. An enumeration that yields no usable record
fails explicitly rather than reporting an empty index. `NtfsMultiVolumeSource`
composes several single-volume sources into one snapshot: every selected
volume is validated and enumerated independently, a volume that fails does
not discard entries already collected from the others and instead becomes a
recoverable `IndexIssue` tied to its own path, equivalent selected roots (for
example `C:\` and `C:\Users`) are indexed only once, and `collect_initial`
returns an explicit error only when no volume was selected or every selected
volume failed. A deterministic cross-source test builds a real temporary
workspace tree and replays matching synthetic MFT records through the existing
enumeration seam; both sources yield the expected representative nested paths
and names after shared reconstruction. The synthetic replay cannot validate
live MFT contents or elevated raw-volume access, so that behavior still
requires an elevated run against a known NTFS volume.

**Exit criteria:** A selected NTFS volume produces a usable initial index
without recursively walking every directory.

## Milestone 4: Maintain the Index

**Objective:** Reflect file-system changes without a full reindex.

1. [x] Record the USN journal position associated with each completed volume index.
2. [x] Read and apply create, rename, move, and delete events in order.
3. [x] Detect journal resets, wraps, and gaps; mark affected volumes stale and
   rebuild them.
4. [x] Process updates on background workers and publish consistent snapshots to
   the search UI.

**Exit criteria:** File changes on supported NTFS volumes appear or disappear in
search results without restarting the application.

**Progress:** Each successful MFT volume snapshot now retains its USN journal
identifier and next-USN checkpoint. The checkpoint is queried through an
injectable reader using `FSCTL_QUERY_USN_JOURNAL` in production, retained by
the source-independent `SearchIndex`, and published only with the matching
successful volume snapshot. A failed checkpoint query produces a recoverable
indexing issue and does not publish that volume as current.

Changes after each checkpoint are read with the documented
`FSCTL_READ_USN_JOURNAL` control code behind an injectable seam and applied in
journal order: creates and new-name records add or replace entries, deletes
remove them, and old-name records are skipped. Entries are keyed by volume and
file identifier, and paths are re-resolved after every batch, so moving or
renaming a directory also moves its descendants. A refresh only commits its
updates and advanced checkpoints when every volume's journal could be read.
The update loop is not yet connected to the UI.

Before reading changes, each volume's checkpoint is compared with the current
journal state. A new journal identifier (reset), a checkpoint older than the
first readable record (wrap), or a checkpoint past the journal's end (gap)
marks the volume stale; it is then re-enumerated and its entries are replaced
as a whole, and the rebuild and its reason are reported as an indexing issue.
Other volumes keep their entries.

A drive-root argument now starts the NTFS source in the application. After the
initial snapshot, a background poller collects an `UpdateBatch` once per
second against its own copy of the checkpoints and sends non-empty batches to
the search worker, which is the only owner of the index. The worker applies a
batch in one step, then re-runs the latest query against the new snapshot, so
every response the UI receives reflects exactly one index state. Failed
collections leave the checkpoints unchanged and are shown in the status line.

## Milestone 5: Improve Search and User Experience

**Objective:** Make interactive searching useful at scale.

1. [x] Add tokenized matching, prefix matching, path matching, and exact-match
   filters with documented syntax.
2. [x] Preserve deterministic ranking and add metadata-based sorting options.
3. [x] Show full paths and key metadata in results; provide open-file and
   open-containing-folder actions.
4. [x] Add cancellation or request coalescing so stale queries never update the UI.
5. [x] Measure query latency and indexing memory usage against the benchmark
   fixtures.
6. [x] Replace the Slint UI with an eframe/egui UI following
   [the UI mockup](mockup/ui-mockup.html): menu bar, search field, result
   table with sortable columns, row context menu, and status bar.

**Exit criteria:** Common queries update predictably while typing and expose
enough context for users to select a result.

**Progress:** All items in Milestone 5 are complete. Queries are split into
whitespace-separated terms that must all match. `^text`, `=text`, and `path:text`
restrict a term to file-name prefixes, whole file names, or full paths.
`sort:relevance`, `sort:name`, `sort:path`, and `sort:type` order the results.
The eframe/egui UI and the ratatui TUI provide interactive, responsive searching.
Worker command queues drain superseded search requests when typing rapidly,
ensuring that stale intermediate requests never block the search worker or lag the UI.
Search scoring uses pre-computed lowercase search cache (`SearchData`), non-allocating
case-insensitive string comparisons, Top-K partition selection (`select_nth_unstable_by`),
and multi-threaded chunk filtering (`std::thread::scope`). Query latency on 500,000
entries was benchmarked and reduced by 16x from ~350 ms down to ~21 ms.

## Milestone 6: Persistence, Packaging, and Operations

**Objective:** Make the application practical outside development.

1. [x] Persist index snapshots and journal checkpoints atomically per volume.
2. [x] Validate persisted data and rebuild safely after corruption or version
   changes.
3. [x] Add application settings for indexed volumes, excluded paths, and result
   limits.
4. [ ] Package a signed Windows executable and document installation, permissions,
   diagnostics, and uninstallation.
5. [x] Add integration tests for first-run indexing, restart, journal recovery,
   and unsupported volumes.

**Progress:** Items 1, 2, 3, and 5 are complete. An atomic persistence module
(`src/persistence.rs`) serializes index snapshots and journal checkpoints with
a binary header (`OXIFIND1`), version tracking, payload length, and CRC32
integrity checksums. Snapshots are written via atomic temporary files and renames
to eliminate partial or broken writes. Corruption or version mismatches are safely
caught and handled without panicking. Platform-specific application data directories
and JSON application settings (`AppSettings`) support configurable indexed volumes,
pattern exclusions, and result limits. Dedicated integration tests verify roundtrips,
tamper detection, index recreation, continuous USN journal updates, journal wrap/purge
gap detection and full volume rebuilds, USN journal recreation recovery, invalid checkpoint
corruption handling, explicit non-silent rejection of unsupported filesystems (FAT32/exFAT),
permission-denied volume isolation, and multi-volume partial failure resilience across 9 test suites
with 171 passed tests.

**Exit criteria:** The application can be installed, restarted, and recovered
without requiring manual index management.

## Suggested Delivery Order

1. Complete Milestone 1 before changing the public search behavior.
2. Introduce the index model in Milestone 2 before implementing NTFS access.
3. Deliver initial MFT indexing before USN journal processing.
4. Add persistence only after the in-memory index and update model are stable.

## Out of Scope for the Initial Releases

- File-content indexing.
- Network-share indexing.
- Cloud synchronization.
- Compatibility with external search tools and their data formats.
- Non-Windows file-system metadata sources beyond the existing directory walker.
