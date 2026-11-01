# OxiFind Operations & Deployment Guide

This guide covers installation, permission requirements, operational architecture, diagnostics, and uninstallation across Windows, Linux, and macOS.

---

## 1. Architecture & Execution Modes

OxiFind provides high-performance, real-time file searching through two complementary indexing backends:

```
                  +---------------------------------------------+
                  |            OxiFind Search Engine            |
                  |     (In-Memory Index + Persistence Cache)   |
                  +---------------------------------------------+
                                     ^
                                     |
           +-------------------------+-------------------------+
           |                                                   |
   [ Windows Elevated ]                               [ Cross-Platform ]
   NTFS MFT Engine                                    Directory Crawler Engine
   - Direct raw volume access (\Device\HarddiskVolume) - Parallel work-stealing crawler
   - FSCTL_ENUM_USN_DATA MFT enumeration               - Multi-threaded traversal
   - FSCTL_READ_USN_JOURNAL change streaming           - OS event watcher (notify/inotify/FSEvents)
   - Zero disk walking                                 - Non-privileged execution
```

### Mode Comparison

| Feature | NTFS MFT Mode (Windows) | Directory Crawler Mode (All Platforms) |
|---|---|---|
| **Privileges Required** | Elevated (Administrator / `SeBackupPrivilege`) | Standard User (Unprivileged) |
| **Initial Index Time** | ~1–3 seconds for 1,000,000 files | ~5–15 seconds depending on storage speed |
| **Change Detection** | Hardware USN Change Journal stream | Filesystem notify events (`Inotify`, `FSEvents`, `ReadDirectoryChangesW`) |
| **Supported Filesystems** | NTFS | All filesystems (NTFS, ext4, btrfs, APFS, FAT32, exFAT, ReFS) |

---

## 2. Installation & Distribution

### Windows

#### Option A: Inno Setup Installer (`OxiFind-<version>-Setup.exe`)
* Run the installer. It supports both per-user (`%LOCALAPPDATA%\Programs\OxiFind`) and system-wide installation.
* **Optional Tasks:**
  * Add Desktop shortcut.
  * Add OxiFind to the user/system `PATH`.
  * Add **"Search with OxiFind"** to Explorer context menus for drives and folders.

#### Option B: Portable ZIP Package
Extract `oxifind.exe` to any folder. No registry entries or administrative installation are required.

#### Option C: Windows Terminal / CLI
```powershell
# Run GUI
oxifind.exe

# Run interactive Terminal User Interface (TUI)
oxifind.exe --tui

# Search directly from PowerShell / CMD
oxifind.exe --cli "report sort:relevance"
```

### Linux
* **Binary Location:** Place `oxifind` in `/usr/local/bin` or `~/.local/bin`.
* **Desktop Launcher:** Install `packaging/linux/oxifind.desktop` to `~/.local/share/applications/`:
  ```bash
  cp packaging/linux/oxifind.desktop ~/.local/share/applications/
  update-desktop-database ~/.local/share/applications/
  ```

### macOS
* **App Bundle:** Place `OxiFind.app` (configured via `packaging/macos/Info.plist`) in `/Applications`.
* **CLI/TUI:** Run directly from Terminal: `oxifind --tui`.

---

## 3. Permissions & Security Model

### Windows: Elevation & `SeBackupPrivilege`
* **Default Execution:** By default, OxiFind's application manifest specifies `requestedExecutionLevel="asInvoker"`. This guarantees that standard users can launch the application without unexpected UAC prompts.
* **Elevated Operation:** To access raw volume handles (`\\.\C:`) and the NTFS Master File Table, the process requires administrative privileges (`SeBackupPrivilege`).
* **On-Demand Elevation:** When running without administrative privileges on Windows, OxiFind defaults to the cross-platform directory crawler. The user can elevate on demand via the UI menu or restart with elevated rights using the platform elevation abstraction (`platform::elevation`).

### Linux & macOS
* Standard user permissions are sufficient to index all readable directories.
* Permission-denied errors (`EACCES`) on restricted directories (e.g. `/root`, `/var/spool`) are safely reported as isolated issues without terminating traversal.

---

## 4. Persistent Storage & Configuration

OxiFind caches index snapshots and configuration in platform-specific application data directories:

| OS | Cache Directory | Files |
|---|---|---|
| **Windows** | `%LOCALAPPDATA%\OxiFind\` | `index.snapshot`, `settings.json` |
| **Linux** | `~/.local/share/oxifind/` (or `$XDG_DATA_HOME/oxifind`) | `index.snapshot`, `settings.json` |
| **macOS** | `~/Library/Application Support/OxiFind/` | `index.snapshot`, `settings.json` |

### Snapshot Integrity (`index.snapshot`)
* **Header:** 8-byte magic tag `OXIFIND1`.
* **Version:** 4-byte little-endian format version (`CURRENT_VERSION = 1`).
* **Payload Length:** 8-byte payload size tracking.
* **Integrity:** IEEE 802.3 CRC-32 checksum validated on load.
* **Atomicity:** Written to a temporary file (`.tmp.<pid>.<timestamp>`) and swapped atomically using OS rename semantics.

### Settings Configuration (`settings.json`)
```json
{
  "indexed_paths": ["C:\\", "D:\\Data"],
  "excluded_patterns": ["*/.git/*", "*/target/*", "*/node_modules/*"],
  "max_results": 1000
}
```

---

## 5. Diagnostics & Operational Recovery

### USN Journal Wrap / Purge Recovery
If the NTFS USN Change Journal wraps or older records are purged while OxiFind is offline (`checkpoint.next_usn < state.first_usn`):
1. OxiFind automatically detects the journal gap.
2. An informative diagnostic event is logged: `Rebuilt volume C:\ because USN journal records from X up to Y were purged`.
3. The volume is completely re-enumerated in the background without dropping existing search capability.
4. Checkpoints are synchronized to the active journal state.

### USN Journal Recreation Recovery
If an administrator deletes and recreates the USN journal (`fsutil usn deletejournal`):
1. OxiFind detects the changed journal identifier (`state.journal_id != checkpoint.journal_id`).
2. An automatic volume replacement is triggered with zero user intervention required.

### Corrupted Snapshot Recovery
If an `index.snapshot` file is truncated, modified by third-party tools, or fails CRC-32 verification:
1. `load_snapshot()` rejects the corrupt payload.
2. The corrupt file is quarantined or superseded.
3. The engine falls back transparently to a clean initial index crawl.

---

## 6. Code Signing & Release Verification

Windows production executables and installers must be digitally signed with Authenticode.

### PowerShell Signing Script (`packaging/windows/sign-release.ps1`)
```powershell
# Sign with PFX certificate
.\packaging\windows\sign-release.ps1 `
    -TargetFile "target\release\oxifind.exe" `
    -CertFile "C:\certs\codesign.pfx" `
    -TimestampServer "http://timestamp.digicert.com"

# Sign installer
.\packaging\windows\sign-release.ps1 `
    -TargetFile "target\release\installer\OxiFind-0.1.0-Setup.exe" `
    -CertFile "C:\certs\codesign.pfx"
```

### Signature Verification
Verify signature integrity using Microsoft `signtool.exe`:
```powershell
signtool.exe verify /pa /v target\release\oxifind.exe
```

---

## 7. Uninstallation & Data Removal

### Windows
1. Open **Settings > Apps > Installed apps** (or **Control Panel > Programs and Features**).
2. Select **OxiFind** and click **Uninstall**.
3. The uninstaller removes:
   * Program executables and icons.
   * Explorer context menu registry entries.
   * PATH environment variable entries.
   * Application snapshot cache (`%LOCALAPPDATA%\OxiFind`).

### Linux / macOS Manual Removal
```bash
# Remove binary & desktop entry
rm -f ~/.local/bin/oxifind
rm -f ~/.local/share/applications/oxifind.desktop

# Purge cached snapshots and settings
rm -rf ~/.local/share/oxifind ~/.config/oxifind
# On macOS:
rm -rf ~/Library/Application\ Support/OxiFind
```
