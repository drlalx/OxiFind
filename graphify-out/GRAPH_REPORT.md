# Graph Report - OxiFind  (2026-11-09)

## Corpus Check
- 47 files · ~52,964 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 8 file(s) not represented in the graph (top: (none) 3, .toml 1, .desktop 1)

## Summary
- 1183 nodes · 2495 edges · 64 communities (45 shown, 19 thin omitted)
- Extraction: 99% EXTRACTED · 1% INFERRED · 0% AMBIGUOUS · INFERRED: 31 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `27f738fa`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- search.rs
- SearchApp
- app.rs
- engine.rs
- watcher.rs
- SearchIndex
- SearchEntry
- ntfs_source
- WorkerResponse
- NtfsMftSource
- SortKey
- Development Plan
- OxiFind
- ntfs.rs
- Development Burndown
- Project Memory
- oxifind
- persistence.rs
- Code-Review des aktuellen Stands (2026-11-09)
- desktop.rs
- ui.rs
- OxiFind Operations & Deployment Guide
- IndexIssue
- crawler.rs
- test
- DirectorySource
- SortColumn
- terminal.rs
- TuiApp
- FakeEnumerator
- 5. Implementierungs-Meilensteine
- FakeRecord
- IndexScope
- mounts.rs
- JournalState
- tui_app_test.rs
- install.sh
- OxiFind
- .from_source
- 4. Geplante Meilensteine (TDD)
- uninstall.sh
- run_search_worker
- journal_index
- 1. Detaillierte Analyse der einzelnen Projekte
- src
- .new
- tui/app.rs
- rust-programmer.agent.md
- test_journal_recovery.rs
- service.rs
- pathbuf
- main.rs
- poll_updates
- macos/install.sh
- macos/uninstall.sh

## God Nodes (most connected - your core abstractions)
1. `SearchEntry` - 59 edges
2. `IndexIssue` - 42 edges
3. `JournalCheckpoint` - 41 edges
4. `TuiApp` - 41 edges
5. `SearchApp` - 36 edges
6. `ntfs_source()` - 32 edges
7. `SearchIndex` - 28 edges
8. `IndexSource` - 26 edges
9. `IndexUpdate` - 25 edges
10. `collect_entries()` - 25 edges

## Surprising Connections (you probably didn't know these)
- `test_worker_cold_start_and_recovery_workflow()` --calls--> `spawn_search_worker_with_snapshot()`  [INFERRED]
  tests/test_persistence.rs → src/engine.rs
- `test_first_run_snapshot_and_cold_start_recovery()` --calls--> `load_snapshot()`  [INFERRED]
  tests/test_journal_recovery.rs → src/persistence.rs
- `test_load_non_existent_snapshot_returns_none()` --calls--> `load_snapshot()`  [INFERRED]
  tests/test_persistence.rs → src/persistence.rs
- `test_first_run_snapshot_and_cold_start_recovery()` --calls--> `save_snapshot()`  [INFERRED]
  tests/test_journal_recovery.rs → src/persistence.rs
- `sample_entries()` --references--> `SearchEntry`  [EXTRACTED]
  tests/test_persistence.rs → src/search.rs

## Import Cycles
- 1-file cycle: `src/app.rs -> src/app.rs`
- 1-file cycle: `src/crawler.rs -> src/crawler.rs`
- 1-file cycle: `src/ntfs.rs -> src/ntfs.rs`
- 1-file cycle: `src/platform/desktop.rs -> src/platform/desktop.rs`
- 1-file cycle: `src/search.rs -> src/search.rs`
- 1-file cycle: `src/platform/mounts.rs -> src/platform/mounts.rs`
- 1-file cycle: `src/watcher.rs -> src/watcher.rs`
- 1-file cycle: `src/main.rs -> src/main.rs`
- 1-file cycle: `src/platform/elevation.rs -> src/platform/elevation.rs`
- 1-file cycle: `src/platform/service.rs -> src/platform/service.rs`
- 2-file cycle: `src/crawler.rs -> src/watcher.rs -> src/crawler.rs`

## Communities (64 total, 19 thin omitted)

### Community 0 - "search.rs"
Cohesion: 0.09
Nodes (28): breaks_sort_key_ties_by_relevance(), builds_child_paths_from_parent_records(), caret_terms_match_only_file_name_prefixes(), compare_case_insensitive(), compare_entries(), compare_text(), DeniedListing, equals_terms_match_whole_file_names_only() (+20 more)

### Community 1 - "SearchApp"
Cohesion: 0.15
Nodes (9): RowAction, CopyFolderPath, CopyName, CopyPath, Open, SearchApp, ServiceEvent, Finished (+1 more)

### Community 2 - "app.rs"
Cohesion: 0.06
Nodes (20): APP_NAME, default_graphics_backend(), describe_attributes(), ensure_linux_graphics_backend(), FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_SYSTEM (+12 more)

### Community 3 - "engine.rs"
Cohesion: 0.05
Nodes (46): displays(), effective_query(), FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_SYSTEM (+38 more)

### Community 4 - "watcher.rs"
Cohesion: 0.15
Nodes (3): detects_file_creation_and_deletion_events(), DirectoryWatcher, TestDir

### Community 5 - "SearchIndex"
Cohesion: 0.17
Nodes (8): build_search_data(), is_directory(), parse_query(), SearchData, SearchIndex, split_terms(), UpdateBatch, upsert_slot()

### Community 6 - "SearchEntry"
Cohesion: 0.12
Nodes (19): entry(), WorkerResult, PersistedSnapshot, CheckpointSource, collect_directories(), directory(), entry(), filters_large_index_efficiently() (+11 more)

### Community 7 - "ntfs_source"
Cohesion: 0.20
Nodes (24): anchors_records_to_the_volume_root_when_no_root_record_is_reported(), anchors_the_enumerated_root_record_to_the_volume_root(), collect_entries(), entry(), enumerates_the_volume_only_after_validation_succeeds(), fails_when_every_enumerated_record_is_unusable(), indexes_active_records_with_identifiers_names_attributes_and_volume_identity(), keeps_file_reference_sequences_when_resolving_parents() (+16 more)

### Community 8 - "WorkerResponse"
Cohesion: 0.14
Nodes (15): format_index_status(), format_indexing_status(), format_update_status(), ignores_results_for_superseded_queries(), keeps_the_indexing_failure_when_the_worker_disconnects(), keeps_the_selected_path_selected_across_result_updates(), rejects_unresolved_index_entries_when_preparing_opening_paths(), result() (+7 more)

### Community 9 - "NtfsMftSource"
Cohesion: 0.13
Nodes (20): collect_initial_error(), fails_instead_of_reporting_an_empty_index_when_no_records_are_enumerated(), FakeVolumeInspector, multi_collect(), multi_volume_source_fails_when_every_selected_volume_fails(), multi_volume_source_fails_when_no_volumes_are_selected(), multi_volume_source_retains_earlier_successful_volumes_when_a_later_volume_fails(), new_uses_the_default_windows_volume_components() (+12 more)

### Community 10 - "SortKey"
Cohesion: 0.14
Nodes (15): Query, QueryTerm, score_entry(), score_term(), SortKey, Name, Path, Relevance (+7 more)

### Community 11 - "Development Plan"
Cohesion: 0.17
Nodes (11): Development Plan, Goal, Milestone 1: Stabilize the Prototype, Milestone 2: Establish the Index Model, Milestone 3: Implement NTFS Initial Indexing, Milestone 4: Maintain the Index, Milestone 5: Improve Search and User Experience, Milestone 6: Persistence, Packaging, and Operations (+3 more)

### Community 12 - "OxiFind"
Cohesion: 0.20
Nodes (9): Architecture, Current functionality, Current limitations, OxiFind, Planned direction, Quality & Architecture Policies, Running the application, Search syntax (+1 more)

### Community 13 - "ntfs.rs"
Cohesion: 0.03
Nodes (48): ALPHA_ID, ALPHA_REFERENCE, ARCHIVE_ATTRIBUTE, assert_snapshot_rename_is_replayed(), BASIC_INFO_CHANGE_REASON, BRAVO_REFERENCE, CLOSE_REASON, CREATE_REASON (+40 more)

### Community 17 - "persistence.rs"
Cohesion: 0.08
Nodes (27): AppSettings, compute_crc32(), CURRENT_VERSION, default_data_dir(), default_settings_path(), default_snapshot_path(), HEADER_SIZE, load_settings() (+19 more)

### Community 18 - "Code-Review des aktuellen Stands (2026-11-09)"
Cohesion: 0.33
Nodes (5): Code-Review des aktuellen Stands (2026-11-09), Hoch, Mittel, Niedrig, Status (nach Umsetzung)

### Community 19 - "desktop.rs"
Cohesion: 0.08
Nodes (23): containing_folder(), explorer_argument(), file_uri(), open_in_explorer(), open_in_finder(), open_in_linux_desktop(), open_path(), open_with_dialog() (+15 more)

### Community 20 - "ui.rs"
Cohesion: 0.09
Nodes (19): centered_rect(), COLOR_ACCENT, COLOR_BG, COLOR_BORDER, COLOR_BORDER_FOCUS, COLOR_CARD, COLOR_DANGER, COLOR_DIM (+11 more)

### Community 21 - "OxiFind Operations & Deployment Guide"
Cohesion: 0.06
Nodes (30): 1. Architecture & Execution Modes, 2. Installation & Distribution, 3. Permissions & Security Model, 4. Persistent Storage & Configuration, 5. Background Service & Daemon Mode (Syncthing-like), 5. Diagnostics & Operational Recovery, 6. Code Signing & Release Verification, 7. Uninstallation & Data Removal (+22 more)

### Community 22 - "IndexIssue"
Cohesion: 0.10
Nodes (25): ChangingSource, c_drive_checkpoint(), checkpoint_at(), collect_journal_updates(), collect_records(), entry_for(), journal_update(), NtfsMftSource<I, E, J> (+17 more)

### Community 23 - "crawler.rs"
Cohesion: 0.13
Nodes (10): compute_attributes(), compute_file_id(), FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_REPARSE_POINT, should_skip_path(), worker_loop() (+2 more)

### Community 24 - "test"
Cohesion: 0.08
Nodes (24): jobs, test, os, name, branches, branches, if, name (+16 more)

### Community 25 - "DirectorySource"
Cohesion: 0.23
Nodes (9): assigns_correct_attributes_for_directories_and_dotfiles(), crawls_directory_recursively_and_indexes_entries(), create_test_tree(), directory_snapshots_are_not_resumed(), DirectorySource, fails_on_nonexistent_single_root(), respects_custom_exclusions(), start_watching_reports_changes_without_full_crawl() (+1 more)

### Community 26 - "SortColumn"
Cohesion: 0.18
Nodes (9): effective_query(), header_clicks_start_ascending_and_reverse_the_active_column(), ResultSort, SortColumn, .ALL, Name, Path, Relevance (+1 more)

### Community 27 - "terminal.rs"
Cohesion: 0.06
Nodes (17): CliConfig, detect_default_ui(), parse_delay(), parse_renderer(), ServiceAction, Install, Run, Start (+9 more)

### Community 29 - "FakeEnumerator"
Cohesion: 0.11
Nodes (18): assert_rebuilt_when_journal_is(), FakeCursor, FakeEnumerationState, FakeEnumerator, FakeJournal, journal_source(), preserves_delete_and_rename_old_name_semantics_for_basic_info_changes(), preserves_the_volume_root_when_refreshing_basic_info_changes() (+10 more)

### Community 30 - "5. Implementierungs-Meilensteine"
Cohesion: 0.12
Nodes (16): 1. Motivation & Leitprinzipien, 2.1 `Cargo.toml` Konfiguration, 2.2 CLI-Auswahl & Fallback, 2. Feature-Architektur & Abhängigkeiten, 3.1 Farbpalette (Dark Theme), 3. UI-Design & Layout (Ratatui), 4.1 Tastaturbelegung (Keybindings), 4. TUI-Zustandsmaschine (`TuiApp`) (+8 more)

### Community 31 - "FakeRecord"
Cohesion: 0.18
Nodes (6): chunk(), FakeRecord, mft_and_workspace_sources_reconstruct_matching_nested_entries(), refreshes_attributes_from_basic_info_journal_changes(), TemporaryDirectory, volume_tree_chunk()

### Community 32 - "IndexScope"
Cohesion: 0.22
Nodes (6): IndexScope, LocalDrives, TuiModal, Help, ScopePicker, run()

### Community 33 - "mounts.rs"
Cohesion: 0.20
Nodes (10): default_thread_count(), default_local_roots(), default_system_exclusions(), discover_linux_mounts(), is_local_filesystem(), MountEntry, parse_mount_entries(), parses_linux_mount_entries_correctly() (+2 more)

### Community 35 - "JournalState"
Cohesion: 0.08
Nodes (20): check_read_access(), classify(), device_path_for(), FakeJournalCheckpointReader, Failure, Success, filesystem_name(), journal_state() (+12 more)

### Community 36 - "tui_app_test.rs"
Cohesion: 0.19
Nodes (5): editing_keys_update_query_and_cursor(), modal_dialog_open_and_close(), press_key(), selection_navigation_and_bounds(), typing_triggers_search_command()

### Community 39 - "OxiFind"
Cohesion: 0.20
Nodes (9): Architecture, Current functionality, Current limitations, 🔍 OxiFind, Planned direction, Quality & Architecture Policies, Running the application, Search syntax (+1 more)

### Community 40 - ".from_source"
Cohesion: 0.16
Nodes (8): collected_batches_leave_the_index_unchanged_until_applied(), indexes_workspace_entries_with_relative_paths_and_skips_build_metadata(), JournalSource, records_journal_checkpoints_from_the_completed_index_source(), reports_incremental_updates_as_unsupported_for_workspace_indexes(), retains_entries_when_the_source_reports_recoverable_issues(), retains_workspace_entries_and_reports_inaccessible_directories(), TemporaryDirectory

### Community 41 - "4. Geplante Meilensteine (TDD)"
Cohesion: 0.18
Nodes (10): 1. Leitprinzipien & Richtlinien, 2. Zielarchitektur, 3. Plattformspezifische Matrix, 4. Geplante Meilensteine (TDD), Meilenstein M1: Plattformunabhängigkeit des Kerns & Test-Harmonisierung, Meilenstein M2: Plattform-Abstraktionsschicht (`src/platform/`), Meilenstein M3: Parallele Unix-Indexierung (Linux & macOS), Meilenstein M4: Plattformübergreifende Live-Aktualisierung (+2 more)

### Community 43 - "run_search_worker"
Cohesion: 0.33
Nodes (5): is_volume_root(), ResponseSender, run_search_worker(), spawn_search_worker(), spawn_update_poller()

### Community 45 - "journal_index"
Cohesion: 0.42
Nodes (8): applies_created_moved_and_deleted_entries_in_order(), journal_index(), journal_tree(), keeps_the_index_and_checkpoints_when_collecting_updates_fails(), removes_entries_only_from_the_volume_named_by_the_update(), renaming_a_directory_moves_its_descendants(), replaces_every_entry_of_a_rebuilt_volume(), resolves_orphaned_entries_once_their_parent_is_created()

### Community 46 - "1. Detaillierte Analyse der einzelnen Projekte"
Cohesion: 0.25
Nodes (7): 1.1 `fd` (`examples/fd`), 1.2 `File-Minnow-Linux` (`examples/file-minnow-linux`), 1.3 `goz` (`examples/goz`), 1.4 `Flash Find` (`examples/flash_find`), 1. Detaillierte Analyse der einzelnen Projekte, 2. Vergleichsmatrix mit OxiFind, Architektur-Analyse & Referenz-Implementierungen

### Community 47 - "src"
Cohesion: 0.25
Nodes (7): Auto-generated signatures, Code signatures, SigMap commands, src, src\app.rs, src\ntfs.rs, src\search.rs

### Community 50 - "tui/app.rs"
Cohesion: 0.12
Nodes (4): FocusedWidget, ResultTable, SearchInput, SearchOptions

### Community 53 - "test_journal_recovery.rs"
Cohesion: 0.07
Nodes (28): MftEnumerationCursor, MftRecordEnumerator, VolumeInspector, WindowsMftEnumerator, ARCHIVE_ATTRIBUTE, build_mft_chunk(), DIRECTORY_ATTRIBUTE, DOC_REFERENCE (+20 more)

### Community 54 - "service.rs"
Cohesion: 0.18
Nodes (15): install_service(), plist_path(), schtasks_delay_value(), service_status(), ServiceStatus, Active, Inactive, NotInstalled (+7 more)

### Community 57 - "pathbuf"
Cohesion: 0.21
Nodes (4): collects_and_applies_live_file_events_to_search_index(), crawls_and_indexes_nested_files_with_multithreading(), skips_git_and_target_directories_by_default(), TestDir

### Community 58 - "main.rs"
Cohesion: 0.25
Nodes (6): index_scope_from_args(), indexes_all_local_drives_without_an_argument(), main(), print_usage(), rejects_more_than_one_index_root(), uses_the_single_argument_as_the_index_root()

### Community 59 - "poll_updates"
Cohesion: 0.18
Nodes (15): answers_searches_and_remembers_the_latest_query(), applies_update_batches_and_refreshes_the_latest_query(), emits_collected_batches_and_advances_the_polled_checkpoints(), forwards_update_failures_to_the_ui(), handle_command(), poll_updates(), reports_update_failures_without_advancing_the_polled_checkpoints(), SearchRequest (+7 more)

## Knowledge Gaps
- **61 isolated node(s):** `Relevance`, `Name`, `Path`, `Type`, `CopyName` (+56 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 433 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **19 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `SearchEntry` connect `SearchEntry` to `search.rs`, `app.rs`, `watcher.rs`, `SearchIndex`, `ntfs_source`, `.from_source`, `NtfsMftSource`, `ntfs.rs`, `journal_index`, `persistence.rs`, `IndexIssue`, `crawler.rs`, `DirectorySource`?**
  _High betweenness centrality (0.056) - this node is a cross-community bridge._
- **What connects `Relevance`, `Name`, `Path` to the rest of the system?**
  _61 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `search.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.09009009009009009 - nodes in this community are weakly interconnected._
- **Why does `IndexScope` connect `IndexScope` to `SearchApp`, `app.rs`, `engine.rs`, `.with_channels`, `WorkerResponse`, `run_search_worker`, `tui/app.rs`, `main.rs`, `terminal.rs`, `TuiApp`?**
  _High betweenness centrality (0.052) - this node is a cross-community bridge._
- **Should `SearchApp` be split into smaller, more focused modules?**
  _Cohesion score 0.1497326203208556 - nodes in this community are weakly interconnected._
- **Why does `TuiApp` connect `TuiApp` to `IndexScope`, `.with_channels`, `engine.rs`, `tui_app_test.rs`, `tui/app.rs`, `ui.rs`?**
  _High betweenness centrality (0.046) - this node is a cross-community bridge._
- **Should `app.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.056910569105691054 - nodes in this community are weakly interconnected._