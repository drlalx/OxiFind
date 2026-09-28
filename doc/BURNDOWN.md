# Development Burndown

Updated: 2026-09-28

| Milestone | Planned items | Complete | Remaining |
| --- | ---: | ---: | ---: |
| 1. Stabilize the Prototype | 4 | 4 | 0 |
| 2. Establish the Index Model | 4 | 4 | 0 |
| 3. Implement NTFS Initial Indexing | 4 | 4 | 0 |
| 4. Maintain the Index | 4 | 4 | 0 |
| 5. Improve Search and User Experience | 6 | 4 | 2 |
| 6. Persistence, Packaging, and Operations | 5 | 0 | 5 |
| **Total** | **27** | **20** | **7** |

```text
Completed [####################.......] 20/27
Remaining [#######....................] 7/27
```

Milestone 1 is complete. Workspace traversal, relative paths, `target`/`.git`
exclusions, and a child directory whose listing is denied by an explicit ACL
are covered against a real temporary directory tree.

Milestone 2 now has its source-independent entry metadata model. NTFS-specific
identifiers and attributes are populated by the MFT source. Its
snapshot/update source contract now represents upserts and removals explicitly.
Parent-child relationships reconstruct full paths and report orphaned or
cyclic records without discarding them.

Milestone 2 is complete; deterministic fixtures contain 100, 10,000, and
100,000 synthetic entries for future benchmarks.

Milestone 3 has its volume-validation foundation. `NtfsMftSource` resolves the
selected volume's drive root, reads its filesystem name, and confirms raw read
access using the documented `GetVolumeInformationW` and `CreateFileW` Win32
APIs (via the `windows` crate). Unsupported filesystems, inaccessible volumes,
and paths without a local drive letter each return an explicit `io::Error`
with no silent fallback.

Validated volumes are now enumerated with the documented `FSCTL_ENUM_USN_DATA`
device control behind an injectable record-enumeration seam. Active records
become index entries with file and parent identifiers, names, attributes, and
volume identity, anchored to the volume root for the shared path resolver.
Record lengths, versions, and name ranges are bounds-checked; unusable records
are reported as recoverable issues, while deleted records are skipped; an
enumeration without usable records fails explicitly instead of reporting an
empty index.

`NtfsMultiVolumeSource` now composes several single-volume `NtfsMftSource`
instances into one snapshot. Each selected volume is validated and enumerated
independently; a volume that fails validation or enumeration is reported as a
recoverable `IndexIssue` tied to its own path, without discarding entries
already collected from other volumes, regardless of whether the failure
happens before or after a successful volume. Equivalent selected roots (for
example `C:\` and `C:\Users`) are deduplicated so a volume is only indexed
once, and `collect_initial` returns an explicit error only when no volume was
selected or every selected volume failed. A deterministic cross-source test
now builds a real temporary workspace tree and replays matching synthetic MFT
records through the existing seam, asserting the same expected representative
nested paths and names after shared reconstruction. It does not exercise live
MFT data or the elevated raw-volume permission requirement; those limits need
an elevated run against a known NTFS volume.

Milestone 4 has started. Each completed MFT volume snapshot records its USN
journal identifier and next-USN position through the documented
`FSCTL_QUERY_USN_JOURNAL` control code. `SearchIndex` retains those
source-independent checkpoints alongside the snapshot. In multi-volume
operation, a volume is only published after both enumeration and checkpoint
collection succeed; a checkpoint failure is a recoverable issue and leaves
that volume out of the current snapshot.

Journal changes after each checkpoint are now read through the documented
`FSCTL_READ_USN_JOURNAL` control code and applied in order through
`SearchIndex::refresh`. Creates, renames, moves, and deletes update entries
keyed by volume and file identifier; directory renames move their
descendants; and a failed read leaves both entries and checkpoints unchanged.
Path resolution is now scoped per volume, which fixes parent lookups when
several volumes share the same file reference numbers.

Journal resets, wraps, and gaps are detected by comparing each checkpoint with
the current journal state before reading. An affected volume is re-enumerated
and replaced as a whole, with the reason reported as an indexing issue.

Milestone 4 is complete. A drive-root argument starts the NTFS source in the
application; a background poller collects journal update batches once per
second, and the search worker applies each batch in one step and refreshes
the latest query, so the UI always receives results from a single consistent
snapshot. Live validation against a real volume still requires an elevated
run.

Milestone 5 has started with a documented search syntax: whitespace-separated
terms must all match, `^`, `=`, and `path:` restrict a term to file-name
prefixes, whole file names, or full paths, and quotes keep whitespace inside a
term. Results are ranked by the sum of their per-term ranks.

`sort:` terms now order results by relevance, file name, full path, or type
(folders first), optionally reversed, with deterministic tie-breakers.

Results show their type, key attributes, and full path. Double-clicking a
result opens it, and **Show in folder** opens Explorer with the item selected.

The UI now uses eframe/egui with a menu bar, a sortable result table with a
row context menu, and a status bar, following the mockup in `doc/mockup`.
