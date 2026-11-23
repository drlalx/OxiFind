//! Multi-threaded directory crawler and index source for multiplatform filesystem indexing.

use std::{
    collections::VecDeque,
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

use crate::{
    persistence::AppSettings,
    platform::mounts::{default_local_roots, default_system_exclusions, is_path_excluded},
    search::{EntryMetadata, IndexIssue, IndexSource, IndexUpdate, JournalCheckpoint, SearchEntry},
    watcher::DirectoryWatcher,
};

// Only computed on non-Windows targets; Windows reads the native attributes.
#[cfg_attr(windows, allow(dead_code))]
const FILE_ATTRIBUTE_READONLY: u32 = 0x0000_0001;
#[cfg_attr(windows, allow(dead_code))]
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x0000_0002;
#[cfg_attr(windows, allow(dead_code))]
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
#[cfg_attr(windows, allow(dead_code))]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

/// Multi-threaded crawler that indexes directories across any platform.
#[derive(Clone)]
pub struct DirectorySource {
    roots: Vec<PathBuf>,
    exclusions: Vec<PathBuf>,
    excluded_names: Vec<String>,
    num_threads: usize,
    watcher: Arc<Mutex<Option<DirectoryWatcher>>>,
}

impl DirectorySource {
    /// Creates a directory source for a single root path with default exclusions.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self::with_roots(vec![root.into()])
    }

    /// Creates a directory source for multiple roots with default exclusions.
    pub fn with_roots(roots: Vec<PathBuf>) -> Self {
        Self {
            roots,
            exclusions: default_system_exclusions(),
            excluded_names: normalize_excluded_patterns(&AppSettings::default().excluded_patterns),
            num_threads: default_thread_count(),
            watcher: Arc::new(Mutex::new(None)),
        }
    }

    /// Creates a directory source configured to index the host's local drives or root filesystem.
    pub fn local_drives() -> Self {
        Self::with_roots(default_local_roots())
    }

    /// Replaces the exclusion paths.
    #[allow(dead_code)]
    pub fn with_exclusions(mut self, exclusions: Vec<PathBuf>) -> Self {
        self.exclusions = exclusions;
        self
    }

    /// Sets directory/pattern exclusions (always keeping `.git` and `target` excluded).
    pub fn with_excluded_patterns(mut self, patterns: &[String]) -> Self {
        self.excluded_names = normalize_excluded_patterns(patterns);
        self
    }

    /// Returns the configured index roots.
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Returns the normalized excluded directory/path patterns.
    pub fn excluded_names(&self) -> &[String] {
        &self.excluded_names
    }

    /// Sets the worker thread count.
    #[allow(dead_code)]
    pub fn with_threads(mut self, num_threads: usize) -> Self {
        self.num_threads = num_threads.max(1);
        self
    }

    /// Executes the multi-threaded crawl, appending results and issues.
    pub fn crawl_parallel(
        &self,
        out: &mut Vec<SearchEntry>,
        issues: &mut Vec<IndexIssue>,
    ) -> io::Result<()> {
        if self.roots.is_empty() {
            return Ok(());
        }

        // Validate single root existence early
        if self.roots.len() == 1 {
            let root = &self.roots[0];
            if !root.exists() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("Path does not exist: {}", root.display()),
                ));
            }
        }

        // Filter out exclusions that are ancestors of any requested root so that
        // targeting a directory inside e.g. /tmp still indexes that directory.
        let effective_exclusions: Vec<PathBuf> = self
            .exclusions
            .iter()
            .filter(|excl| !self.roots.iter().any(|root| root.starts_with(excl)))
            .cloned()
            .collect();

        let initial_dirs: VecDeque<PathBuf> = self
            .roots
            .iter()
            .filter(|root| !is_path_excluded(root, &effective_exclusions))
            .cloned()
            .collect();

        if initial_dirs.is_empty() {
            return Ok(());
        }

        let work_queue = Arc::new(WorkQueue::new(initial_dirs));
        let exclusions = Arc::new(effective_exclusions);
        let excluded_names = Arc::new(self.excluded_names.clone());
        let mut handles = Vec::with_capacity(self.num_threads);

        for _ in 0..self.num_threads {
            let queue = Arc::clone(&work_queue);
            let excl = Arc::clone(&exclusions);
            let names = Arc::clone(&excluded_names);

            let handle = thread::spawn(move || worker_loop(queue, excl, names));
            handles.push(handle);
        }

        for handle in handles {
            match handle.join() {
                Ok(worker_result) => {
                    out.extend(worker_result.entries);
                    issues.extend(worker_result.issues);
                }
                Err(_) => {
                    issues.push(IndexIssue {
                        path: PathBuf::new(),
                        message: "A crawler worker thread terminated unexpectedly".to_owned(),
                    });
                }
            }
        }

        Ok(())
    }
}

impl IndexSource for DirectorySource {
    fn start_watching(&self) -> Vec<IndexIssue> {
        let (dir_watcher, watch_issues) = DirectoryWatcher::with_exclusions_and_patterns(
            &self.roots,
            &self.exclusions,
            &self.excluded_names,
        );
        let mut guard = match self.watcher.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        *guard = Some(dir_watcher);
        watch_issues
    }

    fn collect_initial(
        &self,
        out: &mut Vec<SearchEntry>,
        issues: &mut Vec<IndexIssue>,
    ) -> io::Result<()> {
        self.crawl_parallel(out, issues)
    }

    fn collect_snapshot(
        &self,
        out: &mut Vec<SearchEntry>,
        issues: &mut Vec<IndexIssue>,
    ) -> io::Result<Vec<JournalCheckpoint>> {
        issues.extend(self.start_watching());

        self.collect_initial(out, issues)?;

        Ok(vec![JournalCheckpoint {
            volume_identity: "directory_source".to_owned(),
            journal_id: 1,
            next_usn: 1,
        }])
    }

    fn collect_updates(
        &self,
        checkpoints: &mut [JournalCheckpoint],
        out: &mut Vec<IndexUpdate>,
        issues: &mut Vec<IndexIssue>,
    ) -> io::Result<()> {
        let mut guard = match self.watcher.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if let Some(ref mut w) = *guard {
            w.drain_updates(out, issues);
            if let Some(checkpoint) = checkpoints.first_mut() {
                checkpoint.next_usn = checkpoint.next_usn.saturating_add(out.len() as i64);
            }
        }
        Ok(())
    }

    fn can_resume_from_snapshot(&self) -> bool {
        // Directory watching has no persistent journal: changes made while stopped are unknown.
        false
    }

    fn journal_checkpoints(&self) -> io::Result<Vec<JournalCheckpoint>> {
        Ok(vec![JournalCheckpoint {
            volume_identity: "directory_source".to_owned(),
            journal_id: 1,
            next_usn: 1,
        }])
    }
}

fn default_thread_count() -> usize {
    thread::available_parallelism().map_or(4, |n| n.get().clamp(2, 16))
}

struct WorkerResult {
    entries: Vec<SearchEntry>,
    issues: Vec<IndexIssue>,
}

struct WorkQueue {
    dirs: Mutex<VecDeque<PathBuf>>,
    cond: Condvar,
    active_workers: AtomicUsize,
    terminated: Mutex<bool>,
}

impl WorkQueue {
    fn new(dirs: VecDeque<PathBuf>) -> Self {
        Self {
            dirs: Mutex::new(dirs),
            cond: Condvar::new(),
            active_workers: AtomicUsize::new(0),
            terminated: Mutex::new(false),
        }
    }

    fn pop_directory(&self) -> Option<PathBuf> {
        let mut dirs = match self.dirs.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };

        loop {
            let is_term = match self.terminated.lock() {
                Ok(g) => *g,
                Err(p) => *p.into_inner(),
            };
            if is_term {
                return None;
            }

            if let Some(dir) = dirs.pop_front() {
                self.active_workers.fetch_add(1, Ordering::SeqCst);
                return Some(dir);
            }

            // Queue is empty: check if any workers are currently processing directories
            if self.active_workers.load(Ordering::SeqCst) == 0 {
                // All workers idle and queue empty: terminate work
                let mut term_guard = match self.terminated.lock() {
                    Ok(g) => g,
                    Err(p) => p.into_inner(),
                };
                *term_guard = true;
                drop(term_guard);
                self.cond.notify_all();
                return None;
            }

            // Sleep until new subdirectories are discovered or termination is signaled
            dirs = match self.cond.wait(dirs) {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
        }
    }

    fn finish_directory(&self, new_subdirs: Vec<PathBuf>) {
        let mut dirs = match self.dirs.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };

        dirs.extend(new_subdirs);
        self.active_workers.fetch_sub(1, Ordering::SeqCst);
        self.cond.notify_all();
    }
}

fn worker_loop(
    queue: Arc<WorkQueue>,
    exclusions: Arc<Vec<PathBuf>>,
    excluded_names: Arc<Vec<String>>,
) -> WorkerResult {
    let mut entries = Vec::new();
    let mut issues = Vec::new();

    while let Some(current_dir) = queue.pop_directory() {
        let mut subdirs = Vec::new();

        match fs::read_dir(&current_dir) {
            Ok(read_dir) => {
                for dir_entry in read_dir {
                    let entry = match dir_entry {
                        Ok(e) => e,
                        Err(err) => {
                            issues.push(IndexIssue {
                                path: current_dir.clone(),
                                message: err.to_string(),
                            });
                            continue;
                        }
                    };

                    let path = entry.path();
                    if should_skip_path(&path, &exclusions, &excluded_names) {
                        continue;
                    }

                    let file_type = match entry.file_type() {
                        Ok(ft) => ft,
                        Err(err) => {
                            issues.push(IndexIssue {
                                path: path.clone(),
                                message: err.to_string(),
                            });
                            continue;
                        }
                    };

                    let metadata = entry.metadata().ok();
                    let attributes = compute_attributes(&path, &file_type, metadata.as_ref());
                    let file_id = compute_file_id(metadata.as_ref());

                    let display = path.display().to_string();
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_owned();

                    entries.push(SearchEntry {
                        display: display.clone(),
                        path: display,
                        path_resolved: true,
                        metadata: EntryMetadata {
                            file_id,
                            parent_id: None,
                            name,
                            full_path: path.clone(),
                            volume_identity: None,
                            attributes,
                        },
                    });

                    // Only descend into non-symlink directories to avoid circular symlinks
                    if file_type.is_dir() && !file_type.is_symlink() {
                        subdirs.push(path);
                    }
                }
            }
            Err(err) => {
                issues.push(IndexIssue {
                    path: current_dir,
                    message: err.to_string(),
                });
            }
        }

        queue.finish_directory(subdirs);
    }

    WorkerResult { entries, issues }
}

pub(crate) fn normalize_excluded_patterns(patterns: &[String]) -> Vec<String> {
    let mut normalized = Vec::with_capacity(patterns.len() + 2);
    for default in [".git", "target"] {
        normalized.push(default.to_owned());
    }
    for pattern in patterns {
        let mut trimmed = pattern.trim();
        while let Some(rest) = trimmed.strip_prefix("*/").or_else(|| trimmed.strip_prefix('/')) {
            trimmed = rest;
        }
        while let Some(rest) = trimmed.strip_suffix("/*").or_else(|| trimmed.strip_suffix('/')) {
            trimmed = rest;
        }
        if !trimmed.is_empty() && !normalized.iter().any(|existing| existing == trimmed) {
            normalized.push(trimmed.to_owned());
        }
    }
    normalized
}

pub(crate) fn should_skip_path(
    path: &Path,
    exclusions: &[PathBuf],
    excluded_names: &[String],
) -> bool {
    if is_path_excluded(path, exclusions) {
        return true;
    }

    let has_excluded_component = path.components().any(|component| {
        let std::path::Component::Normal(name) = component else {
            return false;
        };
        let Some(name) = name.to_str() else {
            return false;
        };
        matches!(name, ".git" | "target")
            || excluded_names
                .iter()
                .any(|pattern| !pattern.contains(['/', '\\']) && pattern == name)
    });
    if has_excluded_component {
        return true;
    }

    excluded_names
        .iter()
        .any(|pattern| pattern.contains(['/', '\\']) && path.ends_with(pattern))
}

#[cfg(windows)]
pub(crate) fn compute_attributes(
    _path: &Path,
    _file_type: &fs::FileType,
    metadata: Option<&fs::Metadata>,
) -> u32 {
    use std::os::windows::fs::MetadataExt;
    if let Some(meta) = metadata {
        meta.file_attributes()
    } else {
        0
    }
}

#[cfg(not(windows))]
pub(crate) fn compute_attributes(
    path: &Path,
    file_type: &fs::FileType,
    metadata: Option<&fs::Metadata>,
) -> u32 {
    let mut attributes = 0u32;
    if file_type.is_dir() {
        attributes |= FILE_ATTRIBUTE_DIRECTORY;
    }
    if file_type.is_symlink() {
        attributes |= FILE_ATTRIBUTE_REPARSE_POINT;
    }
    let is_hidden = path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| name.starts_with('.') && name != "." && name != "..");
    if is_hidden {
        attributes |= FILE_ATTRIBUTE_HIDDEN;
    }
    if metadata.is_some_and(|meta| meta.permissions().readonly()) {
        attributes |= FILE_ATTRIBUTE_READONLY;
    }
    attributes
}

pub(crate) fn compute_file_id(metadata: Option<&fs::Metadata>) -> Option<u128> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.map(|meta| ((meta.dev() as u128) << 64) | (meta.ino() as u128))
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, File},
        io::Write,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    #[cfg_attr(windows, allow(unused_imports))]
    use super::{DirectorySource, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN};
    use crate::search::IndexSource;

    struct TemporaryDirectory {
        path: PathBuf,
    }

    impl TemporaryDirectory {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "oxifind-crawler-test-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
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

    fn create_test_tree() -> TemporaryDirectory {
        let temp_dir = TemporaryDirectory::new();
        let root = temp_dir.path();

        let sub = root.join("nested_folder");
        fs::create_dir(&sub).expect("create_dir");

        let file1 = root.join("regular.txt");
        let mut f1 = File::create(&file1).expect("create file1");
        f1.write_all(b"hello world").expect("write file1");

        let file2 = sub.join("child.dat");
        let mut f2 = File::create(&file2).expect("create file2");
        f2.write_all(b"child").expect("write file2");

        let dotfile = root.join(".hidden_config");
        let mut f3 = File::create(&dotfile).expect("create dotfile");
        f3.write_all(b"secret").expect("write dotfile");

        let git_dir = root.join(".git");
        fs::create_dir(&git_dir).expect("create .git");
        let git_file = git_dir.join("config");
        File::create(&git_file).expect("create git config");

        temp_dir
    }

    #[test]
    fn crawls_directory_recursively_and_indexes_entries() {
        let temp = create_test_tree();
        let source = DirectorySource::new(temp.path()).with_threads(2);

        let mut entries = Vec::new();
        let mut issues = Vec::new();
        source.collect_initial(&mut entries, &mut issues).unwrap();

        assert!(issues.is_empty(), "Issues encountered: {issues:?}");
        assert!(entries.len() >= 4);

        let names: Vec<String> = entries.iter().map(|e| e.metadata.name.clone()).collect();
        assert!(names.contains(&"nested_folder".to_owned()));
        assert!(names.contains(&"regular.txt".to_owned()));
        assert!(names.contains(&"child.dat".to_owned()));
        assert!(names.contains(&".hidden_config".to_owned()));
        // .git and its contents must be skipped
        assert!(!names.contains(&".git".to_owned()));
        assert!(!names.contains(&"config".to_owned()));
    }

    #[test]
    fn assigns_correct_attributes_for_directories_and_dotfiles() {
        let temp = create_test_tree();
        let source = DirectorySource::new(temp.path());

        let mut entries = Vec::new();
        let mut issues = Vec::new();
        source.collect_initial(&mut entries, &mut issues).unwrap();

        let folder = entries
            .iter()
            .find(|e| e.metadata.name == "nested_folder")
            .unwrap();
        assert_ne!(folder.metadata.attributes & FILE_ATTRIBUTE_DIRECTORY, 0);

        #[cfg(not(windows))]
        {
            let hidden = entries
                .iter()
                .find(|e| e.metadata.name == ".hidden_config")
                .unwrap();
            assert_ne!(hidden.metadata.attributes & FILE_ATTRIBUTE_HIDDEN, 0);
        }
    }

    #[test]
    fn respects_custom_exclusions() {
        let temp = create_test_tree();
        let custom_excl = vec![temp.path().join("nested_folder")];
        let source = DirectorySource::new(temp.path()).with_exclusions(custom_excl);

        let mut entries = Vec::new();
        let mut issues = Vec::new();
        source.collect_initial(&mut entries, &mut issues).unwrap();

        let names: Vec<String> = entries.iter().map(|e| e.metadata.name.clone()).collect();
        assert!(!names.contains(&"nested_folder".to_owned()));
        assert!(!names.contains(&"child.dat".to_owned()));
        assert!(names.contains(&"regular.txt".to_owned()));
    }

    #[test]
    fn fails_on_nonexistent_single_root() {
        let missing = PathBuf::from("/nonexistent/path/12345");
        let source = DirectorySource::new(missing);

        let mut entries = Vec::new();
        let mut issues = Vec::new();
        let result = source.collect_initial(&mut entries, &mut issues);
        assert!(result.is_err());
    }

    #[test]
    fn directory_snapshots_are_not_resumed() {
        let temp = TemporaryDirectory::new();
        let source = DirectorySource::new(temp.path());
        assert!(!source.can_resume_from_snapshot());
    }

    #[test]
    fn start_watching_reports_changes_without_full_crawl() {
        let temp = TemporaryDirectory::new();
        let source = DirectorySource::new(temp.path());
        let _ = source.start_watching();

        fs::write(temp.path().join("created_later.txt"), "new").unwrap();

        let mut checkpoints = source.journal_checkpoints().unwrap();
        let mut updates = Vec::new();
        let mut issues = Vec::new();
        for _ in 0..50 {
            source
                .collect_updates(&mut checkpoints, &mut updates, &mut issues)
                .unwrap();
            if !updates.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        assert!(!updates.is_empty(), "expected the new file to be reported");
    }

    #[test]
    fn skips_default_and_glob_excluded_patterns() {
        let temp = TemporaryDirectory::new();
        let root = temp.path();

        for dir in ["node_modules", ".cache", ".stversions", "dist", "keep"] {
            let dir_path = root.join(dir);
            fs::create_dir_all(&dir_path).unwrap();
            fs::write(dir_path.join(format!("{dir}_file.txt")), "x").unwrap();
        }

        let source = DirectorySource::new(root)
            .with_excluded_patterns(&[
                "node_modules".to_owned(),
                ".cache".to_owned(),
                ".stversions".to_owned(),
                "*/dist/*".to_owned(),
            ]);

        let mut entries = Vec::new();
        let mut issues = Vec::new();
        source.collect_initial(&mut entries, &mut issues).unwrap();

        let names: Vec<&str> = entries.iter().map(|e| e.metadata.name.as_str()).collect();
        assert!(names.contains(&"keep"));
        assert!(names.contains(&"keep_file.txt"));
        for skipped in [
            "node_modules",
            "node_modules_file.txt",
            ".cache",
            ".cache_file.txt",
            ".stversions",
            ".stversions_file.txt",
            "dist",
            "dist_file.txt",
        ] {
            assert!(!names.contains(&skipped), "expected {skipped} to be skipped");
        }
        assert!(super::should_skip_path(
            &root.join("node_modules").join("deep").join("ignored.js"),
            &[],
            &["node_modules".to_owned()],
        ));
    }
}
