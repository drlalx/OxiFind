# Architektur-Analyse & Referenz-Implementierungen

Dieses Dokument analysiert vier spezialisierte Rust-Dateisuch-Projekte, die im Verzeichnis `examples/` als Referenz-Codebases ausgecheckt wurden:

1. **`examples/fd`** ([sharkdp/fd](https://github.com/sharkdp/fd)) – Paralleler Multiplatform-CLI-Dateisucher
2. **`examples/file-minnow-linux`** ([Puddings22/File-Minnow-Linux](https://github.com/Puddings22/File-Minnow-Linux)) – Linux-nativer „Everything“-Klon (GTK3)
3. **`examples/goz`** ([mustafaahci/goz](https://github.com/mustafaahci/goz)) – Windows NTFS MFT/USN-Daemon & Search Engine
4. **`examples/flash_find`** ([kcvabeysinghe/flash_find](https://github.com/kcvabeysinghe/flash_find)) – In-Memory-Suche mit FST & Iced GUI (Windows)

---

## 1. Detaillierte Analyse der einzelnen Projekte

### 1.1 `fd` (`examples/fd`)
- **Fokus & Typ**: Multiplatform-Kommandozeilen-Werkzeug zur schnellen Dateinamensuche (moderner `find`-Ersatz).
- **Dateisystem-Traversal**:
  - Nutzt das Crate `ignore` (`ignore::WalkBuilder`), welches intern auf `crossbeam-channel` und Multi-Thread-Work-Stealing basiert.
  - Erkennt und respektiert standardmäßig `.gitignore`, `.ignore`, versteckte Dateien und Dateitypen (`--type f`, `--type d`, `--type symlink`).
- **Such- & Index-Strategie**:
  - **Kein permanenter Index**: `fd` durchsucht das Dateisystem ad-hoc bei jeder Abfrage.
  - Verwendet `aho-corasick` für Multi-Pattern-String-Matching und `regex` für reguläre Ausdrücke.
- **Erkenntnisse für OxiFind**:
  - Robuste Filterlogik für Symlink-Schleifen, Permission-Denied-Handling und plattformübergreifende Pfadnormalisierung (`normpath`).

---

### 1.2 `File-Minnow-Linux` (`examples/file-minnow-linux`)
- **Fokus & Typ**: Desktop-Anwendung ausschließlich für Linux, orientiert am Bedienkonzept von Voidtools *Everything*.
- **Architektur & GUI**:
  - Native GTK3-Oberfläche (`gtk = "0.18"`, `gio`), System-Tray-Integration (`ksni`), globale Hotkeys unter X11 (`x11rb`) und Wayland Desktop Portal (`ashpd`).
- **Dateisystem-Erkennung & Indexierung**:
  - Verwendet `walkdir` für das initiale Einlesen des Dateibaums.
  - Liest `/proc/mounts` in `mounts.rs` aus und filtert Pseudodateisysteme (`proc`, `sysfs`, `devtmpfs`, `cgroup`) aus.
- **Live-Monitoring**:
  - Überwacht Verzeichnisse in Echtzeit über `inotify = "0.11"` (`src/watch.rs`).
- **Such- & Speicherstrategie**:
  - In-Memory-Speicherung aller Pfade in `src/index.rs` und `src/store.rs`, serialisiert auf Festplatte für schnelle Programmstarts.
- **Erkenntnisse für OxiFind**:
  - Praktisches Vorbild für Linux-spezifische Features wie Desktop-Integration, Mime-Type-Erkennung und Wayland/X11-Hotkey-Portals.

---

### 1.3 `goz` (`examples/goz`)
- **Fokus & Typ**: Hochoptimierte Windows-NTFS-Dateisuche mit nativer Kernel-Ioctl-Anbindung.
- **Architektur**:
  - Mehrschichtige Workspace-Architektur:
    - `goz-winfs`: Direkte Windows-API- und IOCTL-Aufrufe.
    - `goz-core`: Datenstrukturen, Indizes, Query-Compiler und UTF-16/WTF-8-String-Handling.
    - `goz-daemon`: Hintergrunddienst mit Administratorrechten.
    - `goz-cli`: Client-Tool.
- **NTFS MFT & USN-Journaling**:
  - Liest das Master File Table (MFT) über `FSCTL_ENUM_USN_DATA` direkt aus.
  - Überwacht Dateiänderungen live über `FSCTL_READ_USN_JOURNAL`.
  - Trennung zwischen Daemon (Admin-Rechte für Rohzugriff) und Client (über Windows Named Pipes mit Security Descriptors `pipe_sd.rs`).
- **Query & Kompatibilität**:
  - Bietet `escompat` (Kompatibilität mit der Syntax des *Everything* CLI-Tools `es.exe`).
- **Erkenntnisse für OxiFind**:
  - Sauberes Handling von Windows-Eigenheiten wie unvollständigen UTF-16-Surrogaten via WTF-8 (`wtf8.rs`) und IPC über Named Pipes.

---

### 1.4 `Flash Find` (`examples/flash_find`)
- **Fokus & Typ**: Minimalistische Windows-Desktop-App mit extrem schneller In-Memory-Volltextsuche.
- **Architektur & GUI**:
  - Komplettes Projekt in einer einzigen Datei (`src/main.rs`, ~540 Zeilen).
  - Modernes, reaktives GUI mit `iced = "0.12"` (Elm-Architektur).
- **Indexierung & Persistenz**:
  - Nutzt `jwalk` (paralleles Directory-Walking über Rayon) für die Laufwerks-Indexierung.
  - Persistiert Pfade hochkomprimiert auf der Festplatte via `fst` (Finite State Transducer / `fst::SetBuilder`).
- **Such-Algorithmus**:
  - Lädt beim Start alle Pfade aus der FST-Datei in den RAM (`Arc<Vec<String>>`).
  - Bei jedem Tastendruck startet eine parallele Rayon-Pipeline (`cache.par_iter().fold(...)`).
  - Jeder Worker-Thread führt einen lokalen `BinaryHeap` (Min-Heap für Top 50 Ergebnisse) mit heuristischem Scoring:
    - Exakter Treffer: Score 10.000
    - Prefix-Treffer: Score 5.000
    - Substring im Dateinamen: Score 100
    - Substring im Pfad: Score 10
  - Durchsucht ~1,5 Millionen Dateien in unter 30 ms auf normaler Consumer-Hardware.
- **Erkenntnisse für OxiFind**:
  - Heuristisches Top-K-Heap-Scoring mit Rayon-Parallelismus liefert extrem geringe Latenz bei Tastatureingaben.

---

## 2. Vergleichsmatrix mit OxiFind

| Eigenschaft | `fd` | `File-Minnow-Linux` | `goz` | `Flash Find` | **OxiFind** |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Plattformen** | Multi (CLI) | Nur Linux | Nur Windows | Nur Windows | **Windows, Linux, macOS** |
| **GUI** | Keine (CLI) | GTK3 | Keine (CLI/Daemon) | Iced | **egui / eframe** |
| **Index-Art** | Kein Index | In-Memory / File | In-Memory Daemon | FST Disk + RAM | **In-Memory Slot-Index** |
| **Windows NTFS** | Standard-FS-API | N/A | Direct MFT / USN | `jwalk` | **MFT Enumeration + USN** |
| **Unix Indexer** | `ignore` (Walk) | `walkdir` | N/A | N/A | **Paralleler Work-Stealer** |
| **Live Monitoring** | Keines | `inotify` | USN Journal | Keines (Re-Index) | **USN (Win) + `notify` (Unix)** |
| **Rechte-Handling** | Normal | Normal | Named-Pipe Daemon | Normal | **Auto UAC / pkexec / osascript** |
| **Code-Qualität** | Sehr hoch | Gut | Sehr modular | Sehr kompakt | **No-Panic Policy + 100% MIT** |
