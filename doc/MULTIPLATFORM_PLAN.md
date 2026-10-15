# OxiFind Multiplatform: Architektur- & Entwicklungsplan (Windows, Linux, macOS)

Dieses Dokument spezifiziert die Architektur, Richtlinien und Meilensteine zur Weiterentwicklung von **OxiFind** zu einer nativen **Multiplatform-Anwendung** für **Windows**, **Linux** und **macOS**.

---

## 1. Leitprinzipien & Richtlinien

1. **Echte Multiplatform-Unterstützung**:
   - Keine separate Portierung oder Verzweigung, sondern eine einheitliche Codebasis mit modularer Plattformabstraktion (`src/platform/`).
   - Erstklassige Unterstützung für Windows (MFT/USN), Linux (Parallel-Crawler/Inotify) und macOS (Parallel-Crawler/FSEvents/Finder).
2. **No-Panic Policy (Clippy)**:
   - In `Cargo.toml` sind panikbehaftete Konstrukte im Produktionscode strikt untersagt (`panic`, `unwrap_used`, `expect_used`, `panic_in_result_fn`, `unimplemented`, `todo`, `unreachable` stehen auf `deny`).
   - Jeder Fehlerfall wird defensiv über `Result` und `Option` gehandhabt.
3. **Lizenz-Whitelist (Nur MIT-kompatible Lizenzen)**:
   - Alle direkten und transitiven Abhängigkeiten müssen eine MIT-kompatible, permissive Open-Source-Lizenz besitzen.
   - Erlaubt: `MIT`, `Apache-2.0`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `0BSD`, `Zlib`, `Unicode-3.0`, `BSL-1.0`, `OFL-1.1`, `Ubuntu-font-1.0`, `Unlicense`, `CC0-1.0`.
   - Durchsetzung: Konfiguriert in `deny.toml` und automatisiert bei jedem Testlauf verifiziert über `tests/license_compliance.rs`.
4. **Test-Driven Development (TDD)**:
   - Neue Funktionalität wird zuerst durch Tests beschrieben und anschließend umgesetzt.

---

## 2. Zielarchitektur

```
                                  ┌───────────────────────────┐
                                  │   eframe / egui Desktop   │
                                  │ (Windows, Linux, macOS)   │
                                  └─────────────┬─────────────┘
                                                │
                                  ┌─────────────▼─────────────┐
                                  │      SearchIndex Core     │
                                  │    (Plattform-Agnostisch) │
                                  └─────────────┬─────────────┘
                                                │ Trait IndexSource
        ┌───────────────────────────────────────┼──────────────────────────────────────┐
        ▼                                       ▼                                      ▼
┌───────────────────────────────┐ ┌───────────────────────────────┐ ┌───────────────────────────────┐
│       Windows Backend         │ │        Linux Backend          │ │        macOS Backend          │
├───────────────────────────────┤ ├───────────────────────────────┤ ├───────────────────────────────┤
│ • Indexer: NtfsMftSource      │ │ • Indexer: LinuxDirectory-    │ │ • Indexer: MacDirectory-      │
│   (raw FSCTL_ENUM_USN_DATA)   │ │   Source (Parallel-Crawler,   │ │   Source (Parallel-Crawler,   │
│ • Watcher: USN Journal Poller │ │   Mount-Erkennung, Exclusions)│ │   /Volumes & APFS-Mounts)     │
│ • Desktop: explorer.exe       │ │ • Watcher: Inotify / Poller   │ │ • Watcher: FSEvents / Poller  │
│ • Elevation: UAC Token/Runas  │ │ • Desktop: xdg-open & DBus    │ │ • Desktop: open & open -R     │
│                               │ │   FileManager1.ShowItems      │ │   (Finder Reveal)             │
│                               │ │ • Elevation: pkexec / geteuid │ │ • Elevation: Authorization    │
│                               │ │                               │ │   Services / geteuid          │
└───────────────────────────────┘ └───────────────────────────────┘ └───────────────────────────────┘
```

---

## 3. Plattformspezifische Matrix

| Funktion | Windows | Linux | macOS |
| :--- | :--- | :--- | :--- |
| **Initial-Index** | `NtfsMultiVolumeSource` (MFT ioctl) | `LinuxDirectorySource` (Parallel `jwalk`/Threads) | `MacDirectorySource` (Parallel `jwalk`/Threads) |
| **Default-Scope** | Alle lokalen NTFS-Laufwerke (`C:\`, `D:\`) | Root `/` (Exclusions) oder `$HOME` | Root `/` und `/Volumes/*` |
| **Exclusions** | Recycle Bin, System Volume Information | `/proc`, `/sys`, `/dev`, `/run`, `/snap`, `/tmp` | `.Spotlight-V100`, `.fseventsd`, `/System/Volumes/Data` |
| **Live-Updates** | USN Change Journal (`FSCTL_READ_USN_JOURNAL`) | Inotify / Dateisystem-Events | FSEvents / Dateisystem-Events |
| **Datei öffnen** | ShellExecute / `explorer.exe "<path>"` | `xdg-open "<path>"` | `open "<path>"` |
| **Im Dateimanager zeigen** | `explorer.exe /select,"<path>"` | DBus `FileManager1.ShowItems` (Fallback: `xdg-open`) | `open -R "<path>"` |
| **Rechte / Elevation** | Windows UAC Token (`runas`) | Polkit (`pkexec`) / `geteuid() == 0` | `osascript` admin / `geteuid() == 0` |
| **Attribute** | Read-Only, Hidden, System, Folder | Permissions, Dotfile-Hidden, Symlink, Folder | Permissions, Dotfile/Finder-Hidden, Symlink, Folder |

---

## 4. Geplante Meilensteine (TDD)

### Meilenstein M1: Plattformunabhängigkeit des Kerns & Test-Harmonisierung
1. [x] **No-Panic Policy**: Clippy-Lints (`panic = "deny"`, `unwrap_used = "deny"`, etc.) in `Cargo.toml` aktivieren.
2. [x] **Lizenz-Compliance**: `deny.toml` und `tests/license_compliance.rs` einrichten.
3. [x] **Pfad-Normalisierung**: `search.rs` und Tests von Windows-spezifischen Annahmen (`\`, `C:\`) befreien.
4. [x] **Test-Suite grün**: `cargo test` läuft auf allen Zielbetriebssystemen fehlerfrei durch.

### Meilenstein M2: Plattform-Abstraktionsschicht (`src/platform/`)
1. [x] Strukturierung in `src/platform/{mod.rs, desktop.rs, elevation.rs, mounts.rs}`.
2. [x] Desktop-Aktionen: `open_item` und `reveal_in_file_manager` für Windows (`explorer`), Linux (`xdg-open` + DBus) und macOS (`open -R`).
3. [x] Attribut-Formatierung für POSIX und Windows.
4. [x] Plattformspezifische Elevation-Erkennung (`geteuid == 0`, `pkexec`, `osascript`, `TOKEN_ELEVATION`).

### Meilenstein M3: Parallele Unix-Indexierung (Linux & macOS)
1. [x] Implementierung des performanten Multi-Thread-Crawlers (`DirectorySource` in `src/crawler.rs`).
2. [x] Automatische Mount-Point-Erkennung (Linux `/proc/mounts`, macOS `/Volumes`).
3. [x] Systempfad-Ausschlüsse (`/proc`, `/sys`, `/dev`, `/run`, `/tmp`, `.Spotlight-V100`, etc.).
4. [x] Fehlertoleranz: Überspringen geschützter Verzeichnisse (`EACCES`) als `IndexIssue`.

### Meilenstein M4: Plattformübergreifende Live-Aktualisierung
1. [x] Windows: Bestehender USN Journal Poller bleibt aktiv für NTFS-Volumes.
2. [x] Linux & macOS: Event-basierter Watcher (`DirectoryWatcher` via `notify`-Crate in `src/watcher.rs`) mit Batch-Verarbeitung.
3. [x] Graceful Fallback auf periodisches Polling bei Watch-Limit-Erschöpfung (`ENOSPC`) oder fehlenden Rechten.

### Meilenstein M5: Multiplatform-Validierung & CI
1. [x] CI-Matrix-Test für Windows, Ubuntu (Linux) und macOS (`.github/workflows/ci.yml`).
2. [x] `.desktop`-Integration für Linux (`packaging/linux/oxifind.desktop`) und `.app`-Bundle-Vorbereitung für macOS (`packaging/macos/Info.plist`).
3. [x] Dokumentation in `doc/README.md` und `doc/BURNDOWN.md` aktualisieren.

