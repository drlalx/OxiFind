# OxiFind TUI: Architektur- & Entwicklungsplan (Ratatui & Crossterm)

Dieses Dokument spezifiziert die Konzeption, Architektur und schrittweise Umsetzung eines nativer **Terminal User Interface (TUI)** für **OxiFind**, angelehnt an das bewährte Muster in **OxiDisc**.

---

## 1. Motivation & Leitprinzipien

OxiFind verfügt aktuell über eine Desktop-Oberfläche auf Basis von `egui`/`eframe`. Für Remote-Server (SSH), Headless-Installationen, Minimal-Setups oder reine Terminal-Nutzer soll OxiFind eine vollwertige, reaktive Terminal-Oberfläche erhalten, ohne Einbußen bei der Suchgeschwindigkeit.

### Leitprinzipien
1. **Konsistenz mit OxiDisc**:
   - Modulare Cargo-Features: `egui-ui` und `tui` (standardmäßig beide aktiv).
   - Backend: `ratatui` (0.30) mit `crossterm` (0.29).
   - Farbpalette und Design abgestimmt auf das moderne Dark-Theme von egui.
2. **Keine Leistungskompromisse**:
   - Die TUI teilt sich exakt dieselbe Kern-Engine (`SearchIndex`, MFT/USN-Scanner, paralleler Linux/macOS-Crawler und Watcher).
   - Asynchrone Entkopplung: TUI-Rendering blockiert niemals den Hintergrund-Crawler.
   - Reaktive Tastatureingabe: Sofortige Typ-Ahead-Suche in < 1 ms bei Tastendruck.
3. **Strikte Qualitäts- & Lizenzrichtlinien**:
   - **No-Panic Policy**: Volle Einhaltung der Clippy-Deny-Vorgaben (`panic`, `unwrap`, `expect` verboten).
   - **Terminal-Sicherheit**: RAII-Guard stellt sicher, dass das Terminal bei jedem regulären Exit oder Fehler sauber in den Normalzustand zurückkehrt (`disable_raw_mode`, `LeaveAlternateScreen`).
   - **100 % MIT-kompatibel**: `ratatui` und `crossterm` sind unter MIT lizenziert.

---

## 2. Feature-Architektur & Abhängigkeiten

### 2.1 `Cargo.toml` Konfiguration
```toml
[features]
default = ["egui-ui", "tui"]
egui-ui = ["dep:eframe", "dep:egui", "dep:egui_extras"]
tui = ["dep:ratatui", "dep:crossterm"]

[dependencies]
# GUI (optional)
eframe = { version = "0.36", default-features = false, features = ["default_fonts", "wayland", "x11"], optional = true }
egui = { version = "0.36", optional = true }
egui_extras = { version = "0.36", optional = true }

# TUI (optional)
ratatui = { version = "0.30", default-features = false, features = ["crossterm"], optional = true }
crossterm = { version = "0.29", optional = true }

# Shared Core
notify = "8.2.0"
```

### 2.2 CLI-Auswahl & Fallback
OxiFind unterstützt sowohl explizite CLI-Flags als auch eine automatische Erkennung:
- `oxifind --tui` oder `oxifind --ui tui`: Startet die Terminal-Oberfläche.
- `oxifind --gui` oder `oxifind --ui egui`: Startet die Desktop-GUI.
- **Headless Auto-Detection**: Wenn kein Display-Server verfügbar ist (z. B. auf Linux/macOS weder `DISPLAY` noch `WAYLAND_DISPLAY` gesetzt ist), fällt OxiFind automatisch auf die TUI zurück, anstatt mit einem Verbindungsfehler abzubrechen.

---

## 3. UI-Design & Layout (Ratatui)

Das TUI-Layout orientiert sich am vertikalen Aufbau der egui-Oberfläche und nutzt das Farbschema aus OxiDisc:

```text
┌─ OxiFind v0.1.0 ────────────────────── [STATUS: 1,428,912 indexed] ─ [Scope: All Drives] ┐
│                                                                                           │
│  Search: [cargo test__________________________________________] [Aa] [ab] [\] [.*]        │
│                                                                                           │
├───────────────────────────────────────────────────────────────────────────────────────────┤
│  Name ▲                      │ Folder                             │ Size     │ Modified   │
├──────────────────────────────┼────────────────────────────────────┼──────────┼────────────┤
│ > cargo-test-runner.rs       │ src/crawler/tests                  │ 4.2 KB   │ 2026-10-14 │
│   cargo_test_helper.rs       │ tests/common                       │ 1.1 KB   │ 2026-10-13 │
│   test_cargo_integration.sh  │ scripts                            │ 840 B    │ 2026-10-10 │
│   cargo-test-output.log      │ target/debug                       │ 12.8 KB  │ 2026-10-09 │
│                                                                                           │
├───────────────────────────────────────────────────────────────────────────────────────────┤
│  Selected: /home/user/develop/oxifind/src/crawler/tests/cargo-test-runner.rs              │
│  Attributes: Regular File, Read-Write | Matches: 4 / 1,428,912 in 0.42 ms                 │
├───────────────────────────────────────────────────────────────────────────────────────────┤
│  <Enter> Open  <Ctrl+O> Reveal  <Tab> Scope  <F1/?> Help  <↑/↓/j/k> Navigate  <q> Quit    │
└───────────────────────────────────────────────────────────────────────────────────────────┘
```

### 3.1 Farbpalette (Dark Theme)
- **Hintergrund**: `Color::Rgb(22, 29, 41)` / Standard-Terminal-Background
- **Karten / Panels**: `Color::Rgb(26, 35, 50)`
- **Rahmen (Borders)**: `Color::Rgb(42, 54, 72)`
- **Aktiver Fokus / Akzent**: `Color::Rgb(0, 230, 210)` (Oxi-Cyan)
- **Text Hauptfarbe**: `Color::Rgb(230, 238, 248)`
- **Text Muted / Pfade**: `Color::Rgb(140, 155, 175)`
- **Highlight Suchtreffer**: `Color::Rgb(250, 204, 21)` (Gelb) oder Fett/Unterstrichen
- **Erfolg / Indexing**: `Color::Rgb(34, 197, 94)` (Grün)

---

## 4. TUI-Zustandsmaschine (`TuiApp`)

```rust
pub struct TuiApp {
    // Suchzustand
    query: String,
    cursor_position: usize,
    options: SearchOptions, // case_sensitive, whole_word, match_path, regex
    
    // Ergebnis- und Tabellenzustand
    table_state: TableState,
    selected_row: usize,
    sort_column: SortColumn,
    sort_ascending: bool,
    
    // Anbindung an Search Core
    search_index: Arc<RwLock<SearchIndex>>,
    search_results: Vec<SearchResult>,
    search_time_micros: u128,
    
    // UI-Fokus & Modale
    focused_widget: FocusedWidget, // SearchInput, ResultTable, ScopeSelector
    active_modal: Option<TuiModal>, // Help, ScopePicker, ElevationNotice
    status_message: Option<(String, Instant)>,
    
    should_quit: bool,
}
```

### 4.1 Tastaturbelegung (Keybindings)
| Taste | Aktion |
| :--- | :--- |
| **Zeicheneingabe** | Direkte Eingabe in die Suchzeile (kein Umweg über Moduswechsel) |
| **`Esc`** | Suchfeld leeren / Aktives Modal schließen |
| **`↓` / `j`** | Nächster Treffer in der Tabelle |
| **`↑` / `k`** | Vorheriger Treffer in der Tabelle |
| **`PageDown` / `PageUp`** | Seite vor- / zurückspringen |
| **`Enter`** | Datei mit System-Standardprogramm öffnen (`platform::desktop::open_path`) |
| **`Ctrl+O` / `Alt+O`** | Im Dateimanager hervorheben (`platform::desktop::reveal_path`) |
| **`Ctrl+C` / `y`** | Pfad der ausgewählten Datei in die Zwischenablage kopieren |
| **`Tab`** | Umschalten zwischen Suchfeld, Filter-Toggles und Tabelle |
| **`F1` / `?`** | Hilfe-Modal öffnen |
| **`F2`** | Scope-Auswahl (Laufwerke / Verzeichnis wechseln) |
| **`Ctrl+R` / `F5`** | Neu-Indexierung anstoßen |
| **`Ctrl+Q` / `q`** (wenn Tabelle fokussiert) | OxiFind beenden |

---

## 5. Implementierungs-Meilensteine

### Meilenstein T0: Feature-Entkopplung & Terminal-Guard
1. [x] `Cargo.toml`: Features `egui-ui` und `tui` einführen, optionale Abhängigkeiten setzen.
2. [x] `TerminalGuard`: RAII-Struktur für `enable_raw_mode()`, `EnterAlternateScreen`, `EnableMouseCapture` und automatische Aufräumung in `Drop`.
3. [x] Lizenzprüfung für `ratatui` und `crossterm` in `tests/license_compliance.rs` und `deny.toml` validieren.

### Meilenstein T1: Kern-Event-Loop & CLI-Dispatch
1. [x] `src/main.rs`: CLI-Parser erweitern um `--tui`, `--gui`, `--ui <backend>`.
2. [x] Event-Loop mit non-blocking `crossterm::event::poll(Duration::from_millis(50))`.
3. [x] Asynchrone Anbindung an `SearchIndex` (Worker meldet Fortschritt über Channel).

### Meilenstein T2: Ratatui UI-Komponenten
1. [x] **Header & Status**: Titelzeile, Indexfortschrittsbalken, Gesamtzahl indizierter Dateien.
2. [x] **Suchleiste**: Eingabefeld mit sichtbarem Cursor, Suchmodus-Indikatoren (`Aa`, `ab`, `\`, `.*`).
3. [x] **Ergebnistabelle**: `ratatui::widgets::Table` mit sortierbaren Spalten, Selektions-Hervorhebung und Formatierung (Dateigröße, Datum).
4. [x] **Statusleiste & Shortcut-Footer**: Trefferanzahl, Suchzeit in Millisekunden, Hotkey-Legende.

### Meilenstein T3: Interaktivität & Aktionen
1. [x] Typ-Ahead-Suche: Jeder Tastendruck aktualisiert das Suchergebnis live.
2. [x] Desktop-Aktionen: Integration von `open_path` und `reveal_path` bei `Enter` und `Ctrl+O`.
3. [x] Clipboard-Support (`y` / `Ctrl+C`).
4. [x] Modale Dialoge: Hilfe-Overlay (`F1`) und Scope-Auswahlliste (`F2`).

### Meilenstein T4: Qualitätssicherung & Tests
1. [x] TDD: Unit- und Integrationstests mit `ratatui::backend::TestBackend` (Prüfung von Rendering und Key-Events ohne echtes Terminal).
2. [x] No-Panic-Policy-Check: `cargo clippy --tests` ohne Warnings/Panics.
3. [x] Validierung auf Linux, Windows und macOS.
