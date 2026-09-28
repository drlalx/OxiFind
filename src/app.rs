use std::{
    cell::{Cell, RefCell},
    error::Error,
    io,
    ops::ControlFlow,
    path::{Component, Path, PathBuf, Prefix},
    rc::Rc,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use slint::{ModelRc, SharedString, VecModel};

use crate::{
    ntfs::NtfsMftSource,
    search::{IndexSource, JournalCheckpoint, SearchIndex, UpdateBatch, WorkspaceSource},
};

const RESULT_LIMIT: usize = 200;
const UPDATE_POLL_INTERVAL: Duration = Duration::from_secs(1);

slint::slint! {
    import { LineEdit, ScrollView, VerticalBox } from "std-widgets.slint";

    export component MainWindow inherits Window {
        in-out property <string> query: "";
        in property <[string]> results: [];
        in property <string> status: "";
        callback query_changed(string);

        VerticalBox {
            padding: 12px;
            spacing: 8px;

            Text {
                text: "Everything Lite";
                font-size: 22px;
            }

            LineEdit {
                text <=> root.query;
                placeholder-text: "Search files";
                edited => {
                    root.query_changed(self.text);
                }
            }

            Text {
                text: root.status;
            }

            ScrollView {
                vertical-stretch: 1;
                VerticalBox {
                    spacing: 4px;
                    for item in root.results : Text {
                        text: item;
                    }
                }
            }
        }
    }
}

#[derive(Default)]
struct SearchRequest {
    revision: u64,
    query: String,
}

#[derive(Default)]
struct SearchResponse {
    revision: u64,
    results: Vec<String>,
}

enum WorkerCommand {
    Search(SearchRequest),
    ApplyUpdates(UpdateBatch),
    UpdateFailed(String),
}

enum WorkerResponse {
    IndexReady {
        indexed_items: usize,
        indexing_issues: usize,
        indexing_duration: Duration,
        initial_results: Vec<String>,
    },
    SearchResults(SearchResponse),
    IndexingFailed(String),
    /// Published after a batch was applied, with the latest query re-run against the new snapshot.
    IndexUpdated {
        indexed_items: usize,
        indexing_issues: usize,
        refreshed: SearchResponse,
    },
    UpdateFailed(String),
}

pub fn run(root: PathBuf) -> Result<(), Box<dyn Error + Send + Sync>> {
    let window = MainWindow::new()?;
    window.set_status(format_indexing_status(&root).into());
    window.set_results(make_results_model(Vec::new()));

    let (request_tx, request_rx) = mpsc::channel::<WorkerCommand>();
    let (response_tx, response_rx) = mpsc::channel::<WorkerResponse>();
    spawn_search_worker(root.clone(), request_tx.clone(), request_rx, response_tx);

    let latest_revision = Rc::new(Cell::new(0_u64));
    let is_indexing = Rc::new(Cell::new(true));
    let worker_stopped = Rc::new(Cell::new(false));
    let response_rx = Rc::new(RefCell::new(response_rx));
    let window_weak = window.as_weak();
    let timer = slint::Timer::default();

    timer.start(slint::TimerMode::Repeated, Duration::from_millis(33), {
        let response_rx = Rc::clone(&response_rx);
        let latest_revision = Rc::clone(&latest_revision);
        let is_indexing = Rc::clone(&is_indexing);
        let worker_stopped = Rc::clone(&worker_stopped);
        let window_weak = window_weak.clone();
        move || {
            loop {
                match response_rx.borrow_mut().try_recv() {
                    Ok(WorkerResponse::IndexReady {
                        indexed_items,
                        indexing_issues,
                        indexing_duration,
                        initial_results,
                    }) => {
                        is_indexing.set(false);
                        if let Some(window) = window_weak.upgrade() {
                            window.set_results(make_results_model(initial_results));
                            window.set_status(
                                format_index_status(
                                    &root,
                                    indexing_duration,
                                    indexed_items,
                                    indexing_issues,
                                )
                                .into(),
                            );
                        }
                    }
                    Ok(WorkerResponse::SearchResults(response)) => {
                        if response.revision == latest_revision.get() {
                            if let Some(window) = window_weak.upgrade() {
                                let result_count = response.results.len();
                                window.set_results(make_results_model(response.results));
                                window.set_status(format!("{result_count} results").into());
                            }
                        }
                    }
                    Ok(WorkerResponse::IndexingFailed(error)) => {
                        is_indexing.set(false);
                        worker_stopped.set(true);
                        if let Some(window) = window_weak.upgrade() {
                            window.set_status(format!("Indexing failed: {error}").into());
                        }
                    }
                    Ok(WorkerResponse::IndexUpdated {
                        indexed_items,
                        indexing_issues,
                        refreshed,
                    }) => {
                        if let Some(window) = window_weak.upgrade() {
                            // A newer query's own response will follow, already based on this snapshot.
                            if refreshed.revision == latest_revision.get() {
                                window.set_results(make_results_model(refreshed.results));
                            }
                            window.set_status(
                                format_update_status(indexed_items, indexing_issues).into(),
                            );
                        }
                    }
                    Ok(WorkerResponse::UpdateFailed(message)) => {
                        if let Some(window) = window_weak.upgrade() {
                            window.set_status(message.into());
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        if !worker_stopped.replace(true) {
                            if let Some(window) = window_weak.upgrade() {
                                window.set_status("Search worker stopped".into());
                            }
                        }
                        break;
                    }
                }
            }
        }
    });

    window.on_query_changed({
        let latest_revision = Rc::clone(&latest_revision);
        let is_indexing = Rc::clone(&is_indexing);
        let worker_stopped = Rc::clone(&worker_stopped);
        let request_tx = request_tx.clone();
        let window_weak = window_weak.clone();
        move |query| {
            let revision = latest_revision.get().saturating_add(1);
            latest_revision.set(revision);

            if worker_stopped.get() {
                return;
            }

            if !is_indexing.get() {
                if let Some(window) = window_weak.upgrade() {
                    window.set_status(format!("Searching for \"{}\"", query).into());
                }
            }

            if request_tx
                .send(WorkerCommand::Search(SearchRequest {
                    revision,
                    query: query.to_string(),
                }))
                .is_err()
            {
                worker_stopped.set(true);
                if let Some(window) = window_weak.upgrade() {
                    window.set_status("Search worker stopped".into());
                }
            }
        }
    });

    window.run()?;
    Ok(())
}

fn spawn_search_worker(
    root: PathBuf,
    command_tx: mpsc::Sender<WorkerCommand>,
    command_rx: mpsc::Receiver<WorkerCommand>,
    response_tx: mpsc::Sender<WorkerResponse>,
) {
    thread::spawn(move || {
        // Drive roots require MFT access; a failure is reported instead of falling back to walking.
        if is_volume_root(&root) {
            run_search_worker(
                NtfsMftSource::new(root),
                command_tx,
                command_rx,
                response_tx,
            );
        } else {
            run_search_worker(
                WorkspaceSource::new(root),
                command_tx,
                command_rx,
                response_tx,
            );
        }
    });
}

fn run_search_worker<S: IndexSource + Send + 'static>(
    source: S,
    command_tx: mpsc::Sender<WorkerCommand>,
    command_rx: mpsc::Receiver<WorkerCommand>,
    response_tx: mpsc::Sender<WorkerResponse>,
) {
    let indexing_started = Instant::now();
    let mut index = match SearchIndex::from_source(&source) {
        Ok(index) => index,
        Err(error) => {
            let _ = response_tx.send(WorkerResponse::IndexingFailed(error.to_string()));
            return;
        }
    };

    if response_tx
        .send(WorkerResponse::IndexReady {
            indexed_items: index.len(),
            indexing_issues: index.indexing_issue_count(),
            indexing_duration: indexing_started.elapsed(),
            initial_results: index.search("", RESULT_LIMIT),
        })
        .is_err()
    {
        return;
    }

    spawn_update_poller(source, index.journal_checkpoints().to_vec(), command_tx);

    let mut latest = SearchRequest::default();
    while let Ok(command) = command_rx.recv() {
        if response_tx
            .send(handle_command(&mut index, &mut latest, command))
            .is_err()
        {
            return;
        }
    }
}

/// Handles one worker command; the index is only touched here, so every response reflects one snapshot.
fn handle_command(
    index: &mut SearchIndex,
    latest: &mut SearchRequest,
    command: WorkerCommand,
) -> WorkerResponse {
    match command {
        WorkerCommand::Search(request) => {
            let response = SearchResponse {
                revision: request.revision,
                results: index.search(&request.query, RESULT_LIMIT),
            };
            *latest = request;
            WorkerResponse::SearchResults(response)
        }
        WorkerCommand::ApplyUpdates(batch) => {
            index.apply(batch);
            WorkerResponse::IndexUpdated {
                indexed_items: index.len(),
                indexing_issues: index.indexing_issue_count(),
                refreshed: SearchResponse {
                    revision: latest.revision,
                    results: index.search(&latest.query, RESULT_LIMIT),
                },
            }
        }
        WorkerCommand::UpdateFailed(message) => WorkerResponse::UpdateFailed(message),
    }
}

fn spawn_update_poller<S: IndexSource + Send + 'static>(
    source: S,
    mut checkpoints: Vec<JournalCheckpoint>,
    command_tx: mpsc::Sender<WorkerCommand>,
) {
    thread::spawn(move || {
        loop {
            match poll_updates(&source, &mut checkpoints) {
                ControlFlow::Break(()) => return,
                ControlFlow::Continue(Some(command)) => {
                    if command_tx.send(command).is_err() {
                        return;
                    }
                }
                ControlFlow::Continue(None) => {}
            }
            thread::sleep(UPDATE_POLL_INTERVAL);
        }
    });
}

/// Collects one batch after `checkpoints`, advancing them only when collection succeeds.
///
/// Breaks for sources without incremental updates. Empty batches are not sent; the next
/// non-empty batch carries the advanced checkpoints to the index.
fn poll_updates(
    source: &impl IndexSource,
    checkpoints: &mut Vec<JournalCheckpoint>,
) -> ControlFlow<(), Option<WorkerCommand>> {
    match UpdateBatch::collect(source, checkpoints) {
        Ok(batch) => {
            checkpoints.clone_from(&batch.checkpoints);
            ControlFlow::Continue((!batch.is_empty()).then_some(WorkerCommand::ApplyUpdates(batch)))
        }
        Err(error) if error.kind() == io::ErrorKind::Unsupported => ControlFlow::Break(()),
        Err(error) => ControlFlow::Continue(Some(WorkerCommand::UpdateFailed(format!(
            "Index update failed: {error}"
        )))),
    }
}

/// Returns whether `path` names exactly the root directory of a local drive, such as `C:\`.
fn is_volume_root(path: &Path) -> bool {
    let mut components = path.components();
    let is_drive = matches!(
        components.next(),
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
    );
    is_drive && matches!(components.next(), Some(Component::RootDir)) && components.next().is_none()
}

fn make_results_model(results: Vec<String>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        results
            .into_iter()
            .map(SharedString::from)
            .collect::<Vec<_>>(),
    ))
}

fn format_indexing_status(root: &std::path::Path) -> String {
    format!("Indexing {}...", root.display())
}

fn format_index_status(
    root: &std::path::Path,
    indexing_duration: Duration,
    indexed_items: usize,
    indexing_issues: usize,
) -> String {
    let status = format!(
        "Indexed {indexed_items} items from {} in {} ms",
        root.display(),
        indexing_duration.as_millis()
    );

    if indexing_issues == 0 {
        return status;
    }

    let path_label = if indexing_issues == 1 {
        "path"
    } else {
        "paths"
    };
    format!("{status} ({indexing_issues} inaccessible {path_label})")
}

fn format_update_status(indexed_items: usize, indexing_issues: usize) -> String {
    let status = format!("Index updated: {indexed_items} items");
    match indexing_issues {
        0 => status,
        1 => format!("{status} (1 issue)"),
        _ => format!("{status} ({indexing_issues} issues)"),
    }
}

#[cfg(test)]
mod tests {
    use std::{io, ops::ControlFlow, path::Path, time::Duration};

    use crate::search::{
        EntryMetadata, IndexIssue, IndexSource, IndexUpdate, JournalCheckpoint, SearchEntry,
        SearchIndex, UpdateBatch, WorkspaceSource,
    };

    use super::{
        SearchRequest, WorkerCommand, WorkerResponse, format_index_status, format_indexing_status,
        format_update_status, handle_command, is_volume_root, poll_updates,
    };

    #[derive(Default)]
    struct ChangingSource {
        updates: Vec<IndexUpdate>,
        fails: bool,
    }

    impl IndexSource for ChangingSource {
        fn collect_initial(
            &self,
            out: &mut Vec<SearchEntry>,
            _issues: &mut Vec<IndexIssue>,
        ) -> io::Result<()> {
            out.push(entry(1, None, r"C:\"));
            out.push(entry(2, Some(1), "alpha.txt"));
            Ok(())
        }

        fn collect_updates(
            &self,
            checkpoints: &mut [JournalCheckpoint],
            out: &mut Vec<IndexUpdate>,
            _issues: &mut Vec<IndexIssue>,
        ) -> io::Result<()> {
            if self.fails {
                return Err(io::Error::other("journal offline"));
            }
            out.extend(self.updates.clone());
            for checkpoint in checkpoints {
                checkpoint.next_usn += 1;
            }
            Ok(())
        }

        fn journal_checkpoints(&self) -> io::Result<Vec<JournalCheckpoint>> {
            Ok(vec![JournalCheckpoint {
                volume_identity: r"C:\".to_owned(),
                journal_id: 1,
                next_usn: 10,
            }])
        }
    }

    fn entry(file_id: u128, parent_id: Option<u128>, name: &str) -> SearchEntry {
        SearchEntry {
            display: name.to_owned(),
            path: name.to_owned(),
            metadata: EntryMetadata {
                file_id: Some(file_id),
                parent_id,
                name: name.to_owned(),
                full_path: name.into(),
                volume_identity: Some(r"C:\".to_owned()),
                attributes: 0,
            },
        }
    }

    #[test]
    fn answers_searches_and_remembers_the_latest_query() {
        let mut index = SearchIndex::from_source(&ChangingSource::default()).unwrap();
        let mut latest = SearchRequest::default();

        let response = handle_command(
            &mut index,
            &mut latest,
            WorkerCommand::Search(SearchRequest {
                revision: 3,
                query: "alp".to_owned(),
            }),
        );

        let WorkerResponse::SearchResults(response) = response else {
            panic!("a search must be answered with search results");
        };
        assert_eq!(response.revision, 3);
        assert_eq!(response.results, vec![r"C:\alpha.txt"]);
        assert_eq!(latest.query, "alp");
    }

    #[test]
    fn applies_update_batches_and_refreshes_the_latest_query() {
        let source = ChangingSource {
            updates: vec![IndexUpdate::Upsert(entry(3, Some(1), "alpine.txt"))],
            ..ChangingSource::default()
        };
        let mut index = SearchIndex::from_source(&source).unwrap();
        let batch = UpdateBatch::collect(&source, index.journal_checkpoints()).unwrap();
        let mut latest = SearchRequest {
            revision: 5,
            query: "alp".to_owned(),
        };

        let response = handle_command(&mut index, &mut latest, WorkerCommand::ApplyUpdates(batch));

        let WorkerResponse::IndexUpdated {
            indexed_items,
            indexing_issues,
            refreshed,
        } = response
        else {
            panic!("an applied batch must publish the updated index");
        };
        assert_eq!(indexed_items, 3);
        assert_eq!(indexing_issues, 0);
        assert_eq!(refreshed.revision, 5);
        assert_eq!(refreshed.results, vec![r"C:\alpha.txt", r"C:\alpine.txt"]);
    }

    #[test]
    fn forwards_update_failures_to_the_ui() {
        let mut index = SearchIndex::from_source(&ChangingSource::default()).unwrap();

        let response = handle_command(
            &mut index,
            &mut SearchRequest::default(),
            WorkerCommand::UpdateFailed("journal offline".to_owned()),
        );

        assert!(
            matches!(response, WorkerResponse::UpdateFailed(message) if message == "journal offline")
        );
    }

    #[test]
    fn stops_polling_sources_without_incremental_updates() {
        let flow = poll_updates(&WorkspaceSource::new("."), &mut Vec::new());

        assert!(flow.is_break());
    }

    #[test]
    fn emits_collected_batches_and_advances_the_polled_checkpoints() {
        let source = ChangingSource {
            updates: vec![IndexUpdate::Upsert(entry(3, Some(1), "alpine.txt"))],
            ..ChangingSource::default()
        };
        let mut checkpoints = source.journal_checkpoints().unwrap();

        let flow = poll_updates(&source, &mut checkpoints);

        assert!(matches!(
            flow,
            ControlFlow::Continue(Some(WorkerCommand::ApplyUpdates(batch))) if batch.updates.len() == 1
        ));
        assert_eq!(checkpoints[0].next_usn, 11);
    }

    #[test]
    fn skips_empty_batches_but_keeps_their_checkpoints() {
        let source = ChangingSource::default();
        let mut checkpoints = source.journal_checkpoints().unwrap();

        let flow = poll_updates(&source, &mut checkpoints);

        assert!(matches!(flow, ControlFlow::Continue(None)));
        assert_eq!(checkpoints[0].next_usn, 11);
    }

    #[test]
    fn reports_update_failures_without_advancing_the_polled_checkpoints() {
        let source = ChangingSource {
            fails: true,
            ..ChangingSource::default()
        };
        let mut checkpoints = source.journal_checkpoints().unwrap();

        let flow = poll_updates(&source, &mut checkpoints);

        assert!(matches!(
            flow,
            ControlFlow::Continue(Some(WorkerCommand::UpdateFailed(message))) if message.contains("journal offline")
        ));
        assert_eq!(checkpoints[0].next_usn, 10);
    }

    #[test]
    fn treats_only_drive_roots_as_volume_selections() {
        assert!(is_volume_root(Path::new(r"C:\")));
        assert!(is_volume_root(Path::new(r"\\?\D:\")));
        assert!(!is_volume_root(Path::new("C:")));
        assert!(!is_volume_root(Path::new(r"C:\Users")));
        assert!(!is_volume_root(Path::new("relative")));
        assert!(!is_volume_root(Path::new(r"\\server\share\")));
    }

    #[test]
    fn reports_item_and_issue_counts_after_an_index_update() {
        assert_eq!(format_update_status(3, 0), "Index updated: 3 items");
        assert_eq!(
            format_update_status(3, 1),
            "Index updated: 3 items (1 issue)"
        );
        assert_eq!(
            format_update_status(3, 2),
            "Index updated: 3 items (2 issues)"
        );
    }

    #[test]
    fn reports_the_index_root_while_initial_indexing_is_in_progress() {
        assert_eq!(
            format_indexing_status(Path::new(r"C:\search-root")),
            r"Indexing C:\search-root...",
        );
    }

    #[test]
    fn reports_index_root_and_duration_in_the_startup_status() {
        assert_eq!(
            format_index_status(
                Path::new(r"C:\search-root"),
                Duration::from_millis(250),
                12,
                0
            ),
            r"Indexed 12 items from C:\search-root in 250 ms",
        );
    }

    #[test]
    fn omits_issue_information_when_indexing_completes_without_issues() {
        assert_eq!(
            format_index_status(Path::new(r"C:\search-root"), Duration::ZERO, 12, 0),
            r"Indexed 12 items from C:\search-root in 0 ms",
        );
    }

    #[test]
    fn reports_recoverable_indexing_issues_in_the_startup_status() {
        assert_eq!(
            format_index_status(Path::new(r"C:\search-root"), Duration::ZERO, 12, 2),
            r"Indexed 12 items from C:\search-root in 0 ms (2 inaccessible paths)",
        );
    }

    #[test]
    fn uses_the_singular_wording_for_one_indexing_issue() {
        assert_eq!(
            format_index_status(Path::new(r"C:\search-root"), Duration::ZERO, 1, 1),
            r"Indexed 1 items from C:\search-root in 0 ms (1 inaccessible path)",
        );
    }
}
