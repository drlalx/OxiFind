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
    platform::mounts::{default_local_roots, default_system_exclusions, is_path_excluded},
    search::{
        EntryMetadata, IndexIssue, IndexSource, IndexUpdate, JournalCheckpoint, SearchEntry,
    },
    watcher::DirectoryWatcher,
};

const FILE_ATTRIBUTE_READONLY: u32 = 0x0000_0001;
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x0000_0002;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

/// Multi-threaded crawler that indexes directories across any platform.
#[derive(Clone)]
pub struct DirectorySource {
    roots: Vec<PathBuf>,
    exclusions: Vec<PathBuf>,
    num_threads: usize,
    watcher: Arc<Mutex<Option<DirectoryWatcher>>>,
}

impl DirectorySource {
    /// Creates a directory source for a single root path with default exclusions.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            roots: vec![root.into()],
            exclusions: default_system_exclusions(),
            num_threads: default_thread_count(),
            watcher: Arc::new(Mutex::new(None)),
        }
    }

    /// Creates a directory source for multiple roots with default exclusions.
    #[allow(dead_code)]
    pub fn with_roots(roots: Vec<PathBuf>) -> Self {
        Self {
            roots,
            exclusions: default_system_exclusions(),
            num_threads: default_thread_count(),
            watcher: Arc::new(Mutex::new(None)),
        }
    }

    /// Creates a directory source configured to index the host's local drives or root filesystem.
    pub fn local_drives() -> Self {
        Self {
            roots: default_local_roots(),
            exclusions: default_system_exclusions(),
            num_threads: default_thread_count(),
            watcher: Arc::new(Mutex::new(None)),
        }
    }

    /// Replaces the exclusion paths.
    #[allow(dead_code)]
    pub fn with_exclusions(mut self, exclusions: Vec<PathBuf>) -> Self {
        self.exclusions = exclusions;
        self
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
        let mut handles = Vec::with_capacity(self.num_threads);

        for _ in 0..self.num_threads {
            let queue = Arc::clone(&work_queue);
            let excl = Arc::clone(&exclusions);

            let handle = thread::spawn(move || worker_loop(queue, excl));
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
        let (dir_watcher, watch_issues) = DirectoryWatcher::new(&self.roots, &self.exclusions);
        issues.extend(watch_issues);
        {
            let mut guard = match self.watcher.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            *guard = Some(dir_watcher);
        }

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
        let guard = match self.watcher.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if let Some(ref w) = *guard {
            w.drain_updates(out, issues);
            if let Some(checkpoint) = checkpoints.first_mut() {
                checkpoint.next_usn = checkpoint.next_usn.saturating_add(out.len() as i64);
            }
        }
        Ok(())
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

fn worker_loop(queue: Arc<WorkQueue>, exclusions: Arc<Vec<PathBuf>>) -> WorkerResult {
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
                    if should_skip_path(&path, &exclusions) {
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

pub(crate) fn should_skip_path(path: &Path, exclusions: &[PathBuf]) -> bool {
    if is_path_excluded(path, exclusions) {
        return true;
    }

    if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
        matches!(file_name, ".git" | "target")
    } else {
        false
    }
}

#[cfg(windows)]
pub(crate) fn compute_attributes(_path: &Path, _file_type: &fs::FileType, metadata: Option<&fs::Metadata>) -> u32 {
    use std::os::windows::fs::MetadataExt;
    if let Some(meta) = metadata {
        meta.file_attributes()
    } else {
        0
    }
}

#[cfg(not(windows))]
pub(crate) fn compute_attributes(path: &Path, file_type: &fs::FileType, metadata: Option<&fs::Metadata>) -> u32 {
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

    use super::{
        DirectorySource, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN,
    };
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
            let path = std::env::temp_dir()
                .join(format!("oxifind-crawler-test-{}-{unique}", std::process::id()));
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
}
