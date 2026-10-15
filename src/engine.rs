use std::{
    fmt, io,
    ops::ControlFlow,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use crate::{
    crawler::DirectorySource,
    search::{
        IndexSource, JournalCheckpoint, SearchIndex, SearchResult, UpdateBatch,
    },
};
#[cfg(windows)]
use crate::ntfs::{NtfsMftSource, NtfsMultiVolumeSource, local_fixed_volumes};

pub const RESULT_LIMIT: usize = 200;
pub const UPDATE_POLL_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SortColumn {
    #[default]
    Relevance,
    Name,
    Path,
    Type,
    Size,
    DateModified,
}

impl SortColumn {
    pub const ALL: [Self; 4] = [Self::Relevance, Self::Name, Self::Path, Self::Type];

    pub fn label(self) -> &'static str {
        match self {
            Self::Relevance => "Relevance",
            Self::Name => "Name",
            Self::Path => "Path",
            Self::Type => "Type",
            Self::Size => "Size",
            Self::DateModified => "Date Modified",
        }
    }

    pub fn query_key(self) -> &'static str {
        match self {
            Self::Relevance => "relevance",
            Self::Name => "name",
            Self::Path => "path",
            Self::Type => "type",
            Self::Size => "size",
            Self::DateModified => "dm",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ResultSort {
    pub column: SortColumn,
    pub descending: bool,
}

impl ResultSort {
    /// Clicking the active column reverses it; any other column starts ascending.
    pub fn toggled(self, column: SortColumn) -> Self {
        Self {
            column,
            descending: self.column == column && !self.descending,
        }
    }

    pub fn header_label(self, column: SortColumn) -> String {
        match (self.column == column, self.descending) {
            (true, false) => format!("{} \u{2B06}", column.label()),
            (true, true) => format!("{} \u{2B07}", column.label()),
            (false, _) => column.label().to_owned(),
        }
    }

    pub fn label(self) -> String {
        let direction = if self.descending {
            "descending"
        } else {
            "ascending"
        };
        format!("Sort: {} ({direction})", self.column.label())
    }

    pub fn status_label(self) -> String {
        self.label()
    }
}

/// Prefixes the UI sort order, so a `sort:` term typed into the query still wins.
pub fn effective_query(query: &str, sort: ResultSort) -> String {
    if sort == ResultSort::default() {
        return query.to_owned();
    }
    let direction = if sort.descending { "-" } else { "" };
    format!("sort:{direction}{} {query}", sort.column.query_key())
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchRequest {
    pub revision: u64,
    pub query: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchResponse {
    pub revision: u64,
    pub results: Vec<SearchResult>,
}

#[derive(Debug)]
pub enum WorkerCommand {
    Search(SearchRequest),
    ApplyUpdates(UpdateBatch),
    UpdateFailed(String),
}

#[derive(Clone, Debug)]
pub enum WorkerResponse {
    IndexReady {
        indexed_items: usize,
        indexing_issues: usize,
        indexing_duration: Duration,
        initial_results: Vec<SearchResult>,
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

/// What the application indexes.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum IndexScope {
    /// Every local fixed drive, read from its master file table or root directories.
    #[default]
    LocalDrives,
    /// One directory, or one drive when the path is a drive root.
    Root(PathBuf),
}

impl fmt::Display for IndexScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LocalDrives => formatter.write_str("all local drives"),
            Self::Root(root) => write!(formatter, "{}", root.display()),
        }
    }
}

pub trait WorkerNotifier: Send + 'static {
    fn notify(&self, response: WorkerResponse) -> Result<(), mpsc::SendError<WorkerResponse>>;
}

impl WorkerNotifier for mpsc::Sender<WorkerResponse> {
    fn notify(&self, response: WorkerResponse) -> Result<(), mpsc::SendError<WorkerResponse>> {
        self.send(response)
    }
}

pub fn spawn_search_worker<N: WorkerNotifier>(
    scope: IndexScope,
    command_tx: mpsc::Sender<WorkerCommand>,
    command_rx: mpsc::Receiver<WorkerCommand>,
    notifier: N,
) {
    thread::spawn(move || match scope {
        #[cfg(windows)]
        IndexScope::LocalDrives => run_search_worker(
            NtfsMultiVolumeSource::new(local_fixed_volumes()),
            command_tx,
            command_rx,
            notifier,
        ),
        #[cfg(not(windows))]
        IndexScope::LocalDrives => run_search_worker(
            DirectorySource::local_drives(),
            command_tx,
            command_rx,
            notifier,
        ),
        #[cfg(windows)]
        IndexScope::Root(root) if is_volume_root(&root) => run_search_worker(
            NtfsMftSource::new(root),
            command_tx,
            command_rx,
            notifier,
        ),
        IndexScope::Root(root) => run_search_worker(
            DirectorySource::new(root),
            command_tx,
            command_rx,
            notifier,
        ),
    });
}

fn run_search_worker<S: IndexSource + Send + 'static, N: WorkerNotifier>(
    source: S,
    command_tx: mpsc::Sender<WorkerCommand>,
    command_rx: mpsc::Receiver<WorkerCommand>,
    notifier: N,
) {
    let indexing_started = Instant::now();
    let mut index = match SearchIndex::from_source(&source) {
        Ok(index) => index,
        Err(error) => {
            let _ = notifier.notify(WorkerResponse::IndexingFailed(error.to_string()));
            return;
        }
    };

    if notifier
        .notify(WorkerResponse::IndexReady {
            indexed_items: index.len(),
            indexing_issues: index.indexing_issue_count(),
            indexing_duration: indexing_started.elapsed(),
            initial_results: index.search_results("", RESULT_LIMIT),
        })
        .is_err()
    {
        return;
    }

    spawn_update_poller(source, index.journal_checkpoints().to_vec(), command_tx);

    let mut latest = SearchRequest::default();
    while let Ok(command) = command_rx.recv() {
        if notifier
            .notify(handle_command(&mut index, &mut latest, command))
            .is_err()
        {
            return;
        }
    }
}

fn handle_command(
    index: &mut SearchIndex,
    latest: &mut SearchRequest,
    command: WorkerCommand,
) -> WorkerResponse {
    match command {
        WorkerCommand::Search(request) => {
            let response = SearchResponse {
                revision: request.revision,
                results: index.search_results(&request.query, RESULT_LIMIT),
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
                    results: index.search_results(&latest.query, RESULT_LIMIT),
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

pub fn is_volume_root(path: &Path) -> bool {
    let bytes = path.as_os_str().as_encoded_bytes();
    match bytes {
        [drive, b':', b'\\'] | [drive, b':', b'/'] if drive.is_ascii_alphabetic() => true,
        [drive, b':'] if drive.is_ascii_alphabetic() => true,
        [b'/'] => true,
        _ => false,
    }
}

pub const FILE_ATTRIBUTE_READONLY: u32 = 0x0000_0001;
pub const FILE_ATTRIBUTE_HIDDEN: u32 = 0x0000_0002;
pub const FILE_ATTRIBUTE_SYSTEM: u32 = 0x0000_0004;
pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
pub const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x0000_0020;
pub const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

pub fn describe_attributes(attributes: u32) -> String {
    let kind = if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        "Folder"
    } else {
        "File"
    };
    [
        (FILE_ATTRIBUTE_READONLY, "read-only"),
        (FILE_ATTRIBUTE_HIDDEN, "hidden"),
        (FILE_ATTRIBUTE_SYSTEM, "system"),
    ]
    .into_iter()
    .filter(|(flag, _)| attributes & flag != 0)
    .fold(kind.to_owned(), |description, (_, label)| {
        format!("{description}, {label}")
    })
}

pub fn split_result_path(path_str: &str) -> (String, String) {
    let trimmed = path_str.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return (path_str.to_owned(), String::new());
    }
    if let Some(pos) = trimmed.rfind(['/', '\\']) {
        let name = trimmed[pos + 1..].to_owned();
        let mut folder = trimmed[..pos].to_owned();
        if folder.ends_with(':') {
            folder.push(path_str.chars().nth(pos).unwrap_or('\\'));
        } else if folder.is_empty() && (path_str.starts_with('/') || path_str.starts_with('\\')) {
            folder.push(path_str.chars().next().unwrap_or('/'));
        }
        (name, folder)
    } else {
        (path_str.to_owned(), String::new())
    }
}
