# Project Memory

## Current implementation status

- `oxifind` accepts an optional `INDEX_ROOT` command-line argument, parsed
  into `app::IndexScope`.
- Without `INDEX_ROOT` (`IndexScope::LocalDrives`), the worker indexes every
  local fixed drive (`ntfs::local_fixed_volumes`: `GetLogicalDrives` +
  `GetDriveTypeW == DRIVE_FIXED`) through `NtfsMultiVolumeSource`. This needs
  administrator rights; `elevation::is_elevated` checks the token, and when
  indexing fails without elevation the central panel and the File menu offer
  `elevation::restart_elevated` (`ShellExecuteW` "runas" with re-quoted
  arguments), then close the window.
- More than one positional argument is rejected with usage information.
- `app::run` receives the resolved index root from `main`.
- Search entries and equally ranked results are ordered case-insensitively with
  deterministic display-name and path tie-breakers.
- `SearchIndex::search_results` returns `SearchResult`s (display, full path,
  path-resolution status, attributes) and is what the app uses; the test-only
  `search` returns just the displays. Both share `matching_entries`. The egui UI (`SearchApp`,
  eframe 0.36 `App::logic`/`App::ui`) shows each result as a `ResultRow`
  (name, containing folder, `describe_attributes` type, full path) in an
  `egui_extras::TableBuilder`; double-click opens, the row context menu and
  the Edit/Search menus run `RowAction`s (open, reveal, copy name/path).
  Header clicks toggle a `ResultSort`, which `effective_query` prefixes as a
  `sort:` term so a typed `sort:` term still wins. The worker sends through
  `ResponseSender`, which calls `ctx.request_repaint()` after every response;
  `SearchApp::apply_response` ignores stale revisions and keeps the selection
  by path. `open_in_explorer` spawns
  `explorer.exe` with a raw, quoted absolute path (`/select,` for reveal),
  rejects paths containing quotes, and ignores Explorer's exit code; failures
  go to the status bar notice. `ResultRow::action_path` rejects unresolved
  index paths before any Explorer action, including previously absolute
  paths whose parent records were removed. Search and copy actions retain
  those records; valid relative workspace paths still resolve against the
  working directory.
- The query parser behind it uses `parse_query`/`split_terms`:
  whitespace outside double quotes separates terms, quotes are removed, and
  modifiers are only recognized before a quote (`path:` case-insensitive,
  then `^`, then `=`). Terms without text are dropped; no terms lists the
  first entries. Every term must match; `score_term` ranks against
  pre-lowercased `metadata.name` and `path` stored in `SearchData`: name prefix 0,
  name contains 1, path contains 2 (`^` only 0, `=` only whole name, `path:` only path).
  The entry score is the sum over terms. Parallelized chunk filtering (`std::thread::scope`),
  zero-allocation case-insensitive comparisons, and Top-K selection (`select_nth_unstable_by`)
  ensure near-instant filtering on large indexes.
- Workers in `app.rs` and `engine.rs` drain queued search commands, skipping stale
  requests when typing rapidly so the search worker always evaluates only the latest revision.
- `parse_query` returns a `Query` with terms and a `SortOrder`. A `sort:`
  term (case-insensitive, before quotes) selects `SortKey::Relevance`,
  `Name`, `Path`, or `Type` (directory attribute first), reversed by a
  leading `-`; the last valid one wins, an empty value is dropped, and an
  unknown key stays an ordinary contains term. Results sort by the key, then
  score, then `compare_entries`; only a query without terms and with the
  default order takes the unsorted fast path.
- Workspace indexing uses iterative directory traversal. Errors on child paths
  are retained as indexing issues without discarding successfully indexed
  entries; an inaccessible root remains a startup error. A Windows-only test
  denies listing on a child directory via `icacls` (Everyone deny ACE, removed
  on drop) and asserts the remaining entries plus exactly one issue for it.
- The startup status reports the selected index root, elapsed indexing time,
  indexed item count, and recoverable indexing issues.
- Initial indexing runs in a background worker. Search requests entered while
  it runs are queued and processed after the index is ready.
- `app::run` selects `NtfsMftSource` when the index root is exactly a drive
  root (`is_volume_root`, e.g. `C:\` or `\\?\C:\`), otherwise
  `WorkspaceSource`; NTFS failures are reported, never replaced by walking.
  The search worker is the only owner of `SearchIndex` and handles
  `WorkerCommand`s (`Search`, `ApplyUpdates(UpdateBatch)`, `UpdateFailed`)
  in order through `handle_command`, remembering the latest request. After
  the initial snapshot, the source moves to an update-poller thread that
  calls `poll_updates` every `UPDATE_POLL_INTERVAL` (1 s) against its own
  checkpoint copy: it stops for `Unsupported` sources, advances checkpoints
  only on success, skips empty batches, and sends failures as
  `UpdateFailed`. Applying a batch answers with `IndexUpdated`, containing
  counts and the latest query re-run on the new snapshot; the UI shows those
  results only if no newer query revision was issued.
- `UpdateBatch::collect(source, checkpoints)` gathers updates, issues, and
  advanced checkpoints without touching an index; `SearchIndex::apply`
  applies one batch in order and adopts its checkpoints. `refresh` combines
  both.
- `SearchEntry` now retains source-independent `EntryMetadata`: optional file
  and parent identifiers, name, full path, optional volume identity, and file
  attributes. The workspace source supplies name, full path, and the
  directory attribute for directories.
- `IndexSource` separates `collect_initial` from `collect_updates`. Sources
  without change tracking report `Unsupported`; `IndexUpdate` represents
  upserts and removals by volume identity and file identifier.
  `collect_snapshot` supplies initial entries and matching checkpoints
  together; its default captures checkpoints before initial collection.
  NTFS sources validate first, read the journal state before MFT enumeration,
  and publish only successfully completed volume snapshots.
  `collect_updates(checkpoints, out, issues)` appends changes after each
  checkpoint in order and advances the checkpoints it read.
- Snapshot construction resolves full paths from parent-child identifiers.
  Identifiers are scoped to their volume identity, because NTFS file
  references (including the root, record 5) repeat across volumes.
  Missing parent records and cycles retain their supplied paths and become
  recoverable indexing issues. `SearchEntry::path_resolved` is recomputed
  after updates and passed through `SearchResult` to `ResultRow`, so these
  records cannot open unrelated working-directory files. Restoring a parent
  also restores the resolved status.
- `SearchIndex::refresh(source)` collects updates against a copy of its
  checkpoints and applies them in order only when collection succeeds;
  otherwise entries and checkpoints stay unchanged. Upserts replace or add
  entries keyed by volume and file identifier, removals drop them, then all
  paths are re-resolved (so directory renames move descendants) and entries
  re-sorted. Path-resolution issues are kept separately from source issues
  and recomputed on every refresh, so resolved orphans stop being reported.
- `NtfsMftSource` validates a selected volume before any MFT access:
  `VolumeInspector` (a `WindowsVolumeInspector` by default, injectable for
  tests) reads the volume's filesystem name via `GetVolumeInformationW` and
  confirms raw read access via `CreateFileW` against the `\\.\<drive>:` device
  path, both from the `windows` crate. Non-NTFS filesystems, inaccessible
  volumes, and paths without a local drive letter each return an explicit
  `io::Error` (`Unsupported`, the underlying OS error kind, or `InvalidInput`)
  with no silent fallback.
- Once validation succeeds, `NtfsMftSource` enumerates the volume's active
  master file table records with the documented `FSCTL_ENUM_USN_DATA` device
  control (`DeviceIoControl` with `MFT_ENUM_DATA_V0`) instead of walking
  directories. `MftRecordEnumerator`/`MftEnumerationCursor` are the seam
  around that native call: `WindowsMftEnumerator` opens the volume once and
  requests one chunk per call, and tests replay synthetic chunks.
- Each chunk starts with the next start file reference number followed by
  `USN_RECORD_V2` records. Parsing is defensive: record lengths below the
  60-byte header or past the chunk end stop the chunk, unsupported record
  versions and file-name ranges that are odd, empty, out of range, or
  overlapping the header skip the record, and every defect becomes a
  recoverable `IndexIssue` instead of a panic. Enumeration also stops when
  the reported next file reference number does not advance.
- Enumerated entries carry the complete NTFS file reference number (including
  its reuse sequence) as `file_id`, and the complete parent file reference as
  `parent_id`, plus the record name, file attributes, and drive root as volume
  identity. The root directory (record index 5) is anchored to the validated
  volume root with no parent, and is
  synthesized from that volume root when the enumeration does not report it,
  so `resolve_entry_paths` rebuilds full paths. Records reported as deleted
  are skipped, and an enumeration without usable records fails with
  `InvalidData` instead of reporting an empty index.
- A deterministic cross-source test builds a real temporary workspace tree and
  replays matching synthetic MFT records through `MftRecordEnumerator`. It
  asserts that both sources expose the expected representative nested paths
  and names after the shared resolver reconstructs the MFT display paths. The
  replay deliberately avoids a live raw volume, so it cannot validate actual
  MFT contents or the elevated permission required for raw-volume access.
- `NtfsMultiVolumeSource<I, E, J>` composes several `NtfsMftSource<I, E, J>`
  instances into a single `IndexSource`. `new(volumes)` builds one production
  source per requested volume with the real Windows APIs; `with_sources`
  accepts already-constructed sources in tests, each with its own injected
  `VolumeInspector`/`MftRecordEnumerator`/`JournalCheckpointReader`.
  `Default` creates an empty multi-volume selection. In
  `collect_initial`, each selected volume is validated and enumerated through
  its own `NtfsMftSource::collect_snapshot` in selection order: a volume whose
  resolved drive root (via the existing `volume_root_path` parser) was
  already indexed is skipped as a duplicate selection before any validation
  or enumeration is attempted; a volume that fails is recorded as an
  `IndexIssue` whose `path` is that volume's own selected path (not
  necessarily its resolved root) and whose message names the volume and
  wraps the underlying error, while entries and issues already collected from
  other volumes are kept and collection continues. `collect_initial` returns
  an explicit `InvalidInput` error, with no entries appended, only when no
  volume was selected or every selected volume failed; the error message
  lists every failed volume's path.
- `JournalCheckpoint` is source-independent snapshot state containing a volume
  identity, USN journal identifier, and next-USN position. `SearchIndex`
  retains all checkpoints supplied by its source. `JournalCheckpointReader`
  keeps the NTFS query injectable for deterministic tests; the production
  reader uses `FSCTL_QUERY_USN_JOURNAL` and `USN_JOURNAL_DATA_V0`. A
  multi-volume source only publishes a volume after its MFT snapshot and
  checkpoint both succeed; a checkpoint failure becomes an indexing issue and
  leaves that volume out of the published snapshot. Initial single- and
  multi-volume snapshots retain the state from before MFT enumeration, so
  changes during enumeration are replayed rather than skipped.
- `JournalCheckpointReader::read_changes` is the seam around the documented
  `FSCTL_READ_USN_JOURNAL` (`READ_USN_JOURNAL_DATA_V0`, non-blocking with
  `BytesToWaitFor = 0`, reason mask limited to create, delete, and both rename
  reasons plus `BASIC_INFO_CHANGE`). `NtfsMftSource::collect_updates` reads
  chunks for its own volume's checkpoint until the reported next USN no
  longer advances; records are
  parsed with the same defensive `USN_RECORD_V2` parser, defects become
  issues, and a chunk too short for the next USN fails with `InvalidData`.
  Deletes become removals; creates, new-name records, and attribute-only
  changes become upserts. Updates to the root record keep the validated
  volume-root anchor. Old-name records are skipped even when combined with
  attribute changes. `NtfsMultiVolumeSource::collect_updates`
  delegates once per distinct selected volume; any volume's read error fails
  the whole refresh. The app's background poller applies these updates and
  refreshes the current results.
- `JournalCheckpointReader::read_state` returns a `JournalState` (journal ID,
  first readable USN, next USN) from `FSCTL_QUERY_USN_JOURNAL`; checkpoints
  are derived from it. Before reading changes, `NtfsMftSource` compares the
  checkpoint with the current state: a different journal ID (reset), a
  checkpoint before the first readable USN (wrap/purge), or a checkpoint past
  the next USN (gap) marks the volume stale. A stale volume is re-enumerated
  via `collect_initial`, emitted as `IndexUpdate::ReplaceVolume`, reported as
  an issue naming the reason, and its checkpoint is reset to the state read
  before enumeration so concurrent changes are replayed later. The search
  index drops every entry of that volume and inserts the new snapshot. A
  failed rebuild fails the refresh, so the next refresh detects the stale
  volume again and retries.
- Multiplatform foundations (Windows, Linux, macOS):
  - Enabled strict Clippy no-panic policy (`panic`, `unwrap_used`, `expect_used`,
    `panic_in_result_fn`, `unimplemented`, `todo`, `unreachable` set to `deny`)
    in `Cargo.toml`. Test code allows unwraps and asserts via `#![cfg_attr(test, ...)]`.
  - Enforced MIT-compatible dependency license policy via `deny.toml` and automated
    integration test `tests/license_compliance.rs` covering all 430 crates.
  - Made path parsing, path resolution (`join_child_path`), filename extraction
    (`path_file_name`), and result row splitting (`split_result_path`) completely
    cross-platform for Windows, Linux, and macOS.
  - Harmonized volume root parsing (`volume_root_path`, `is_volume_root`) across
    platforms.
    - The complete test suite (195 tests across 9 unit and integration test suites)
    passes on Linux, with 0 Clippy errors and 0 warnings under the strict
    no-panic policy.
  - Multiplatform abstraction layer (`src/platform/`):
    - `desktop.rs`: `open_path` (Open & Reveal) via Windows Explorer (`explorer.exe /select,`), Linux FreeDesktop DBus (`org.freedesktop.FileManager1.ShowItems` with fallback to `xdg-open`), and macOS (`open` and `open -R`).
    - `elevation.rs`: `is_elevated` and `restart_elevated` via Windows UAC `TOKEN_ELEVATION`/`runas`, Linux `libc::geteuid() == 0`/`pkexec`, and macOS `osascript with administrator privileges`.
    - `mounts.rs`: Mount detection via Linux `/proc/mounts`, macOS `/Volumes`, Linux `$HOME`-preferred `default_linux_roots_from_home` (falling back to `/`), and expanded system path exclusions (`/proc`, `/sys`, `/dev`, `/run`, `/tmp`, `/var/cache`, `/var/lib/docker`, `/var/lib/containers`, `/timeshift`, `/lost+found`, etc.).
  - Native multi-threaded Unix and directory indexing (`src/crawler.rs`, `src/watcher.rs`, `src/engine.rs`, `src/persistence.rs`, `src/platform/service.rs`):
    - Multi-threaded work-stealing crawler (`DirectorySource`) indexing local drives or targeted directories with `with_excluded_patterns` honoring `AppSettings::excluded_patterns` (defaults: `target`, `.git`, `node_modules`, `.cache`, `.stversions`, `__pycache__`, `.venv`).
    - Filtered Linux `DirectoryWatcher` using `RecursiveMode::NonRecursive` per allowed directory, dynamic registration/pruning of nested subdirectories, and `default_watch_budget` (50% of `/proc/sys/fs/inotify/max_user_watches`, default 32,768) stopping cleanly with a single `IndexIssue` on limit exhaustion instead of spamming watches or starving Syncthing/Tracker.
    - Wired `load_runtime_settings()` and `build_directory_source()` into both worker (`src/app.rs`/`src/engine.rs`) and daemon (`run_service`), added `SnapshotSavePolicy` debouncing (`15s` quiet delay, `60s` min interval, `300s` max dirty age) and compile-time `CRC32_TABLE`, and added `install_service_for_scope` so `oxifind service install [ROOT]` pins the systemd `ExecStart` root.
    - Captures file attributes across platforms (Directory, ReparsePoint/Symlink, ReadOnly, Hidden dotfiles).
    - Immune to symlink recursion loops and gracefully captures `EACCES` permission denied errors as recoverable `IndexIssue`s.
    - Wired into `src/app.rs` for `IndexScope::LocalDrives` and `IndexScope::Root` across Windows, Linux, and macOS.

