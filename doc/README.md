# Everything Lite

Everything Lite is an experimental Rust application inspired by
[Everything](https://www.voidtools.com/), the Windows file search utility. The
project explores how to build a responsive name-based file search experience in
Rust. It is independent of and not affiliated with Voidtools or Everything.

## Current functionality

- A native desktop UI built with [egui](https://github.com/emilk/egui)
  (eframe), with a menu bar, a search field, a sortable result table, and a
  status bar (see [Using the window](#using-the-window)).
- Each result shows its file name, containing folder, and type (file or
  folder, plus read-only, hidden, and system attributes). Double-clicking a
  result opens it; its context menu can also show it in its folder in
  Explorer or copy its name or full path.
- Recursive indexing of the directory from which the application is started.
- Case-insensitive search by file name or path, with multiple terms, prefix,
  exact-name, path-only, and quoted terms (see [Search syntax](#search-syntax)).
- Source-independent entry metadata for file and parent identifiers, name,
  full path, volume identity, and file attributes.
- An index-source contract with separate initial snapshots and incremental
  updates, including volume USN journal checkpoints.
- Parent-child path reconstruction that preserves records with missing parents
  or cycles and reports them as indexing issues.
- Deterministic synthetic index fixtures with 100, 10,000, and 100,000
  entries for future search benchmarks.
- Result ordering that sums a rank per query term: a file name beginning with
  the term ranks first, then a file name containing it, then a matching path.
- Deterministic case-insensitive ordering for equally ranked results, plus
  `sort:` terms for name, path, and folder-first ordering.
- Asynchronous search execution to keep the UI responsive while typing.
- Initial indexing runs in a background worker so the UI remains responsive.
- Startup status shows the index root, elapsed indexing time, and any
  inaccessible child paths.
- Continued indexing after inaccessible child paths.
- `NtfsMftSource` validates a selected volume before any MFT access is
  attempted: it confirms the volume is NTFS-formatted and that the current
  process can open it for raw read access, using the documented
  `GetVolumeInformationW` and `CreateFileW` Win32 APIs.
- Enumeration of a validated volume's active master file table records with
  the documented `FSCTL_ENUM_USN_DATA` device control, which reports the
  records without walking directories. Each record contributes a file
  identifier, parent identifier, name, file attributes, and volume identity,
  anchored to the volume root so full paths are rebuilt from parent-child
  relationships.
- A deterministic cross-source test that indexes a real temporary workspace
  tree and replays matching synthetic MFT records through the existing seam.
  It verifies that shared reconstruction yields the same representative nested
  relative paths and names from both sources.
- Defensive USN record parsing: record lengths, record versions, and
  file-name ranges are bounds-checked, deleted records are skipped, damaged
  records become recoverable indexing issues, and an enumeration without
  usable records fails explicitly instead of reporting an empty index.
- `NtfsMultiVolumeSource` combines several selected NTFS volumes into one
  snapshot: each volume is validated and enumerated independently, a volume
  that fails is reported as a recoverable indexing issue tied to its own path
  without discarding entries already collected from other volumes, equivalent
  selected roots are indexed only once, and collection fails explicitly only
  when no volume was selected or every selected volume failed.
- Each completed NTFS snapshot records the USN journal identifier and
  next-USN position using `FSCTL_QUERY_USN_JOURNAL`; multi-volume snapshots
  publish a volume only when its matching checkpoint was also recorded.
- Selecting a drive root such as `C:\` indexes that volume from its master
  file table. A background poller then reads the USN Change Journal once per
  second and applies creates, renames, moves, and deletes to the running
  index; a reset, wrapped, or gapped journal rebuilds the volume. Open result
  lists are refreshed after every applied change batch.

The recursive workspace index skips `target` and `.git` directories. An empty
query displays the first 200 indexed entries.

## Search syntax

Whitespace separates terms, and a result must match every term. Matching
ignores case.

| Term | Matches |
| --- | --- |
| `text` | File names or full paths containing `text`. |
| `^text` | File names starting with `text`. |
| `=text` | File names equal to `text`. |
| `path:text` | Full paths containing `text`; the file name alone is not considered. |
| `"two words"` | Quotes keep whitespace inside one term and can follow a modifier, such as `path:"Program Files"`. |

Modifiers are only recognized before any quote, so `"^text"` searches for the
literal text `^text`. Terms without text, such as a lone `^`, are ignored, and
a query without terms lists the first indexed entries.

A `sort:` term orders the results instead of filtering them:

| Term | Order |
| --- | --- |
| `sort:relevance` | Default: file-name prefix matches, then file-name matches, then path matches. |
| `sort:name` | File name. |
| `sort:path` | Full path. |
| `sort:type` | Folders before files. |

A leading `-`, such as `sort:-name`, reverses the order. Ties are broken by
relevance and then by the displayed path, so every order is deterministic. The
last valid `sort:` term wins, and an unknown key such as `sort:size` is searched
for as ordinary text.

## Using the window

Clicking the **Name**, **Path**, or **Type** column header sorts by that
column; clicking it again reverses the order. **View > Sort by** also offers
relevance and the direction. A `sort:` term typed into the query overrides the
chosen column.

| Shortcut | Action |
| --- | --- |
| `Ctrl+F` | Focus the search field. |
| `Enter` | Open the selected result, or the first one if none is selected. |
| `Ctrl+Enter` | Show the selected result in its folder. |
| `Ctrl+Shift+C` | Copy the selected result's full path. |
| `Esc` | Clear the search. |
| `Ctrl++` / `Ctrl+-` | Zoom in or out. |

The **View** menu also toggles the status bar and switches between the system,
light, and dark themes. **Help > Search syntax** lists the query syntax.

## Running the application

Install a supported Rust toolchain, then run the application with an optional
directory to index:

```powershell
cargo run -- C:\path\to\index
```

Without an argument, the current working directory becomes the index root.
The **Path** column shows each result's containing folder.

To index a whole NTFS volume and keep it up to date, pass its drive root from
an elevated terminal:

```powershell
cargo run -- C:\
```

Raw volume access requires administrator rights. Without them, indexing fails
with an explicit error instead of falling back to walking the drive.

## Architecture

| Module | Responsibility |
| --- | --- |
| `src/main.rs` | Starts the application and reports startup errors. |
| `src/app.rs` | Defines the egui UI, selects the index source, coordinates search requests, and applies background journal updates. |
| `src/search.rs` | Defines source-independent entry metadata, update contracts, and journal checkpoints; collects workspace entries, stores the index, and ranks queries. |
| `src/ntfs.rs` | Validates NTFS volume access, enumerates a volume's active master file table records into index entries, records USN journal checkpoints, and composes several validated volumes into one snapshot. |

## Current limitations

This is a prototype, not a replacement for Everything:

- It indexes one selected directory or drive at startup rather than all
  system volumes.
- Directory indexing walks the file system recursively, so it can be slow for
  large directories, and it is not updated after startup.
- The index is not persisted.
- `NtfsMftSource` requires the elevated privilege that opening a volume for
  raw read access needs. `NtfsMultiVolumeSource` composes several such
  sources, but the application only selects a single volume. The synthetic
  MFT and journal replays do not exercise a live volume's contents or
  elevated raw-read permission; live validation must be run elevated against
  a known NTFS volume.
- Regular expressions, attribute filters, and content-based search are not
  implemented.

## Planned direction

The intended next steps are to select several volumes in the application,
add search syntax while preserving fast, low-overhead interactive results,
and persist index snapshots with their journal checkpoints.
