#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(name: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("oxifind-integ-{name}-{unique}"));
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
fn crawls_and_indexes_nested_files_with_multithreading() {
    let test_dir = TestDir::new("parallel");
    let root = test_dir.path();

    for dir_idx in 0..5 {
        let dir = root.join(format!("folder_{dir_idx}"));
        fs::create_dir_all(&dir).unwrap();
        for file_idx in 0..10 {
            let file_path = dir.join(format!("file_{file_idx}.txt"));
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "content {dir_idx}-{file_idx}").unwrap();
        }
    }

    let source = oxifind::crawler::DirectorySource::new(root).with_threads(4);
    let mut entries = Vec::new();
    let mut issues = Vec::new();

    oxifind::search::IndexSource::collect_initial(&source, &mut entries, &mut issues).unwrap();

    assert!(issues.is_empty(), "Unexpected issues: {issues:?}");
    // 5 folders + 50 files = 55 entries
    assert_eq!(entries.len(), 55);

    let file_names: Vec<String> = entries.iter().map(|e| e.metadata.name.clone()).collect();
    assert!(file_names.contains(&"folder_0".to_owned()));
    assert!(file_names.contains(&"file_0.txt".to_owned()));
    assert!(file_names.contains(&"file_9.txt".to_owned()));
}

#[test]
fn skips_git_and_target_directories_by_default() {
    let test_dir = TestDir::new("exclusions");
    let root = test_dir.path();

    let git_dir = root.join(".git");
    fs::create_dir_all(&git_dir).unwrap();
    File::create(git_dir.join("HEAD")).unwrap();

    let target_dir = root.join("target");
    fs::create_dir_all(&target_dir).unwrap();
    File::create(target_dir.join("build.log")).unwrap();

    let src_dir = root.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    File::create(src_dir.join("lib.rs")).unwrap();

    let source = oxifind::crawler::DirectorySource::new(root);
    let mut entries = Vec::new();
    let mut issues = Vec::new();

    oxifind::search::IndexSource::collect_initial(&source, &mut entries, &mut issues).unwrap();

    let names: Vec<String> = entries.iter().map(|e| e.metadata.name.clone()).collect();
    assert!(names.contains(&"src".to_owned()));
    assert!(names.contains(&"lib.rs".to_owned()));

    assert!(!names.contains(&".git".to_owned()));
    assert!(!names.contains(&"HEAD".to_owned()));
    assert!(!names.contains(&"target".to_owned()));
    assert!(!names.contains(&"build.log".to_owned()));
}

#[test]
fn elevation_query_succeeds_without_panic() {
    let elevated = oxifind::platform::is_elevated();
    // On Unix, non-root users will be false; root or container may be true
    println!("Process elevation state: {elevated}");
}

#[test]
fn collects_and_applies_live_file_events_to_search_index() {
    let test_dir = TestDir::new("live_events");
    let root = test_dir.path();

    let initial_file = root.join("initial.txt");
    File::create(&initial_file).unwrap();

    let source = oxifind::crawler::DirectorySource::new(root);
    let mut index = oxifind::search::SearchIndex::from_source(&source).unwrap();
    assert_eq!(index.len(), 1);

    // Create a new file during runtime
    let dynamic_file = root.join("dynamic.txt");
    {
        let mut file = File::create(&dynamic_file).unwrap();
        writeln!(file, "dynamic content").unwrap();
    }

    std::thread::sleep(std::time::Duration::from_millis(150));

    let batch =
        oxifind::search::UpdateBatch::collect(&source, index.journal_checkpoints()).unwrap();
    index.apply(batch);

    let results = index.search_results("dynamic", 10);
    assert_eq!(results.len(), 1);
    assert!(results[0].path.contains("dynamic.txt"));

    // Now delete the initial file
    fs::remove_file(&initial_file).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(150));

    let delete_batch =
        oxifind::search::UpdateBatch::collect(&source, index.journal_checkpoints()).unwrap();
    index.apply(delete_batch);

    let remaining = index.search_results("initial", 10);
    assert!(
        remaining.is_empty(),
        "Deleted file still returned in search: {remaining:?}"
    );
}
