use std::{
    fmt, io,
    ops::ControlFlow,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[cfg(windows)]
use crate::ntfs::{NtfsMftSource, NtfsMultiVolumeSource, local_fixed_volumes};
use crate::{
    crawler::DirectorySource,
    persistence::AppSettings,
    search::{IndexSource, JournalCheckpoint, SearchIndex, SearchResult, UpdateBatch},
};

pub const RESULT_LIMIT: usize = 200;
pub const UPDATE_POLL_INTERVAL: Duration = Duration::from_secs(1);
pub const SNAPSHOT_QUIET_DELAY: Duration = Duration::from_secs(15);
pub const SNAPSHOT_MIN_SAVE_INTERVAL: Duration = Duration::from_secs(60);
pub const SNAPSHOT_MAX_DIRTY_AGE: Duration = Duration::from_secs(300);

/// Throttles snapshot serialization so bursts of filesystem events do not trigger 1s disk rewrites.
#[derive(Clone, Copy, Debug)]
pub struct SnapshotSavePolicy {
    dirty_since: Option<Instant>,
    last_change: Option<Instant>,
    last_saved: Instant,
}

impl SnapshotSavePolicy {
    pub fn new(now: Instant) -> Self {
        Self {
            dirty_since: None,
            last_change: None,
            last_saved: now,
        }
    }

    pub fn mark_dirty(&mut self, now: Instant) {
        if self.dirty_since.is_none() {
            self.dirty_since = Some(now);
        }
        self.last_change = Some(now);
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty_since.is_some()
    }

    pub fn should_save(&self, now: Instant) -> bool {
        let (Some(first_dirty), Some(last_change)) = (self.dirty_since, self.last_change) else {
            return false;
        };
        if now.saturating_duration_since(first_dirty) >= SNAPSHOT_MAX_DIRTY_AGE {
            return true;
        }
        now.saturating_duration_since(last_change) >= SNAPSHOT_QUIET_DELAY
            && now.saturating_duration_since(self.last_saved) >= SNAPSHOT_MIN_SAVE_INTERVAL
    }

    pub fn mark_saved(&mut self, now: Instant) {
        self.dirty_since = None;
        self.last_change = None;
        self.last_saved = now;
    }
}

/// Loads persisted settings, falling back to defaults on read or parse errors.
pub fn load_runtime_settings() -> AppSettings {
    let path = crate::persistence::default_settings_path();
    match crate::persistence::load_settings(&path) {
        Ok(settings) => settings,
        Err(err) => {
            eprintln!(
                "Warning: Failed to load settings from {}: {err}",
                path.display()
            );
            AppSettings::default()
        }
    }
}

/// Constructs a `DirectorySource` from the requested scope and persisted settings.
pub fn build_directory_source(scope: &IndexScope, settings: &AppSettings) -> DirectorySource {
    let source = match scope {
        IndexScope::LocalDrives if !settings.indexed_paths.is_empty() => {
            DirectorySource::with_roots(settings.indexed_paths.clone())
        }
        IndexScope::LocalDrives => DirectorySource::local_drives(),
        IndexScope::Root(root) => DirectorySource::new(root.clone()),
    };
    source.with_excluded_patterns(&settings.excluded_patterns)
}

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
            #[cfg(windows)]
            Self::LocalDrives => formatter.write_str("all local drives"),
            #[cfg(not(windows))]
            Self::LocalDrives => formatter.write_str("all local filesystems"),
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
    let snapshot_path = match &scope {
        IndexScope::LocalDrives => Some(crate::persistence::default_snapshot_path()),
        _ => None,
    };
    spawn_search_worker_with_snapshot(scope, command_tx, command_rx, notifier, snapshot_path);
}

pub fn spawn_search_worker_with_snapshot<N: WorkerNotifier>(
    scope: IndexScope,
    command_tx: mpsc::Sender<WorkerCommand>,
    command_rx: mpsc::Receiver<WorkerCommand>,
    notifier: N,
    snapshot_path: Option<PathBuf>,
) {
    let settings = load_runtime_settings();
    thread::spawn(move || match scope {
        #[cfg(windows)]
        IndexScope::LocalDrives => run_search_worker(
            NtfsMultiVolumeSource::new(local_fixed_volumes()),
            command_tx,
            command_rx,
            notifier,
            snapshot_path,
        ),
        #[cfg(not(windows))]
        IndexScope::LocalDrives => run_search_worker(
            build_directory_source(&IndexScope::LocalDrives, &settings),
            command_tx,
            command_rx,
            notifier,
            snapshot_path,
        ),
        #[cfg(windows)]
        IndexScope::Root(root) if is_volume_root(&root) => run_search_worker(
            NtfsMftSource::new(root),
            command_tx,
            command_rx,
            notifier,
            snapshot_path,
        ),
        IndexScope::Root(root) => run_search_worker(
            build_directory_source(&IndexScope::Root(root), &settings),
            command_tx,
            command_rx,
            notifier,
            snapshot_path,
        ),
    });
}

fn run_search_worker<S: IndexSource + Send + 'static, N: WorkerNotifier>(
    source: S,
    command_tx: mpsc::Sender<WorkerCommand>,
    command_rx: mpsc::Receiver<WorkerCommand>,
    notifier: N,
    snapshot_path: Option<PathBuf>,
) {
    let indexing_started = Instant::now();
    let mut loaded_from_cache = false;

    let mut index = if let Some(ref path) = snapshot_path
        && source.can_resume_from_snapshot()
    {
        match crate::persistence::load_snapshot(path) {
            Ok(Some(persisted)) => {
                loaded_from_cache = true;
                SearchIndex::from_persisted(persisted.entries, persisted.checkpoints)
            }
            _ => match SearchIndex::from_source(&source) {
                Ok(index) => index,
                Err(error) => {
                    let _ = notifier.notify(WorkerResponse::IndexingFailed(error.to_string()));
                    return;
                }
            },
        }
    } else {
        match SearchIndex::from_source(&source) {
            Ok(index) => index,
            Err(error) => {
                let _ = notifier.notify(WorkerResponse::IndexingFailed(error.to_string()));
                return;
            }
        }
    };

    if let Some(ref path) = snapshot_path
        && !loaded_from_cache
    {
        let _ =
            crate::persistence::save_snapshot(path, index.journal_checkpoints(), index.entries());
    }

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
    let mut save_policy = SnapshotSavePolicy::new(Instant::now());
    while let Ok(mut command) = command_rx.recv() {
        if let WorkerCommand::Search(_) = command {
            while let Ok(next) = command_rx.try_recv() {
                match next {
                    WorkerCommand::Search(newer_request) => {
                        command = WorkerCommand::Search(newer_request);
                    }
                    other => {
                        let was_update = matches!(command, WorkerCommand::ApplyUpdates(_));
                        if notifier
                            .notify(handle_command(&mut index, &mut latest, command))
                            .is_err()
                        {
                            return;
                        }
                        if was_update {
                            save_policy.mark_dirty(Instant::now());
                        }
                        command = other;
                        break;
                    }
                }
            }
        }
        let was_update = matches!(command, WorkerCommand::ApplyUpdates(_));
        if notifier
            .notify(handle_command(&mut index, &mut latest, command))
            .is_err()
        {
            return;
        }
        let now = Instant::now();
        if was_update {
            save_policy.mark_dirty(now);
        }
        if let Some(ref path) = snapshot_path
            && save_policy.should_save(now)
            && crate::persistence::save_snapshot(path, index.journal_checkpoints(), index.entries())
                .is_ok()
        {
            save_policy.mark_saved(now);
        }
    }

    if let Some(ref path) = snapshot_path
        && save_policy.is_dirty()
    {
        let _ =
            crate::persistence::save_snapshot(path, index.journal_checkpoints(), index.entries());
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

pub fn run_service(
    scope: IndexScope,
    delay_seconds: u32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("==> OxiFind Background Service initializing...");

    if delay_seconds > 0 {
        println!(
            "==> Startup delay: waiting {delay_seconds} seconds before indexing / monitoring..."
        );
        thread::sleep(Duration::from_secs(delay_seconds as u64));
    }

    let settings = load_runtime_settings();

    // One source instance is used for the initial load, the watcher and polling: the watcher
    // state lives in the instance, so a second instance would never report any change.
    #[cfg(windows)]
    let source: Box<dyn IndexSource> = match &scope {
        IndexScope::LocalDrives => Box::new(NtfsMultiVolumeSource::new(local_fixed_volumes())),
        IndexScope::Root(root) if is_volume_root(root) => {
            Box::new(NtfsMftSource::new(root.clone()))
        }
        IndexScope::Root(_) => Box::new(build_directory_source(&scope, &settings)),
    };
    #[cfg(not(windows))]
    let source: Box<dyn IndexSource> = {
        let dir_source = build_directory_source(&scope, &settings);
        let root_list: Vec<String> = dir_source
            .roots()
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        println!("==> Configured roots: {}", root_list.join(", "));
        Box::new(dir_source)
    };

    let snapshot_path = crate::persistence::default_snapshot_path();

    println!(
        "==> Checking for existing database snapshot at {}...",
        snapshot_path.display()
    );
    let mut index = match crate::persistence::load_snapshot(&snapshot_path) {
        Ok(Some(persisted)) if source.can_resume_from_snapshot() => {
            println!(
                "==> Reusing existing database with {} items (timestamp: {}).",
                persisted.entries.len(),
                persisted.timestamp_secs
            );
            for issue in source.start_watching() {
                eprintln!("Warning: {}: {}", issue.path.display(), issue.message);
            }
            SearchIndex::from_persisted(persisted.entries, persisted.checkpoints)
        }
        other => {
            if let Ok(Some(persisted)) = &other {
                println!(
                    "==> Existing snapshot with {} items cannot be reconciled for this source; rebuilding.",
                    persisted.entries.len()
                );
            }
            println!("==> Starting full filesystem crawl for scope '{scope}'...");

            let idx = SearchIndex::from_source(&source)?;
            println!(
                "==> Full scan complete: indexed {} items ({} issue(s)).",
                idx.len(),
                idx.indexing_issue_count()
            );
            crate::persistence::save_snapshot(
                &snapshot_path,
                idx.journal_checkpoints(),
                idx.entries(),
            )?;
            idx
        }
    };

    println!("==> Background service active: monitoring filesystem for changes...");

    let mut checkpoints = index.journal_checkpoints().to_vec();
    let mut save_policy = SnapshotSavePolicy::new(Instant::now());

    loop {
        thread::sleep(UPDATE_POLL_INTERVAL);
        match poll_updates(&source, &mut checkpoints) {
            ControlFlow::Break(()) => {
                if save_policy.is_dirty() {
                    let _ = crate::persistence::save_snapshot(
                        &snapshot_path,
                        index.journal_checkpoints(),
                        index.entries(),
                    );
                }
                println!("==> Source update polling completed.");
                break;
            }
            ControlFlow::Continue(Some(WorkerCommand::ApplyUpdates(batch))) => {
                let batch_size = batch.updates.len();
                index.apply(batch);
                if batch_size > 0 {
                    save_policy.mark_dirty(Instant::now());
                    println!(
                        "==> Applied {batch_size} change(s). In-memory index: {} items.",
                        index.len()
                    );
                }
            }
            ControlFlow::Continue(Some(WorkerCommand::UpdateFailed(msg))) => {
                eprintln!("Warning: {msg}");
            }
            ControlFlow::Continue(None) => {}
            ControlFlow::Continue(Some(WorkerCommand::Search(_))) => {}
        }

        let now = Instant::now();
        if save_policy.should_save(now) {
            match crate::persistence::save_snapshot(
                &snapshot_path,
                index.journal_checkpoints(),
                index.entries(),
            ) {
                Ok(()) => {
                    save_policy.mark_saved(now);
                    println!("==> Persisted database snapshot ({} items).", index.len());
                }
                Err(e) => {
                    eprintln!("Warning: Failed to save updated snapshot: {e}");
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn skips_stale_search_commands_in_worker() {
        let (command_tx, command_rx) = mpsc::channel();
        let (response_tx, response_rx) = mpsc::channel();

        let temp_dir =
            std::env::temp_dir().join(format!("oxifind-eng-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("testfile.txt");
        let _ = std::fs::write(&file_path, "hello");

        spawn_search_worker(
            IndexScope::Root(temp_dir.clone()),
            command_tx.clone(),
            command_rx,
            response_tx,
        );

        let mut ready = false;
        while let Ok(resp) = response_rx.recv() {
            if let WorkerResponse::IndexReady { .. } = resp {
                ready = true;
                break;
            }
        }
        assert!(ready);

        let _ = command_tx.send(WorkerCommand::Search(SearchRequest {
            revision: 1,
            query: "t".to_owned(),
        }));
        let _ = command_tx.send(WorkerCommand::Search(SearchRequest {
            revision: 2,
            query: "te".to_owned(),
        }));
        let _ = command_tx.send(WorkerCommand::Search(SearchRequest {
            revision: 3,
            query: "test".to_owned(),
        }));

        let mut last_rev = 0;
        while let Ok(resp) = response_rx.recv() {
            if let WorkerResponse::SearchResults(SearchResponse { revision, .. }) = resp {
                last_rev = revision;
                if revision == 3 {
                    break;
                }
            }
        }
        assert_eq!(last_rev, 3);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn formats_index_scope_display_appropriately_for_platform() {
        #[cfg(windows)]
        assert_eq!(format!("{}", IndexScope::LocalDrives), "all local drives");
        #[cfg(not(windows))]
        assert_eq!(
            format!("{}", IndexScope::LocalDrives),
            "all local filesystems"
        );

        assert_eq!(
            format!("{}", IndexScope::Root(PathBuf::from("/test/path"))),
            "/test/path"
        );
    }

    #[test]
    fn snapshot_save_policy_throttles_bursts_and_flushes_after_quiet_or_max_age() {
        let start = Instant::now();
        let mut policy = SnapshotSavePolicy::new(start);
        assert!(!policy.is_dirty());
        assert!(!policy.should_save(start + Duration::from_secs(600)));

        // Continuous burst updates within quiet window & min save interval stay deferred.
        policy.mark_dirty(start + Duration::from_secs(1));
        assert!(policy.is_dirty());
        assert!(!policy.should_save(start + Duration::from_secs(5)));
        assert!(!policy.should_save(start + Duration::from_secs(20))); // < min save interval (60s)

        // After quiet delay AND min save interval, save is allowed.
        assert!(policy.should_save(start + Duration::from_secs(65)));
        policy.mark_saved(start + Duration::from_secs(65));
        assert!(!policy.is_dirty());

        // Continuous updates with no quiet gap still flush once max dirty age (300s) is reached.
        let t1 = start + Duration::from_secs(70);
        policy.mark_dirty(t1);
        for step in (75..=365).step_by(10) {
            let now = start + Duration::from_secs(step);
            policy.mark_dirty(now);
            assert!(!policy.should_save(now), "saved too early at +{step}s");
        }
        assert!(policy.should_save(start + Duration::from_secs(370)));
    }

    #[test]
    fn build_directory_source_applies_settings_paths_and_exclusions() {
        let settings = crate::persistence::AppSettings {
            indexed_paths: vec![
                PathBuf::from("/tmp/oxifind-a"),
                PathBuf::from("/tmp/oxifind-b"),
            ],
            excluded_patterns: vec!["node_modules".to_owned(), "*/.cache/*".to_owned()],
            ..crate::persistence::AppSettings::default()
        };

        let local_source = build_directory_source(&IndexScope::LocalDrives, &settings);
        assert_eq!(
            local_source.roots(),
            &[
                PathBuf::from("/tmp/oxifind-a"),
                PathBuf::from("/tmp/oxifind-b")
            ]
        );
        assert!(
            local_source
                .excluded_names()
                .contains(&"node_modules".to_owned())
        );
        assert!(local_source.excluded_names().contains(&".cache".to_owned()));

        let root_source =
            build_directory_source(&IndexScope::Root(PathBuf::from("/tmp/explicit")), &settings);
        assert_eq!(root_source.roots(), &[PathBuf::from("/tmp/explicit")]);
        assert!(root_source.excluded_names().contains(&".cache".to_owned()));
    }
}
