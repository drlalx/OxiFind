//! Cross-platform filesystem watcher using the `notify` crate for live index updates.

use std::{
    collections::HashSet,
    fs,
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{Receiver, channel},
    },
};

use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::{
    crawler::{compute_attributes, compute_file_id, should_skip_path},
    search::{EntryMetadata, IndexIssue, IndexUpdate, SearchEntry},
};

pub struct DirectoryWatcher {
    _watcher: Option<RecommendedWatcher>,
    rx: Receiver<notify::Result<Event>>,
    exclusions: Arc<Vec<PathBuf>>,
}

impl DirectoryWatcher {
    /// Creates a directory watcher on the given root directories, respecting exclusions.
    pub fn new(roots: &[PathBuf], exclusions: &[PathBuf]) -> (Self, Vec<IndexIssue>) {
        let (tx, rx) = channel();
        let mut issues = Vec::new();

        let watcher_res = RecommendedWatcher::new(
            move |res| {
                let _ = tx.send(res);
            },
            Config::default(),
        );

        let mut watcher = match watcher_res {
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

        if let Some(ref mut w) = watcher {
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

        (
            Self {
                _watcher: watcher,
                rx,
                exclusions: effective_exclusions,
            },
            issues,
        )
    }

    /// Drains any received file events and converts them into IndexUpdate operations.
    pub fn drain_updates(
        &self,
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
                if should_skip_path(&path, &self.exclusions) {
                    continue;
                }
                touched_paths.insert(path);
            }
        }

        // Deduplicate and process each touched path once per batch
        for path in touched_paths {
            if path.exists() {
                // Upsert newly created or modified item
                if let Ok(metadata) = fs::symlink_metadata(&path) {
                    let file_type = metadata.file_type();
                    let attributes = compute_attributes(&path, &file_type, Some(&metadata));
                    let file_id = compute_file_id(Some(&metadata));
                    let display = path.display().to_string();
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_owned();

                    out_updates.push(IndexUpdate::Upsert(SearchEntry {
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
                    }));
                }
            } else {
                // Item removed or renamed away
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
        let (watcher, issues) = DirectoryWatcher::new(&[temp.path().to_path_buf()], &[]);
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
}
