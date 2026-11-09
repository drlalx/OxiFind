# Graph Report - OxiFind  (2026-11-09)

## Corpus Check
- 47 files · ~52,964 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 8 file(s) not represented in the graph (top: (none) 3, .toml 1, .desktop 1)

## Summary
- 1181 nodes · 2495 edges · 73 communities (55 shown, 18 thin omitted)
- Extraction: 99% EXTRACTED · 1% INFERRED · 0% AMBIGUOUS · INFERRED: 31 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `17869026`
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
- JournalCheckpoint
- crawler.rs
- test
- DirectorySource
- SortColumn
- terminal.rs
- TuiApp
- FakeEnumerator
- 5. Implementierungs-Meilensteine
- FakeRecord
- JournalState
- mounts.rs
- .new
- JournalState
- tui_app_test.rs
- install.sh
- io
- OxiFind
- .from_source
- 4. Geplante Meilensteine (TDD)
- uninstall.sh
- run_search_worker
- render
- journal_index
- 1. Detaillierte Analyse der einzelnen Projekte
- src
- SearchResult
- .new
- tui/app.rs
- rust-programmer.agent.md
- test_journal_recovery.rs
- service.rs
- pathbuf
- main.rs
- poll_updates
- VolumeInspector
- .new
- macos/install.sh
- macos/uninstall.sh
- ServiceAction
- .collect_initial
- MftEnumerationCursor
- test_continuous_journal_update_advances_checkpoint
- FakeJournalCheckpointReader
- RowAction

## God Nodes (most connected - your core abstractions)
1. `SearchEntry` - 59 edges
2. `IndexIssue` - 42 edges
3. `JournalCheckpoint` - 41 edges
4. `TuiApp` - 41 edges
5. `SearchApp` - 36 edges
6. `ntfs_source()` - 32 edges
7. `SearchIndex` - 28 edges
8. `IndexSource` - 26 edges
9. `collect_entries()` - 25 edges
10. `IndexUpdate` - 25 edges

## Surprising Connections (you probably didn't know these)
- `test_worker_cold_start_and_recovery_workflow()` --calls--> `spawn_search_worker_with_snapshot()`  [INFERRED]
  tests/test_persistence.rs → src/engine.rs
- `test_first_run_snapshot_and_cold_start_recovery()` --calls--> `save_snapshot()`  [INFERRED]
  tests/test_journal_recovery.rs → src/persistence.rs
- `test_first_run_snapshot_and_cold_start_recovery()` --calls--> `load_snapshot()`  [INFERRED]
  tests/test_journal_recovery.rs → src/persistence.rs
- `test_load_non_existent_snapshot_returns_none()` --calls--> `load_snapshot()`  [INFERRED]
  tests/test_persistence.rs → src/persistence.rs
- `test_atomic_save_and_load_snapshot_roundtrip()` --calls--> `save_snapshot()`  [INFERRED]
  tests/test_persistence.rs → src/persistence.rs

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

## Communities (73 total, 18 thin omitted)

### Community 0 - "search.rs"
Cohesion: 0.10
Nodes (27): breaks_sort_key_ties_by_relevance(), builds_child_paths_from_parent_records(), caret_terms_match_only_file_name_prefixes(), compare_case_insensitive(), compare_entries(), compare_text(), equals_terms_match_whole_file_names_only(), FILE_ATTRIBUTE_DIRECTORY (+19 more)

### Community 1 - "SearchApp"
Cohesion: 0.17
Nodes (6): rejects_unresolved_index_entries_when_preparing_opening_paths(), Open, SearchApp, ServiceEvent, Finished, Status

### Community 2 - "app.rs"
Cohesion: 0.06
Nodes (20): APP_NAME, default_graphics_backend(), describe_attributes(), ensure_linux_graphics_backend(), FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_SYSTEM (+12 more)

### Community 3 - "engine.rs"
Cohesion: 0.05
Nodes (46): effective_query(), FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_SYSTEM, handle_command() (+38 more)

### Community 4 - "watcher.rs"
Cohesion: 0.14
Nodes (4): EntryMetadata, detects_file_creation_and_deletion_events(), DirectoryWatcher, TestDir

### Community 5 - "SearchIndex"
Cohesion: 0.11
Nodes (21): entry(), build_search_data(), directory(), entry(), filters_large_index_efficiently(), identified_entry(), is_directory(), large_index_fixture() (+13 more)

### Community 6 - "SearchEntry"
Cohesion: 0.09
Nodes (18): ChangingSource, WorkerResult, describe(), CheckpointSource, collect_directories(), collected_batches_leave_the_index_unchanged_until_applied(), IndexIssue, IndexSource (+10 more)

### Community 7 - "ntfs_source"
Cohesion: 0.26
Nodes (19): anchors_records_to_the_volume_root_when_no_root_record_is_reported(), anchors_the_enumerated_root_record_to_the_volume_root(), collect_entries(), entry(), enumerates_the_volume_only_after_validation_succeeds(), indexes_active_records_with_identifiers_names_attributes_and_volume_identity(), keeps_file_reference_sequences_when_resolving_parents(), ntfs_source() (+11 more)

### Community 8 - "WorkerResponse"
Cohesion: 0.21
Nodes (7): format_index_status(), format_indexing_status(), format_update_status(), ignores_results_for_superseded_queries(), keeps_the_indexing_failure_when_the_worker_disconnects(), keeps_the_selected_path_selected_across_result_updates(), IndexingFailed

### Community 9 - "NtfsMftSource"
Cohesion: 0.14
Nodes (23): collect_initial_error(), fails_instead_of_reporting_an_empty_index_when_no_records_are_enumerated(), fails_when_every_enumerated_record_is_unusable(), FakeVolumeInspector, multi_collect(), multi_volume_source_deduplicates_equivalent_selected_volume_roots(), multi_volume_source_fails_when_every_selected_volume_fails(), multi_volume_source_fails_when_no_volumes_are_selected() (+15 more)

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
Nodes (40): ALPHA_ID, ALPHA_REFERENCE, ARCHIVE_ATTRIBUTE, BASIC_INFO_CHANGE_REASON, BRAVO_REFERENCE, CLOSE_REASON, CREATE_REASON, DATA_EXTEND_REASON (+32 more)

### Community 17 - "persistence.rs"
Cohesion: 0.08
Nodes (26): AppSettings, compute_crc32(), CURRENT_VERSION, default_data_dir(), default_settings_path(), default_snapshot_path(), HEADER_SIZE, load_settings() (+18 more)

### Community 18 - "Code-Review des aktuellen Stands (2026-11-09)"
Cohesion: 0.33
Nodes (5): Code-Review des aktuellen Stands (2026-11-09), Hoch, Mittel, Niedrig, Status (nach Umsetzung)

### Community 19 - "desktop.rs"
Cohesion: 0.08
Nodes (23): containing_folder(), explorer_argument(), file_uri(), open_in_explorer(), open_in_finder(), open_in_linux_desktop(), open_path(), open_with_dialog() (+15 more)

### Community 20 - "ui.rs"
Cohesion: 0.09
Nodes (11): COLOR_ACCENT, COLOR_BG, COLOR_BORDER, COLOR_BORDER_FOCUS, COLOR_CARD, COLOR_DANGER, COLOR_DIM, COLOR_HIGHLIGHT (+3 more)

### Community 21 - "OxiFind Operations & Deployment Guide"
Cohesion: 0.06
Nodes (30): 1. Architecture & Execution Modes, 2. Installation & Distribution, 3. Permissions & Security Model, 4. Persistent Storage & Configuration, 5. Background Service & Daemon Mode (Syncthing-like), 5. Diagnostics & Operational Recovery, 6. Code Signing & Release Verification, 7. Uninstallation & Data Removal (+22 more)

### Community 22 - "JournalCheckpoint"
Cohesion: 0.14
Nodes (15): c_drive_checkpoint(), checkpoint_at(), collect_journal_updates(), collect_records(), entry_for(), journal_update(), NtfsMftSource<I, E, J>, read_u64() (+7 more)

### Community 23 - "crawler.rs"
Cohesion: 0.12
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

### Community 29 - "FakeEnumerator"
Cohesion: 0.11
Nodes (17): assert_rebuilt_when_journal_is(), FakeCursor, FakeEnumerationState, FakeEnumerator, FakeJournal, journal_source(), preserves_delete_and_rename_old_name_semantics_for_basic_info_changes(), preserves_the_volume_root_when_refreshing_basic_info_changes() (+9 more)

### Community 30 - "5. Implementierungs-Meilensteine"
Cohesion: 0.12
Nodes (16): 1. Motivation & Leitprinzipien, 2.1 `Cargo.toml` Konfiguration, 2.2 CLI-Auswahl & Fallback, 2. Feature-Architektur & Abhängigkeiten, 3.1 Farbpalette (Dark Theme), 3. UI-Design & Layout (Ratatui), 4.1 Tastaturbelegung (Keybindings), 4. TUI-Zustandsmaschine (`TuiApp`) (+8 more)

### Community 31 - "FakeRecord"
Cohesion: 0.13
Nodes (11): chunk(), extracts_the_drive_root_from_an_absolute_path(), FakeRecord, mft_and_workspace_sources_reconstruct_matching_nested_entries(), refreshes_attributes_from_basic_info_journal_changes(), rejects_relative_paths_without_a_drive_letter(), rejects_unc_network_share_paths(), reports_trailing_bytes_that_cannot_form_a_record_length() (+3 more)

### Community 32 - "JournalState"
Cohesion: 0.21
Nodes (6): journal_state(), JournalCheckpointReader, JournalState, read_journal_changes(), WindowsJournalCheckpointReader, MockJournalReader

### Community 33 - "mounts.rs"
Cohesion: 0.20
Nodes (10): default_thread_count(), default_local_roots(), default_system_exclusions(), discover_linux_mounts(), is_local_filesystem(), MountEntry, parse_mount_entries(), parses_linux_mount_entries_correctly() (+2 more)

### Community 35 - "JournalState"
Cohesion: 0.19
Nodes (7): classify(), device_path_for(), open_volume(), to_io_error(), treats_non_win32_hresults_as_other(), UsnEnumerationCursor, win32_code()

### Community 36 - "tui_app_test.rs"
Cohesion: 0.17
Nodes (5): editing_keys_update_query_and_cursor(), modal_dialog_open_and_close(), press_key(), selection_navigation_and_bounds(), typing_triggers_search_command()

### Community 38 - "io"
Cohesion: 0.17
Nodes (9): CliConfig, detect_default_ui(), parse_delay(), parse_renderer(), SUPPORTED_RENDERERS, UiBackend, Egui, Tui (+1 more)

### Community 39 - "OxiFind"
Cohesion: 0.20
Nodes (9): Architecture, Current functionality, Current limitations, 🔍 OxiFind, Planned direction, Quality & Architecture Policies, Running the application, Search syntax (+1 more)

### Community 40 - ".from_source"
Cohesion: 0.19
Nodes (7): DeniedListing, indexes_workspace_entries_with_relative_paths_and_skips_build_metadata(), records_journal_checkpoints_from_the_completed_index_source(), reports_incremental_updates_as_unsupported_for_workspace_indexes(), retains_entries_when_the_source_reports_recoverable_issues(), retains_workspace_entries_and_reports_inaccessible_directories(), TemporaryDirectory

### Community 41 - "4. Geplante Meilensteine (TDD)"
Cohesion: 0.18
Nodes (10): 1. Leitprinzipien & Richtlinien, 2. Zielarchitektur, 3. Plattformspezifische Matrix, 4. Geplante Meilensteine (TDD), Meilenstein M1: Plattformunabhängigkeit des Kerns & Test-Harmonisierung, Meilenstein M2: Plattform-Abstraktionsschicht (`src/platform/`), Meilenstein M3: Parallele Unix-Indexierung (Linux & macOS), Meilenstein M4: Plattformübergreifende Live-Aktualisierung (+2 more)

### Community 43 - "run_search_worker"
Cohesion: 0.26
Nodes (8): is_volume_root(), ResponseSender, run_search_worker(), spawn_search_worker(), spawn_update_poller(), WorkerCommand, Search, UpdateFailed

### Community 44 - "render"
Cohesion: 0.47
Nodes (8): centered_rect(), render(), render_details(), render_footer(), render_header(), render_modal(), render_search_bar(), render_table()

### Community 45 - "journal_index"
Cohesion: 0.42
Nodes (8): applies_created_moved_and_deleted_entries_in_order(), journal_index(), journal_tree(), keeps_the_index_and_checkpoints_when_collecting_updates_fails(), removes_entries_only_from_the_volume_named_by_the_update(), renaming_a_directory_moves_its_descendants(), replaces_every_entry_of_a_rebuilt_volume(), resolves_orphaned_entries_once_their_parent_is_created()

### Community 46 - "1. Detaillierte Analyse der einzelnen Projekte"
Cohesion: 0.25
Nodes (7): 1.1 `fd` (`examples/fd`), 1.2 `File-Minnow-Linux` (`examples/file-minnow-linux`), 1.3 `goz` (`examples/goz`), 1.4 `Flash Find` (`examples/flash_find`), 1. Detaillierte Analyse der einzelnen Projekte, 2. Vergleichsmatrix mit OxiFind, Architektur-Analyse & Referenz-Implementierungen

### Community 47 - "src"
Cohesion: 0.25
Nodes (7): Auto-generated signatures, Code signatures, SigMap commands, src, src\app.rs, src\ntfs.rs, src\search.rs

### Community 48 - "SearchResult"
Cohesion: 0.17
Nodes (15): answers_searches_and_remembers_the_latest_query(), displays(), forwards_update_failures_to_the_ui(), handle_command(), result(), search_results(), SearchRequest, SearchResponse (+7 more)

### Community 50 - "tui/app.rs"
Cohesion: 0.12
Nodes (6): FocusedWidget, ResultTable, SearchInput, TuiModal, Help, ScopePicker

### Community 53 - "test_journal_recovery.rs"
Cohesion: 0.15
Nodes (8): ARCHIVE_ATTRIBUTE, DIRECTORY_ATTRIBUTE, DOC_REFERENCE, HEADER_LEN, PROJECTS_REFERENCE, ROOT_REFERENCE, USN_REASON_CLOSE, USN_REASON_FILE_CREATE

### Community 54 - "service.rs"
Cohesion: 0.18
Nodes (15): install_service(), plist_path(), schtasks_delay_value(), service_status(), ServiceStatus, Active, Inactive, NotInstalled (+7 more)

### Community 57 - "pathbuf"
Cohesion: 0.19
Nodes (4): collects_and_applies_live_file_events_to_search_index(), crawls_and_indexes_nested_files_with_multithreading(), skips_git_and_target_directories_by_default(), TestDir

### Community 58 - "main.rs"
Cohesion: 0.25
Nodes (6): index_scope_from_args(), indexes_all_local_drives_without_an_argument(), main(), print_usage(), rejects_more_than_one_index_root(), uses_the_single_argument_as_the_index_root()

### Community 59 - "poll_updates"
Cohesion: 0.31
Nodes (7): applies_update_batches_and_refreshes_the_latest_query(), emits_collected_batches_and_advances_the_polled_checkpoints(), poll_updates(), reports_update_failures_without_advancing_the_polled_checkpoints(), skips_empty_batches_but_keeps_their_checkpoints(), stops_polling_sources_without_incremental_updates(), ApplyUpdates

### Community 60 - "VolumeInspector"
Cohesion: 0.18
Nodes (6): check_read_access(), filesystem_name(), unsupported_platform_error(), VolumeInspector, WindowsVolumeInspector, MockVolumeInspector

### Community 61 - ".new"
Cohesion: 0.38
Nodes (8): test_first_run_snapshot_and_cold_start_recovery(), test_inaccessible_volume_permission_denied(), test_journal_checkpoint_past_end_corruption_triggers_rebuild(), test_journal_recreation_triggers_rebuild(), test_journal_wrap_purged_records_triggers_full_volume_rebuild(), test_multi_volume_all_failing_reports_explicit_error(), test_multi_volume_mixed_resilience(), test_unsupported_volume_filesystem_explicit_error()

### Community 66 - "ServiceAction"
Cohesion: 0.29
Nodes (7): ServiceAction, Install, Run, Start, Status, Stop, Uninstall

### Community 67 - ".collect_initial"
Cohesion: 0.28
Nodes (5): assert_snapshot_rename_is_replayed(), multi_volume_source_defaults_to_no_selected_volumes(), NtfsMultiVolumeSource<I, E, J>, retains_the_checkpoint_before_multi_volume_enumeration(), retains_the_checkpoint_before_single_volume_enumeration()

### Community 68 - "MftEnumerationCursor"
Cohesion: 0.24
Nodes (5): MftEnumerationCursor, MftRecordEnumerator, WindowsMftEnumerator, MockCursor, MockEnumerator

### Community 70 - "test_continuous_journal_update_advances_checkpoint"
Cohesion: 0.42
Nodes (4): build_mft_chunk(), standard_volume_chunk(), test_continuous_journal_update_advances_checkpoint(), TestRecord

### Community 71 - "FakeJournalCheckpointReader"
Cohesion: 0.40
Nodes (3): FakeJournalCheckpointReader, Failure, Success

### Community 72 - "RowAction"
Cohesion: 0.50
Nodes (4): RowAction, CopyFolderPath, CopyName, CopyPath

## Knowledge Gaps
- **61 isolated node(s):** `oxifind`, `Relevance`, `Name`, `Path`, `Type` (+56 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 431 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **18 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `SearchEntry` connect `SearchIndex` to `search.rs`, `app.rs`, `.collect_initial`, `watcher.rs`, `SearchEntry`, `ntfs_source`, `NtfsMftSource`, `ntfs.rs`, `journal_index`, `persistence.rs`, `JournalCheckpoint`, `crawler.rs`, `DirectorySource`?**
  _High betweenness centrality (0.056) - this node is a cross-community bridge._
- **What connects `oxifind`, `Relevance`, `Name` to the rest of the system?**
  _61 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `search.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.09982174688057041 - nodes in this community are weakly interconnected._
- **Why does `JournalCheckpoint` connect `JournalCheckpoint` to `search.rs`, `app.rs`, `engine.rs`, `.collect_initial`, `SearchIndex`, `SearchEntry`, `NtfsMftSource`, `run_search_worker`, `ntfs.rs`, `persistence.rs`, `crawler.rs`, `DirectorySource`, `poll_updates`, `FakeEnumerator`?**
  _High betweenness centrality (0.048) - this node is a cross-community bridge._
- **Should `app.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.056910569105691054 - nodes in this community are weakly interconnected._
- **Why does `IndexScope` connect `engine.rs` to `SearchApp`, `app.rs`, `io`, `WorkerResponse`, `run_search_worker`, `tui/app.rs`, `main.rs`, `TuiApp`?**
  _High betweenness centrality (0.047) - this node is a cross-community bridge._
- **Should `engine.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.050724637681159424 - nodes in this community are weakly interconnected._