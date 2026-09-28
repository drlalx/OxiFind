use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fs, io,
    path::{Path, PathBuf},
};

const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;

#[derive(Clone, Debug, Default)]
pub struct SearchEntry {
    pub display: String,
    pub path: String,
    pub metadata: EntryMetadata,
}

impl SearchEntry {
    pub fn from_path(display: String, path: PathBuf) -> Self {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();

        Self {
            display,
            path: path.display().to_string(),
            metadata: EntryMetadata {
                name,
                full_path: path,
                ..EntryMetadata::default()
            },
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EntryMetadata {
    pub file_id: Option<u128>,
    pub parent_id: Option<u128>,
    pub name: String,
    pub full_path: PathBuf,
    pub volume_identity: Option<String>,
    pub attributes: u32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IndexIssue {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct JournalCheckpoint {
    pub volume_identity: String,
    pub journal_id: u64,
    pub next_usn: i64,
}

#[derive(Clone, Debug)]
pub enum IndexUpdate {
    Upsert(SearchEntry),
    Remove {
        volume_identity: String,
        file_id: u128,
    },
    /// Replaces every entry of a volume with a freshly collected snapshot.
    ReplaceVolume {
        volume_identity: String,
        entries: Vec<SearchEntry>,
    },
}

pub trait IndexSource {
    fn collect_initial(
        &self,
        out: &mut Vec<SearchEntry>,
        issues: &mut Vec<IndexIssue>,
    ) -> io::Result<()>;

    /// Appends the changes recorded after each checkpoint in order and advances the checkpoints past them.
    fn collect_updates(
        &self,
        _checkpoints: &mut [JournalCheckpoint],
        _out: &mut Vec<IndexUpdate>,
        _issues: &mut Vec<IndexIssue>,
    ) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Incremental updates are not supported by this index source",
        ))
    }

    fn journal_checkpoints(&self) -> io::Result<Vec<JournalCheckpoint>> {
        Ok(Vec::new())
    }
}

pub struct WorkspaceSource {
    root: PathBuf,
}

impl WorkspaceSource {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl IndexSource for WorkspaceSource {
    fn collect_initial(
        &self,
        out: &mut Vec<SearchEntry>,
        issues: &mut Vec<IndexIssue>,
    ) -> io::Result<()> {
        collect_directories(&self.root, out, issues)
    }
}

pub struct SearchIndex {
    entries: Vec<SearchEntry>,
    issues: Vec<IndexIssue>,
    resolution_issues: Vec<IndexIssue>,
    journal_checkpoints: Vec<JournalCheckpoint>,
}

/// Changes collected against a copy of an index's checkpoints, so they can be gathered on another
/// thread and applied later in one step.
#[derive(Debug, Default)]
pub struct UpdateBatch {
    pub updates: Vec<IndexUpdate>,
    pub issues: Vec<IndexIssue>,
    pub checkpoints: Vec<JournalCheckpoint>,
}

impl UpdateBatch {
    pub fn collect(
        source: &impl IndexSource,
        checkpoints: &[JournalCheckpoint],
    ) -> io::Result<Self> {
        let mut batch = Self {
            checkpoints: checkpoints.to_vec(),
            ..Self::default()
        };
        source.collect_updates(
            &mut batch.checkpoints,
            &mut batch.updates,
            &mut batch.issues,
        )?;
        Ok(batch)
    }

    pub fn is_empty(&self) -> bool {
        self.updates.is_empty() && self.issues.is_empty()
    }
}

impl SearchIndex {
    pub fn from_source(source: &impl IndexSource) -> io::Result<Self> {
        let mut entries = Vec::new();
        let mut issues = Vec::new();
        source.collect_initial(&mut entries, &mut issues)?;
        let resolution_issues = resolve_entry_paths(&mut entries);
        entries.sort_by(compare_entries);
        let journal_checkpoints = source.journal_checkpoints()?;
        Ok(Self {
            entries,
            issues,
            resolution_issues,
            journal_checkpoints,
        })
    }

    /// Applies the source's changes since the retained checkpoints and returns how many were applied.
    ///
    /// Entries and checkpoints stay unchanged when collecting the updates fails.
    #[cfg(test)]
    pub fn refresh(&mut self, source: &impl IndexSource) -> io::Result<usize> {
        let batch = UpdateBatch::collect(source, &self.journal_checkpoints)?;
        Ok(self.apply(batch))
    }

    /// Applies a collected batch in order, adopts its checkpoints, and returns how many updates it held.
    pub fn apply(&mut self, batch: UpdateBatch) -> usize {
        let applied = batch.updates.len();
        self.apply_updates(batch.updates);
        self.issues.extend(batch.issues);
        self.journal_checkpoints = batch.checkpoints;
        applied
    }

    fn apply_updates(&mut self, updates: Vec<IndexUpdate>) {
        if updates.is_empty() {
            return;
        }

        let mut slots: Vec<Option<SearchEntry>> = std::mem::take(&mut self.entries)
            .into_iter()
            .map(Some)
            .collect();
        let mut positions: HashMap<(Option<String>, u128), usize> = slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                let metadata = &slot.as_ref()?.metadata;
                Some(((metadata.volume_identity.clone(), metadata.file_id?), index))
            })
            .collect();

        for update in updates {
            match update {
                IndexUpdate::Upsert(entry) => upsert_slot(&mut slots, &mut positions, entry),
                IndexUpdate::Remove {
                    volume_identity,
                    file_id,
                } => {
                    if let Some(index) = positions.remove(&(Some(volume_identity), file_id)) {
                        slots[index] = None;
                    }
                }
                IndexUpdate::ReplaceVolume {
                    volume_identity,
                    entries,
                } => {
                    let on_volume = |identity: &Option<String>| {
                        identity.as_deref() == Some(volume_identity.as_str())
                    };
                    for slot in &mut slots {
                        if slot
                            .as_ref()
                            .is_some_and(|entry| on_volume(&entry.metadata.volume_identity))
                        {
                            *slot = None;
                        }
                    }
                    positions.retain(|(identity, _), _| !on_volume(identity));
                    for entry in entries {
                        upsert_slot(&mut slots, &mut positions, entry);
                    }
                }
            }
        }

        self.entries = slots.into_iter().flatten().collect();
        self.resolution_issues = resolve_entry_paths(&mut self.entries);
        self.entries.sort_by(compare_entries);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn indexing_issue_count(&self) -> usize {
        self.issues.len() + self.resolution_issues.len()
    }

    pub fn journal_checkpoints(&self) -> &[JournalCheckpoint] {
        &self.journal_checkpoints
    }

    /// Returns the displays of [`search_results`](Self::search_results).
    #[cfg(test)]
    pub fn search(&self, query: &str, limit: usize) -> Vec<String> {
        self.matching_entries(query, limit)
            .into_iter()
            .map(|entry| entry.display.clone())
            .collect()
    }

    /// Returns up to `limit` entries matching every term of `query`, with their full paths and attributes.
    ///
    /// Terms are separated by whitespace outside double quotes. `^text` matches file names
    /// starting with `text`, `=text` matches whole file names, `path:text` matches only the
    /// full path, and any other term matches the file name or the path. Matching ignores case.
    /// `sort:relevance|name|path|type`, optionally with a leading `-` to reverse it, orders the
    /// results; ties fall back to relevance and then to the display order.
    pub fn search_results(&self, query: &str, limit: usize) -> Vec<SearchResult> {
        self.matching_entries(query, limit)
            .into_iter()
            .map(|entry| SearchResult {
                display: entry.display.clone(),
                path: entry.path.clone(),
                attributes: entry.metadata.attributes,
            })
            .collect()
    }

    fn matching_entries(&self, query: &str, limit: usize) -> Vec<&SearchEntry> {
        if limit == 0 {
            return Vec::new();
        }

        let Query { terms, sort } = parse_query(query);
        if terms.is_empty() && sort == SortOrder::default() {
            return self.entries.iter().take(limit).collect();
        }

        let mut matches: Vec<(u32, &SearchEntry)> = self
            .entries
            .iter()
            .filter_map(|entry| score_entry(entry, &terms).map(|score| (score, entry)))
            .collect();

        matches.sort_by(|(left_score, left_entry), (right_score, right_entry)| {
            let primary = match sort.key {
                SortKey::Relevance => left_score.cmp(right_score),
                SortKey::Name => {
                    compare_text(&left_entry.metadata.name, &right_entry.metadata.name)
                }
                SortKey::Path => compare_text(&left_entry.path, &right_entry.path),
                SortKey::Type => is_directory(right_entry).cmp(&is_directory(left_entry)),
            };
            let primary = if sort.descending {
                primary.reverse()
            } else {
                primary
            };
            primary
                .then_with(|| left_score.cmp(right_score))
                .then_with(|| compare_entries(left_entry, right_entry))
        });
        matches.truncate(limit);

        matches.into_iter().map(|(_, entry)| entry).collect()
    }
}

/// A matched entry as presented to users: its display text, full path, and file attributes.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchResult {
    pub display: String,
    pub path: String,
    pub attributes: u32,
}

fn upsert_slot(
    slots: &mut Vec<Option<SearchEntry>>,
    positions: &mut HashMap<(Option<String>, u128), usize>,
    entry: SearchEntry,
) {
    let Some(file_id) = entry.metadata.file_id else {
        slots.push(Some(entry));
        return;
    };
    let key = (entry.metadata.volume_identity.clone(), file_id);
    match positions.get(&key) {
        Some(&index) => slots[index] = Some(entry),
        None => {
            positions.insert(key, slots.len());
            slots.push(Some(entry));
        }
    }
}

fn compare_entries(left: &SearchEntry, right: &SearchEntry) -> Ordering {
    left.display
        .to_lowercase()
        .cmp(&right.display.to_lowercase())
        .then_with(|| left.display.cmp(&right.display))
        .then_with(|| left.path.cmp(&right.path))
}

fn compare_text(left: &str, right: &str) -> Ordering {
    left.to_lowercase()
        .cmp(&right.to_lowercase())
        .then_with(|| left.cmp(right))
}

fn is_directory(entry: &SearchEntry) -> bool {
    entry.metadata.attributes & FILE_ATTRIBUTE_DIRECTORY != 0
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum SortKey {
    #[default]
    Relevance,
    Name,
    Path,
    Type,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct SortOrder {
    key: SortKey,
    descending: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct Query {
    terms: Vec<QueryTerm>,
    sort: SortOrder,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum TermKind {
    #[default]
    Contains,
    Prefix,
    Exact,
    Path,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct QueryTerm {
    kind: TermKind,
    needle: String,
}

/// Parses a query into lowercase terms and a sort order, dropping terms that have no text left
/// after their modifier. The last valid `sort:` term wins; unknown sort keys stay search text.
fn parse_query(query: &str) -> Query {
    let mut parsed = Query::default();

    for raw in split_terms(query) {
        // Modifiers only count before any quote, so quoted text stays literal.
        let head = raw.get(..5);
        if let Some(head) = head.filter(|head| head.eq_ignore_ascii_case("sort:")) {
            let value = raw[head.len()..].replace('"', "").to_lowercase();
            let (descending, key) = match value.strip_prefix('-') {
                Some(key) => (true, key),
                None => (false, value.as_str()),
            };
            let key = match key {
                "relevance" => Some(SortKey::Relevance),
                "name" => Some(SortKey::Name),
                "path" => Some(SortKey::Path),
                "type" => Some(SortKey::Type),
                _ => None,
            };
            match key {
                Some(key) => {
                    parsed.sort = SortOrder { key, descending };
                    continue;
                }
                None if value.is_empty() => continue,
                None => {}
            }
        }

        let (kind, rest) = match head {
            Some(head) if head.eq_ignore_ascii_case("path:") => (TermKind::Path, &raw[5..]),
            _ => match (raw.strip_prefix('^'), raw.strip_prefix('=')) {
                (Some(rest), _) => (TermKind::Prefix, rest),
                (_, Some(rest)) => (TermKind::Exact, rest),
                _ => (TermKind::Contains, raw.as_str()),
            },
        };
        let needle = rest.replace('"', "").to_lowercase();
        if !needle.is_empty() {
            parsed.terms.push(QueryTerm { kind, needle });
        }
    }

    parsed
}

/// Splits on whitespace outside double quotes; the quotes stay in the returned terms.
fn split_terms(query: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut current = String::new();
    let mut quoted = false;

    for character in query.chars() {
        if character == '"' {
            quoted = !quoted;
        } else if character.is_whitespace() && !quoted {
            if !current.is_empty() {
                terms.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(character);
    }
    if !current.is_empty() {
        terms.push(current);
    }

    terms
}

/// Sums the per-term ranks (lower is better), or returns `None` when any term does not match.
fn score_entry(entry: &SearchEntry, terms: &[QueryTerm]) -> Option<u32> {
    let name = entry.metadata.name.to_lowercase();
    let path = entry.path.to_lowercase();
    terms.iter().try_fold(0_u32, |total, term| {
        Some(total + score_term(term, &name, &path)?)
    })
}

fn score_term(term: &QueryTerm, name: &str, path: &str) -> Option<u32> {
    let needle = term.needle.as_str();
    match term.kind {
        TermKind::Contains | TermKind::Prefix if name.starts_with(needle) => Some(0),
        TermKind::Exact if name == needle => Some(0),
        TermKind::Contains if name.contains(needle) => Some(1),
        TermKind::Contains | TermKind::Path if path.contains(needle) => Some(2),
        _ => None,
    }
}

pub(crate) fn resolve_entry_paths(entries: &mut [SearchEntry]) -> Vec<IndexIssue> {
    // File identifiers are only unique within their volume.
    let volume_of: Vec<usize> = {
        let mut volumes: HashMap<Option<&str>, usize> = HashMap::new();
        entries
            .iter()
            .map(|entry| {
                let next = volumes.len();
                *volumes
                    .entry(entry.metadata.volume_identity.as_deref())
                    .or_insert(next)
            })
            .collect()
    };
    let positions = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| Some(((volume_of[index], entry.metadata.file_id?), index)))
        .collect::<HashMap<_, _>>();
    let mut issues = Vec::new();

    for index in 0..entries.len() {
        let Some(file_id) = entries[index].metadata.file_id else {
            continue;
        };
        let original_path = entries[index].metadata.full_path.clone();
        let mut lineage = Vec::new();
        let mut visited = HashSet::new();
        let mut current = index;

        let base_path = loop {
            if !visited.insert(current) {
                issues.push(IndexIssue {
                    path: original_path,
                    message: format!("Cycle detected while resolving file ID {file_id}"),
                });
                break None;
            }

            match entries[current].metadata.parent_id {
                None => break Some(entries[current].metadata.full_path.clone()),
                Some(parent_id) => match positions.get(&(volume_of[current], parent_id)) {
                    Some(parent_index) => {
                        lineage.push(current);
                        current = *parent_index;
                    }
                    None => {
                        issues.push(IndexIssue {
                            path: original_path,
                            message: format!("Missing parent record {parent_id}"),
                        });
                        break None;
                    }
                },
            }
        };

        if let Some(mut path) = base_path {
            for child_index in lineage.into_iter().rev() {
                path.push(&entries[child_index].metadata.name);
            }
            entries[index].metadata.full_path = path;
            let display = entries[index].metadata.full_path.display().to_string();
            entries[index].path = display.clone();
            entries[index].display = display;
        }
    }

    issues
}

fn collect_directories(
    root: &Path,
    out: &mut Vec<SearchEntry>,
    issues: &mut Vec<IndexIssue>,
) -> io::Result<()> {
    let mut directories = vec![root.to_path_buf()];

    while let Some(current) = directories.pop() {
        let entries = match fs::read_dir(&current) {
            Ok(entries) => entries,
            Err(error) if current == root => return Err(error),
            Err(error) => {
                record_issue(issues, current, error);
                continue;
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    record_issue(issues, current.clone(), error);
                    continue;
                }
            };

            if should_skip_entry(&entry) {
                continue;
            }

            let path = entry.path();
            let display = path
                .strip_prefix(root)
                .unwrap_or(path.as_path())
                .display()
                .to_string();

            out.push(SearchEntry::from_path(display, path.clone()));

            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    record_issue(issues, path, error);
                    continue;
                }
            };
            if file_type.is_dir() {
                if let Some(directory) = out.last_mut() {
                    directory.metadata.attributes |= FILE_ATTRIBUTE_DIRECTORY;
                }
                directories.push(path);
            }
        }
    }

    Ok(())
}

fn record_issue(issues: &mut Vec<IndexIssue>, path: PathBuf, error: io::Error) {
    issues.push(IndexIssue {
        path,
        message: error.to_string(),
    });
}

fn should_skip_entry(entry: &fs::DirEntry) -> bool {
    match entry.file_name().to_str() {
        Some("target") | Some(".git") => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs, io,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        EntryMetadata, IndexIssue, IndexSource, IndexUpdate, JournalCheckpoint, SearchEntry,
        SearchIndex, SearchResult, UpdateBatch, WorkspaceSource, resolve_entry_paths,
    };

    struct TestSource {
        entries: Vec<SearchEntry>,
        issues: Vec<IndexIssue>,
    }

    impl IndexSource for TestSource {
        fn collect_initial(
            &self,
            out: &mut Vec<SearchEntry>,
            issues: &mut Vec<IndexIssue>,
        ) -> io::Result<()> {
            out.extend(self.entries.clone());
            issues.extend(self.issues.clone());
            Ok(())
        }
    }

    struct CheckpointSource;

    impl IndexSource for CheckpointSource {
        fn collect_initial(
            &self,
            out: &mut Vec<SearchEntry>,
            _issues: &mut Vec<IndexIssue>,
        ) -> io::Result<()> {
            out.push(entry("alpha.txt", r"C:\alpha.txt"));
            Ok(())
        }

        fn journal_checkpoints(&self) -> io::Result<Vec<JournalCheckpoint>> {
            Ok(vec![JournalCheckpoint {
                volume_identity: r"C:\".to_owned(),
                journal_id: 42,
                next_usn: 84,
            }])
        }
    }

    #[derive(Default)]
    struct JournalSource {
        entries: Vec<SearchEntry>,
        updates: Vec<IndexUpdate>,
        next_usn: i64,
        fails: bool,
    }

    impl IndexSource for JournalSource {
        fn collect_initial(
            &self,
            out: &mut Vec<SearchEntry>,
            _issues: &mut Vec<IndexIssue>,
        ) -> io::Result<()> {
            out.extend(self.entries.clone());
            Ok(())
        }

        fn collect_updates(
            &self,
            checkpoints: &mut [JournalCheckpoint],
            out: &mut Vec<IndexUpdate>,
            _issues: &mut Vec<IndexIssue>,
        ) -> io::Result<()> {
            out.extend(self.updates.clone());
            for checkpoint in checkpoints.iter_mut() {
                checkpoint.next_usn = self.next_usn;
            }
            if self.fails {
                return Err(io::Error::other("journal read failed"));
            }
            Ok(())
        }

        fn journal_checkpoints(&self) -> io::Result<Vec<JournalCheckpoint>> {
            Ok(vec![JournalCheckpoint {
                volume_identity: r"C:\".to_owned(),
                journal_id: 42,
                next_usn: 84,
            }])
        }
    }

    fn entry(display: &str, path: &str) -> SearchEntry {
        SearchEntry::from_path(display.to_owned(), PathBuf::from(path))
    }

    fn directory(display: &str, path: &str) -> SearchEntry {
        let mut entry = entry(display, path);
        entry.metadata.attributes = 0x0000_0010;
        entry
    }

    fn volume_entry(
        volume: &str,
        file_id: u128,
        parent_id: Option<u128>,
        name: &str,
    ) -> SearchEntry {
        let full_path = if parent_id.is_none() { volume } else { name };
        let mut entry = identified_entry(file_id, parent_id, name, full_path);
        entry.metadata.volume_identity = Some(volume.to_owned());
        entry
    }

    fn journal_index(entries: Vec<SearchEntry>) -> SearchIndex {
        SearchIndex::from_source(&JournalSource {
            entries,
            ..JournalSource::default()
        })
        .unwrap()
    }

    fn journal_tree() -> Vec<SearchEntry> {
        vec![
            volume_entry(r"C:\", 5, None, r"C:\"),
            volume_entry(r"C:\", 10, Some(5), "docs"),
            volume_entry(r"C:\", 11, Some(10), "draft.txt"),
        ]
    }

    fn identified_entry(
        file_id: u128,
        parent_id: Option<u128>,
        name: &str,
        full_path: &str,
    ) -> SearchEntry {
        SearchEntry {
            display: name.to_owned(),
            path: full_path.to_owned(),
            metadata: EntryMetadata {
                file_id: Some(file_id),
                parent_id,
                name: name.to_owned(),
                full_path: PathBuf::from(full_path),
                ..EntryMetadata::default()
            },
        }
    }

    fn index(entries: Vec<SearchEntry>) -> SearchIndex {
        SearchIndex::from_source(&TestSource {
            entries,
            issues: Vec::new(),
        })
        .unwrap()
    }

    fn small_index_fixture() -> Vec<SearchEntry> {
        synthetic_index_fixture(100)
    }

    fn medium_index_fixture() -> Vec<SearchEntry> {
        synthetic_index_fixture(10_000)
    }

    fn large_index_fixture() -> Vec<SearchEntry> {
        synthetic_index_fixture(100_000)
    }

    fn synthetic_index_fixture(entry_count: usize) -> Vec<SearchEntry> {
        (0..entry_count)
            .map(|index| {
                let file_name = format!("file-{index:06}.txt");
                entry(&file_name, &format!(r"C:\fixture\{file_name}"))
            })
            .collect()
    }

    #[derive(Default)]
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
                "everything-lite-test-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &std::path::Path {
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

    #[test]
    fn indexes_workspace_entries_with_relative_paths_and_skips_build_metadata() {
        let temporary_directory = TemporaryDirectory::new();
        let root = temporary_directory.path();
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::create_dir_all(root.join("target")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join("alpha.txt"), "").unwrap();
        fs::write(root.join("nested").join("bravo.txt"), "").unwrap();
        fs::write(root.join("target").join("skipped.txt"), "").unwrap();
        fs::write(root.join(".git").join("skipped.txt"), "").unwrap();

        let index = SearchIndex::from_source(&WorkspaceSource::new(&root)).unwrap();

        assert_eq!(
            index.search("", 10),
            vec!["alpha.txt", "nested", r"nested\bravo.txt"],
        );
        assert_eq!(index.entries[2].metadata.name, "bravo.txt");
        assert_eq!(
            index.entries[2].metadata.full_path,
            root.join("nested").join("bravo.txt"),
        );
        assert_eq!(index.entries[2].metadata.file_id, None);
        assert_eq!(index.entries[2].metadata.parent_id, None);
        assert_eq!(index.entries[2].metadata.volume_identity, None);
        assert_eq!(index.entries[2].metadata.attributes, 0);
        assert_eq!(index.entries[1].metadata.attributes, 0x0000_0010);
    }

    #[cfg(windows)]
    #[derive(Default)]
    struct DeniedListing {
        path: PathBuf,
    }

    #[cfg(windows)]
    impl DeniedListing {
        // S-1-1-0 is "Everyone"; an explicit deny ACE also blocks owners and administrators.
        fn apply(path: PathBuf) -> Self {
            let status = std::process::Command::new("icacls")
                .arg(&path)
                .args(["/deny", "*S-1-1-0:(RD)"])
                .output()
                .unwrap()
                .status;
            assert!(status.success(), "icacls could not deny directory listing");
            Self { path }
        }
    }

    #[cfg(windows)]
    impl Drop for DeniedListing {
        fn drop(&mut self) {
            let _ = std::process::Command::new("icacls")
                .arg(&self.path)
                .args(["/remove:d", "*S-1-1-0"])
                .output();
        }
    }

    #[cfg(windows)]
    #[test]
    fn retains_workspace_entries_and_reports_inaccessible_directories() {
        let temporary_directory = TemporaryDirectory::new();
        let root = temporary_directory.path();
        let locked = root.join("locked");
        fs::create_dir_all(root.join("open")).unwrap();
        fs::create_dir_all(&locked).unwrap();
        fs::write(root.join("open").join("alpha.txt"), "").unwrap();
        fs::write(locked.join("hidden.txt"), "").unwrap();
        let _denied_listing = DeniedListing::apply(locked.clone());

        let index = SearchIndex::from_source(&WorkspaceSource::new(root)).unwrap();

        assert_eq!(
            index.search("", 10),
            vec!["locked", "open", r"open\alpha.txt"],
        );
        assert_eq!(index.indexing_issue_count(), 1);
        assert_eq!(index.issues[0].path, locked);
    }

    #[test]
    fn reports_incremental_updates_as_unsupported_for_workspace_indexes() {
        let error = WorkspaceSource::new(".")
            .collect_updates(&mut [], &mut Vec::new(), &mut Vec::new())
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn applies_created_moved_and_deleted_entries_in_order() {
        let mut index = journal_index(journal_tree());
        let source = JournalSource {
            updates: vec![
                IndexUpdate::Upsert(volume_entry(r"C:\", 12, Some(10), "temp.txt")),
                IndexUpdate::Upsert(volume_entry(r"C:\", 11, Some(5), "final.txt")),
                IndexUpdate::Remove {
                    volume_identity: r"C:\".to_owned(),
                    file_id: 12,
                },
                IndexUpdate::Upsert(volume_entry(r"C:\", 13, Some(10), "notes.txt")),
            ],
            next_usn: 128,
            ..JournalSource::default()
        };

        let applied = index.refresh(&source).unwrap();

        assert_eq!(applied, 4);
        assert_eq!(
            index.search("", 10),
            vec![r"C:\", r"C:\docs", r"C:\docs\notes.txt", r"C:\final.txt"],
        );
        assert_eq!(index.journal_checkpoints()[0].next_usn, 128);
    }

    #[test]
    fn collected_batches_leave_the_index_unchanged_until_applied() {
        let mut index = journal_index(journal_tree());
        let source = JournalSource {
            updates: vec![IndexUpdate::Remove {
                volume_identity: r"C:\".to_owned(),
                file_id: 11,
            }],
            next_usn: 128,
            ..JournalSource::default()
        };

        let batch = UpdateBatch::collect(&source, index.journal_checkpoints()).unwrap();

        assert_eq!(index.search("draft", 10), vec![r"C:\docs\draft.txt"]);
        assert_eq!(index.journal_checkpoints()[0].next_usn, 84);
        assert_eq!(index.apply(batch), 1);
        assert!(index.search("draft", 10).is_empty());
        assert_eq!(index.journal_checkpoints()[0].next_usn, 128);
    }

    #[test]
    fn replaces_every_entry_of_a_rebuilt_volume() {
        let mut index = journal_index(vec![
            volume_entry(r"C:\", 5, None, r"C:\"),
            volume_entry(r"D:\", 5, None, r"D:\"),
            volume_entry(r"C:\", 7, Some(5), "alpha.txt"),
            volume_entry(r"D:\", 7, Some(5), "alpha.txt"),
            volume_entry(r"C:\", 8, Some(5), "stale.txt"),
        ]);

        index
            .refresh(&JournalSource {
                updates: vec![IndexUpdate::ReplaceVolume {
                    volume_identity: r"C:\".to_owned(),
                    entries: vec![
                        volume_entry(r"C:\", 5, None, r"C:\"),
                        volume_entry(r"C:\", 9, Some(5), "fresh.txt"),
                    ],
                }],
                ..JournalSource::default()
            })
            .unwrap();

        assert_eq!(
            index.search("", 10),
            vec![r"C:\", r"C:\fresh.txt", r"D:\", r"D:\alpha.txt"],
        );
    }

    #[test]
    fn renaming_a_directory_moves_its_descendants() {
        let mut index = journal_index(journal_tree());

        index
            .refresh(&JournalSource {
                updates: vec![IndexUpdate::Upsert(volume_entry(
                    r"C:\",
                    10,
                    Some(5),
                    "archive",
                ))],
                next_usn: 90,
                ..JournalSource::default()
            })
            .unwrap();

        assert_eq!(index.search("draft", 10), vec![r"C:\archive\draft.txt"]);
    }

    #[test]
    fn removes_entries_only_from_the_volume_named_by_the_update() {
        let mut index = journal_index(vec![
            volume_entry(r"C:\", 5, None, r"C:\"),
            volume_entry(r"D:\", 5, None, r"D:\"),
            volume_entry(r"C:\", 7, Some(5), "alpha.txt"),
            volume_entry(r"D:\", 7, Some(5), "alpha.txt"),
        ]);

        index
            .refresh(&JournalSource {
                updates: vec![IndexUpdate::Remove {
                    volume_identity: r"D:\".to_owned(),
                    file_id: 7,
                }],
                ..JournalSource::default()
            })
            .unwrap();

        assert_eq!(index.search("alpha", 10), vec![r"C:\alpha.txt"]);
    }

    #[test]
    fn keeps_the_index_and_checkpoints_when_collecting_updates_fails() {
        let mut index = journal_index(journal_tree());

        let error = index
            .refresh(&JournalSource {
                updates: vec![IndexUpdate::Remove {
                    volume_identity: r"C:\".to_owned(),
                    file_id: 11,
                }],
                next_usn: 128,
                fails: true,
                ..JournalSource::default()
            })
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(index.search("draft", 10), vec![r"C:\docs\draft.txt"]);
        assert_eq!(index.journal_checkpoints()[0].next_usn, 84);
    }

    #[test]
    fn resolves_orphaned_entries_once_their_parent_is_created() {
        let mut index = journal_index(vec![
            volume_entry(r"C:\", 5, None, r"C:\"),
            volume_entry(r"C:\", 11, Some(10), "draft.txt"),
        ]);
        assert_eq!(index.indexing_issue_count(), 1);

        index
            .refresh(&JournalSource {
                updates: vec![IndexUpdate::Upsert(volume_entry(
                    r"C:\",
                    10,
                    Some(5),
                    "docs",
                ))],
                ..JournalSource::default()
            })
            .unwrap();

        assert_eq!(index.indexing_issue_count(), 0);
        assert_eq!(index.search("draft", 10), vec![r"C:\docs\draft.txt"]);
    }

    #[test]
    fn records_journal_checkpoints_from_the_completed_index_source() {
        let index = SearchIndex::from_source(&CheckpointSource)
            .expect("the source checkpoint must be retained with its index");

        assert_eq!(
            index.journal_checkpoints(),
            &[JournalCheckpoint {
                volume_identity: r"C:\".to_owned(),
                journal_id: 42,
                next_usn: 84,
            }]
        );
    }

    #[test]
    fn builds_child_paths_from_parent_records() {
        let mut entries = vec![
            identified_entry(1, None, "root", r"C:\root"),
            identified_entry(2, Some(1), "child", ""),
            identified_entry(3, Some(2), "report.txt", ""),
        ];

        let issues = resolve_entry_paths(&mut entries);

        assert!(issues.is_empty());
        assert_eq!(
            entries[1].metadata.full_path,
            PathBuf::from(r"C:\root\child")
        );
        assert_eq!(
            entries[2].metadata.full_path,
            PathBuf::from(r"C:\root\child\report.txt"),
        );
        assert_eq!(entries[2].path, r"C:\root\child\report.txt");
    }

    #[test]
    fn resolves_parent_records_within_their_own_volume() {
        let mut entries = vec![
            volume_entry(r"C:\", 5, None, r"C:\"),
            volume_entry(r"D:\", 5, None, r"D:\"),
            volume_entry(r"C:\", 7, Some(5), "alpha.txt"),
            volume_entry(r"D:\", 7, Some(5), "bravo.txt"),
        ];

        let issues = resolve_entry_paths(&mut entries);

        assert!(issues.is_empty());
        assert_eq!(entries[2].path, r"C:\alpha.txt");
        assert_eq!(entries[3].path, r"D:\bravo.txt");
    }

    #[test]
    fn retains_orphaned_records_and_reports_the_missing_parent() {
        let mut entries = vec![identified_entry(2, Some(1), "orphan.txt", r"C:\orphan.txt")];

        let issues = resolve_entry_paths(&mut entries);

        assert_eq!(
            entries[0].metadata.full_path,
            PathBuf::from(r"C:\orphan.txt")
        );
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].message, "Missing parent record 1");
    }

    #[test]
    fn retains_cyclic_records_and_reports_the_cycle() {
        let mut entries = vec![
            identified_entry(1, Some(2), "first", r"C:\first"),
            identified_entry(2, Some(1), "second", r"C:\second"),
        ];

        let issues = resolve_entry_paths(&mut entries);

        assert_eq!(entries[0].metadata.full_path, PathBuf::from(r"C:\first"));
        assert_eq!(issues.len(), 2);
        assert_eq!(
            issues[0].message,
            "Cycle detected while resolving file ID 1"
        );
    }

    #[test]
    fn provides_deterministic_small_medium_and_large_index_fixtures() {
        let small = small_index_fixture();
        let medium = medium_index_fixture();
        let large = large_index_fixture();

        assert_eq!(small.len(), 100);
        assert_eq!(medium.len(), 10_000);
        assert_eq!(large.len(), 100_000);
        assert_eq!(small[0].display, "file-000000.txt");
        assert_eq!(large[99_999].path, r"C:\fixture\file-099999.txt");
    }

    #[test]
    fn sorts_entries_case_insensitively() {
        let index = index(vec![
            entry("Zoo", r"C:\Zoo"),
            entry("apple", r"C:\apple"),
            entry("Bravo", r"C:\Bravo"),
        ]);

        assert_eq!(index.search("", 10), vec!["apple", "Bravo", "Zoo"],);
    }

    #[test]
    fn ranks_name_matches_before_path_matches() {
        let index = index(vec![
            entry("report.txt", r"C:\alpha\report.txt"),
            entry("calpine.txt", r"C:\calpine.txt"),
            entry("Alpine.txt", r"C:\Alpine.txt"),
        ]);

        assert_eq!(
            index.search("alp", 10),
            vec!["Alpine.txt", "calpine.txt", "report.txt"],
        );
    }

    #[test]
    fn requires_every_whitespace_separated_term_to_match() {
        let index = index(vec![
            entry("annual report.txt", r"C:\docs\annual report.txt"),
            entry("annual.txt", r"C:\annual.txt"),
            entry("report.txt", r"C:\report.txt"),
        ]);

        assert_eq!(index.search("report annual", 10), vec!["annual report.txt"]);
    }

    #[test]
    fn quotes_keep_spaces_inside_a_single_term() {
        let index = index(vec![
            entry("annual report.txt", r"C:\docs\annual report.txt"),
            entry("report annual.txt", r"C:\docs\report annual.txt"),
        ]);

        assert_eq!(
            index.search("\"annual report\"", 10),
            vec!["annual report.txt"]
        );
    }

    #[test]
    fn caret_terms_match_only_file_name_prefixes() {
        let index = index(vec![
            entry("Alpine.txt", r"C:\Alpine.txt"),
            entry("calpine.txt", r"C:\calpine.txt"),
            entry("report.txt", r"C:\alpine\report.txt"),
        ]);

        assert_eq!(index.search("^alp", 10), vec!["Alpine.txt"]);
    }

    #[test]
    fn equals_terms_match_whole_file_names_only() {
        let index = index(vec![
            entry("notes.txt", r"C:\notes.txt"),
            entry("notes.txt.bak", r"C:\notes.txt.bak"),
            entry("old notes.txt", r"C:\old notes.txt"),
        ]);

        assert_eq!(index.search("=notes.txt", 10), vec!["notes.txt"]);
        assert_eq!(index.search("=NOTES.TXT", 10), vec!["notes.txt"]);
    }

    #[test]
    fn path_terms_match_only_the_full_path() {
        let index = index(vec![
            entry("report.txt", r"C:\projects\report.txt"),
            entry("projects.txt", r"C:\other\projects.txt"),
            entry("notes.txt", r"C:\other\notes.txt"),
        ]);

        assert_eq!(
            index.search("path:other", 10),
            vec!["notes.txt", "projects.txt"]
        );
        assert!(index.search("path:other report", 10).is_empty());
        assert_eq!(
            index.search(r"projects path:projects\", 10),
            vec!["report.txt"]
        );
        assert_eq!(
            index.search(r#"PATH:"C:\projects""#, 10),
            vec!["report.txt"]
        );
    }

    #[test]
    fn modifiers_inside_quotes_are_literal() {
        let index = index(vec![
            entry("^caret.txt", r"C:\^caret.txt"),
            entry("caret.txt", r"C:\caret.txt"),
        ]);

        assert_eq!(index.search("\"^caret\"", 10), vec!["^caret.txt"]);
        assert_eq!(index.search("^caret", 10), vec!["caret.txt"]);
    }

    #[test]
    fn ignores_terms_without_text() {
        let index = index(vec![
            entry("alpha.txt", r"C:\alpha.txt"),
            entry("beta.txt", r"C:\beta.txt"),
        ]);

        assert_eq!(
            index.search("^ = path: \"\"", 10),
            vec!["alpha.txt", "beta.txt"]
        );
    }

    #[test]
    fn ranks_entries_by_their_combined_term_matches() {
        let index = index(vec![
            entry("x-alpha-beta.txt", r"C:\x-alpha-beta.txt"),
            entry("beta.txt", r"C:\alpha\beta.txt"),
            entry("alpha-beta.txt", r"C:\alpha-beta.txt"),
        ]);

        assert_eq!(
            index.search("alpha beta", 10),
            vec!["alpha-beta.txt", "beta.txt", "x-alpha-beta.txt"],
        );
    }

    fn sort_fixture() -> SearchIndex {
        index(vec![
            entry(r"zeta\alpha.txt", r"C:\zeta\alpha.txt"),
            entry("beta.txt", r"C:\beta.txt"),
            entry(r"alpha\gamma.txt", r"C:\alpha\gamma.txt"),
        ])
    }

    #[test]
    fn sorts_results_by_file_name() {
        assert_eq!(
            sort_fixture().search("sort:name", 10),
            vec![r"zeta\alpha.txt", "beta.txt", r"alpha\gamma.txt"],
        );
    }

    #[test]
    fn reverses_a_sort_key_prefixed_with_a_minus_sign() {
        assert_eq!(
            sort_fixture().search("SORT:-name", 10),
            vec![r"alpha\gamma.txt", "beta.txt", r"zeta\alpha.txt"],
        );
    }

    #[test]
    fn sorts_results_by_full_path() {
        assert_eq!(
            sort_fixture().search("sort:path", 10),
            vec![r"alpha\gamma.txt", "beta.txt", r"zeta\alpha.txt"],
        );
        assert_eq!(
            sort_fixture().search("sort:name sort:path", 10),
            sort_fixture().search("sort:path", 10),
        );
    }

    #[test]
    fn sorts_folders_before_files_by_type() {
        let index = index(vec![
            entry("alpha.txt", r"C:\alpha.txt"),
            directory("zeta", r"C:\zeta"),
            directory("beta", r"C:\beta"),
        ]);

        assert_eq!(
            index.search("sort:type", 10),
            vec!["beta", "zeta", "alpha.txt"]
        );
        assert_eq!(
            index.search("sort:-type", 10),
            vec!["alpha.txt", "beta", "zeta"]
        );
    }

    #[test]
    fn breaks_sort_key_ties_by_relevance() {
        let index = index(vec![
            directory("calpine", r"C:\calpine"),
            entry("alpha.txt", r"C:\alpha.txt"),
            directory("alpine", r"C:\alpine"),
        ]);

        assert_eq!(
            index.search("alp sort:type", 10),
            vec!["alpine", "calpine", "alpha.txt"],
        );
    }

    #[test]
    fn keeps_relevance_order_for_sort_relevance() {
        assert_eq!(
            sort_fixture().search("a sort:relevance", 10),
            sort_fixture().search("a", 10),
        );
    }

    #[test]
    fn treats_unknown_sort_keys_as_search_text() {
        assert!(sort_fixture().search("sort:size", 10).is_empty());
        assert_eq!(sort_fixture().search("sort:", 10).len(), 3);
    }

    #[test]
    fn returns_full_paths_and_attributes_with_each_result() {
        let index = index(vec![
            directory("nested", r"C:\root\nested"),
            entry(r"nested\a.txt", r"C:\root\nested\a.txt"),
        ]);

        assert_eq!(
            index.search_results("", 10),
            vec![
                SearchResult {
                    display: "nested".to_owned(),
                    path: r"C:\root\nested".to_owned(),
                    attributes: 0x0000_0010,
                },
                SearchResult {
                    display: r"nested\a.txt".to_owned(),
                    path: r"C:\root\nested\a.txt".to_owned(),
                    attributes: 0,
                },
            ],
        );
        assert_eq!(
            index
                .search_results("a.txt", 10)
                .into_iter()
                .map(|result| result.display)
                .collect::<Vec<_>>(),
            index.search("a.txt", 10),
        );
    }

    #[test]
    fn trims_queries_and_honors_result_limits() {
        let index = index(vec![
            entry("alpha.txt", r"C:\alpha.txt"),
            entry("alphabet.txt", r"C:\alphabet.txt"),
            entry("beta.txt", r"C:\beta.txt"),
        ]);

        assert_eq!(index.search(" alp ", 1), vec!["alpha.txt"]);
        assert_eq!(index.search("", 2), vec!["alpha.txt", "alphabet.txt"]);
        assert!(index.search("alpha", 0).is_empty());
    }

    #[test]
    fn retains_entries_when_the_source_reports_recoverable_issues() {
        let index = SearchIndex::from_source(&TestSource {
            entries: vec![entry("available.txt", r"C:\available.txt")],
            issues: vec![IndexIssue {
                path: PathBuf::from(r"C:\restricted"),
                message: "Access is denied".to_owned(),
            }],
        })
        .unwrap();

        assert_eq!(index.len(), 1);
        assert_eq!(index.indexing_issue_count(), 1);
    }
}
