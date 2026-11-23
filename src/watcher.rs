//! Cross-platform filesystem watcher using the `notify` crate for live index updates.

use std::{
    collections::{HashSet, VecDeque},
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{Receiver, channel},
    },
};

use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::{
    crawler::{compute_attributes, compute_file_id, normalize_excluded_patterns, should_skip_path},
    search::{EntryMetadata, IndexIssue, IndexUpdate, SearchEntry},
};

pub struct DirectoryWatcher {
    watcher: Option<RecommendedWatcher>,
    rx: Receiver<notify::Result<Event>>,
    exclusions: Arc<Vec<PathBuf>>,
    excluded_names: Arc<Vec<String>>,
    watched_dirs: HashSet<PathBuf>,
    max_watches: usize,
    watch_limit_reported: bool,
}

/// Reserves half of the OS inotify watch limit for other tools (Syncthing, Tracker, IDEs).
pub(crate) fn default_watch_budget(system_limit: Option<usize>) -> usize {
    match system_limit {
        Some(0) => 1,
        Some(limit) if limit <= 1024 => limit,
        Some(limit) => (limit / 2).max(1024),
        None => 32_768,
    }
}

fn read_system_max_user_watches() -> Option<usize> {
    #[cfg(target_os = "linux")]
    {
        fs::read_to_string("/proc/sys/fs/inotify/max_user_watches")
            .ok()?
            .trim()
            .parse::<usize>()
            .ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

fn is_watch_limit_error(err: &notify::Error) -> bool {
    matches!(err.kind, notify::ErrorKind::MaxFilesWatch)
        || matches!(
            &err.kind,
            notify::ErrorKind::Io(io_err)
                if io_err.kind() == io::ErrorKind::StorageFull
                    || io_err.raw_os_error() == Some(28)
        )
}

fn build_search_entry(path: PathBuf, file_type: &fs::FileType, metadata: &fs::Metadata) -> SearchEntry {
    let attributes = compute_attributes(&path, file_type, Some(metadata));
    let file_id = compute_file_id(Some(metadata));
    let display = path.display().to_string();
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_owned();

    SearchEntry {
        display: display.clone(),
        path: display,
        path_resolved: true,
        metadata: EntryMetadata {
            file_id,
            parent_id: None,
            name,
            full_path: path,
            volume_identity: None,
            attributes,
        },
    }
}

impl DirectoryWatcher {
    /// Creates a directory watcher on the given root directories with default watch limits.
    #[allow(dead_code)]
    pub fn new(roots: &[PathBuf], exclusions: &[PathBuf]) -> (Self, Vec<IndexIssue>) {
        Self::with_exclusions_and_patterns(roots, exclusions, &[])
    }

    /// Creates a directory watcher respecting path and pattern exclusions with the OS watch budget.
    pub fn with_exclusions_and_patterns(
        roots: &[PathBuf],
        exclusions: &[PathBuf],
        excluded_patterns: &[String],
    ) -> (Self, Vec<IndexIssue>) {
        let budget = default_watch_budget(read_system_max_user_watches());
        Self::with_options(roots, exclusions, excluded_patterns, budget)
    }

    /// Creates a directory watcher respecting path exclusions, pattern exclusions, and a watch budget.
    pub fn with_options(
        roots: &[PathBuf],
        exclusions: &[PathBuf],
        excluded_patterns: &[String],
        max_watches: usize,
    ) -> (Self, Vec<IndexIssue>) {
        let (tx, rx) = channel();
        let mut issues = Vec::new();

        let watcher_res = RecommendedWatcher::new(
            move |res| {
                let _ = tx.send(res);
            },
            Config::default(),
        );

        let watcher = match watcher_res {
            Ok(w) => Some(w),
            Err(err) => {
                issues.push(IndexIssue {
                    path: PathBuf::new(),
                    message: format!("Failed to initialize filesystem watcher: {err}"),
                });
                None
            }
        };

        let effective_exclusions: Arc<Vec<PathBuf>> = Arc::new(
            exclusions
                .iter()
                .filter(|excl| !roots.iter().any(|root| root.starts_with(excl)))
                .cloned()
                .collect(),
        );
        let excluded_names = Arc::new(normalize_excluded_patterns(excluded_patterns));

        let mut instance = Self {
            watcher,
            rx,
            exclusions: effective_exclusions,
            excluded_names,
            watched_dirs: HashSet::new(),
            max_watches: max_watches.max(1),
            watch_limit_reported: false,
        };

        instance.watch_initial_roots(roots, &mut issues);

        (instance, issues)
    }

    /// Returns the number of directories currently tracked by individual watches.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn watched_dir_count(&self) -> usize {
        self.watched_dirs.len()
    }

    fn watch_initial_roots(&mut self, roots: &[PathBuf], issues: &mut Vec<IndexIssue>) {
        #[cfg(target_os = "linux")]
        {
            for root in roots {
                if !root.exists() {
                    continue;
                }
                self.watch_linux_tree(root, None, issues);
                if self.watch_limit_reported {
                    break;
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            if let Some(ref mut w) = self.watcher {
                for root in roots {
                    if !root.exists() {
                        continue;
                    }
                    if let Err(err) = w.watch(root, RecursiveMode::Recursive) {
                        issues.push(IndexIssue {
                            path: root.clone(),
                            message: format!("Could not watch directory {}: {err}", root.display()),
                        });
                    }
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn watch_linux_tree(
        &mut self,
        start_dir: &Path,
        mut collect_children: Option<&mut Vec<IndexUpdate>>,
        issues: &mut Vec<IndexIssue>,
    ) {
        let mut queue = VecDeque::new();
        queue.push_back(start_dir.to_path_buf());
        let is_collecting = collect_children.is_some();

        while let Some(dir) = queue.pop_front() {
            if dir != start_dir && should_skip_path(&dir, &self.exclusions, &self.excluded_names) {
                continue;
            }
            if !self.watched_dirs.contains(&dir) {
                if self.watched_dirs.len() >= self.max_watches {
                    self.report_watch_limit(
                        &dir,
                        format!(
                            "Watch budget reached ({} directories); remaining directories will not be live-watched",
                            self.max_watches
                        ),
                        issues,
                    );
                    if !is_collecting {
                        break;
                    }
                } else if !self.watch_limit_reported
                    && let Some(ref mut w) = self.watcher
                {
                    match w.watch(&dir, RecursiveMode::NonRecursive) {
                        Ok(()) => {
                            self.watched_dirs.insert(dir.clone());
                        }
                        Err(err) if is_watch_limit_error(&err) => {
                            self.report_watch_limit(
                                &dir,
                                format!(
                                    "Filesystem watch limit reached at {} ({err}); remaining directories will not be live-watched",
                                    dir.display()
                                ),
                                issues,
                            );
                            if !is_collecting {
                                break;
                            }
                        }
                        Err(err) => {
                            issues.push(IndexIssue {
                                path: dir.clone(),
                                message: format!("Could not watch directory {}: {err}", dir.display()),
                            });
                            continue;
                        }
                    }
                }
            }

            let Ok(read_dir) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in read_dir.flatten() {
                let entry_path = entry.path();
                if should_skip_path(&entry_path, &self.exclusions, &self.excluded_names) {
                    continue;
                }
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if let Some(ref mut updates) = collect_children
                    && let Ok(meta) = entry.metadata()
                {
                    updates.push(IndexUpdate::Upsert(build_search_entry(
                        entry_path.clone(),
                        &file_type,
                        &meta,
                    )));
                }
                if file_type.is_dir() && !file_type.is_symlink() {
                    queue.push_back(entry_path);
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn report_watch_limit(&mut self, path: &Path, message: String, issues: &mut Vec<IndexIssue>) {
        if !self.watch_limit_reported {
            self.watch_limit_reported = true;
            issues.push(IndexIssue {
                path: path.to_path_buf(),
                message,
            });
        }
    }

    /// Drains any received file events and converts them into IndexUpdate operations.
    pub fn drain_updates(
        &mut self,
        out_updates: &mut Vec<IndexUpdate>,
        out_issues: &mut Vec<IndexIssue>,
    ) {
        let mut touched_paths = HashSet::new();

        while let Ok(event_res) = self.rx.try_recv() {
            let event = match event_res {
                Ok(e) => e,
                Err(err) => {
                    out_issues.push(IndexIssue {
                        path: PathBuf::new(),
                        message: format!("Filesystem watcher error: {err}"),
                    });
                    continue;
                }
            };

            for path in event.paths {
                if should_skip_path(&path, &self.exclusions, &self.excluded_names) {
                    continue;
                }
                touched_paths.insert(path);
            }
        }

        // Deduplicate and process each touched path once per batch
        for path in touched_paths {
            if path.exists() {
                if let Ok(metadata) = fs::symlink_metadata(&path) {
                    let file_type = metadata.file_type();
                    out_updates.push(IndexUpdate::Upsert(build_search_entry(
                        path.clone(),
                        &file_type,
                        &metadata,
                    )));
                    #[cfg(target_os = "linux")]
                    if file_type.is_dir()
                        && !file_type.is_symlink()
                        && !self.watched_dirs.contains(&path)
                    {
                        self.watch_linux_tree(&path, Some(out_updates), out_issues);
                    }
                }
            } else {
                // Item removed or renamed away
                #[cfg(target_os = "linux")]
                self.watched_dirs.retain(|watched| !watched.starts_with(&path));
                out_updates.push(IndexUpdate::RemovePath(path.display().to_string()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, File},
        io::Write,
        path::{Path, PathBuf},
        thread,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use super::DirectoryWatcher;
    use crate::search::IndexUpdate;

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!("oxifind-watch-test-{unique}"));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            if self.path.is_dir() {
                let _ = fs::remove_dir_all(&self.path);
            }
        }
    }

    #[test]
    fn detects_file_creation_and_deletion_events() {
        let temp = TestDir::new();
        let (mut watcher, issues) = DirectoryWatcher::new(&[temp.path().to_path_buf()], &[]);
        assert!(issues.is_empty(), "Watcher init issues: {issues:?}");

        // Create a new file
        let file_path = temp.path().join("watched_doc.txt");
        {
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "hello watcher").unwrap();
        }

        // Give the OS filesystem event subsystem a moment to dispatch
        thread::sleep(Duration::from_millis(150));

        let mut updates = Vec::new();
        let mut drain_issues = Vec::new();
        watcher.drain_updates(&mut updates, &mut drain_issues);

        let created = updates.iter().any(|u| match u {
            IndexUpdate::Upsert(entry) => entry.metadata.name == "watched_doc.txt",
            _ => false,
        });
        assert!(
            created,
            "Did not receive Upsert for created file in {updates:?}"
        );

        // Now delete the file
        fs::remove_file(&file_path).unwrap();
        thread::sleep(Duration::from_millis(150));

        let mut delete_updates = Vec::new();
        watcher.drain_updates(&mut delete_updates, &mut drain_issues);

        let deleted = delete_updates.iter().any(|u| match u {
            IndexUpdate::RemovePath(p) => p.contains("watched_doc.txt"),
            _ => false,
        });
        assert!(
            deleted,
            "Did not receive RemovePath for deleted file in {delete_updates:?}"
        );
    }

    #[test]
    fn default_watch_budget_reserves_half_of_system_limit() {
        assert_eq!(super::default_watch_budget(Some(65_536)), 32_768);
        assert_eq!(super::default_watch_budget(Some(524_288)), 262_144);
        assert_eq!(super::default_watch_budget(Some(100)), 100);
        assert_eq!(super::default_watch_budget(Some(0)), 1);
        assert_eq!(super::default_watch_budget(None), 32_768);
    }

    #[test]
    fn skips_excluded_directories_and_watches_new_nested_subdirs() {
        let temp = TestDir::new();
        let root = temp.path();
        let skipped = root.join("node_modules");
        fs::create_dir_all(&skipped).unwrap();
        let watched = root.join("src");
        fs::create_dir_all(&watched).unwrap();

        let (mut watcher, issues) = DirectoryWatcher::with_options(
            &[root.to_path_buf()],
            &[],
            &["node_modules".to_owned()],
            1000,
        );
        assert!(issues.is_empty(), "Watcher init issues: {issues:?}");
        #[cfg(target_os = "linux")]
        assert_eq!(watcher.watched_dir_count(), 2); // root + src only, NOT node_modules

        fs::write(skipped.join("ignored.js"), "ignored").unwrap();
        let nested = watched.join("nested").join("deep");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("live.rs"), "fn main() {}").unwrap();

        let mut updates = Vec::new();
        let mut drain_issues = Vec::new();
        for _ in 0..30 {
            thread::sleep(Duration::from_millis(50));
            watcher.drain_updates(&mut updates, &mut drain_issues);
            if updates.iter().any(|u| match u {
                IndexUpdate::Upsert(e) => e.metadata.name == "live.rs",
                _ => false,
            }) {
                break;
            }
        }

        assert!(
            updates.iter().any(|u| matches!(u, IndexUpdate::Upsert(e) if e.metadata.name == "live.rs")),
            "expected newly created nested file to be detected: {updates:?}"
        );
        assert!(
            !updates.iter().any(|u| matches!(u, IndexUpdate::Upsert(e) if e.metadata.name == "ignored.js")),
            "excluded directory file must not be emitted: {updates:?}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn stops_gracefully_with_single_issue_when_watch_budget_is_reached() {
        let temp = TestDir::new();
        let root = temp.path();
        for idx in 0..5 {
            fs::create_dir_all(root.join(format!("dir_{idx}"))).unwrap();
        }

        let (watcher, issues) =
            DirectoryWatcher::with_options(&[root.to_path_buf()], &[], &[], 2);

        assert_eq!(watcher.watched_dir_count(), 2);
        assert_eq!(issues.len(), 1, "expected exactly 1 budget warning: {issues:?}");
    }
}
