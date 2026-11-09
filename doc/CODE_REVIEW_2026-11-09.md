# Code-Review des aktuellen Stands (2026-11-09)

Scope: uncommittete Änderungen auf `main` (Service-Modus, Desktop-Integration, CLI, Packaging).
Die Befunde wurden durch einen Review-Agenten ermittelt und nicht manuell gegengeprüft.

## Hoch

1. **`src/engine.rs:535`**: `run_service` beendet sich mit `Ok(())`, wenn `poll_updates` `Break` liefert (`Unsupported`). `DirectorySource` (Nicht-Windows) meldet vermutlich `Unsupported`.
   - systemd (`Restart=on-failure`) startet nicht neu, der Dienst bleibt inaktiv.
   - launchd (`KeepAlive=true`) startet in einer Schleife neu, ohne zu überwachen.
   - Die UI zeigt trotzdem einen laufenden Dienst.
2. **`src/engine.rs:470`**: Ein vorhandener Snapshot wird ohne Abgleich mit dem Dateisystem übernommen. Ein voller Crawl läuft nur ohne Snapshot. Änderungen während der Dienst gestoppt war, fehlen dauerhaft, und der veraltete Stand wird überschrieben.

## Mittel

3. **`src/platform/service.rs:154`**: systemd-Unit ohne `--delay` (Default 15 s) plus `ExecStartPre=sleep`, die Verzögerung wird doppelt angewendet. macOS-plist übergibt `--delay` korrekt.
4. **`src/platform/desktop.rs:99`**: macOS-AppleScript mit unescapten `"` und `\` im Dateipfad. Präparierte Dateinamen können Code ausführen. Bei Fehler stiller Fallback auf `open`.
5. **`src/app.rs:609`**: Service-Menü ruft `service_status()` pro Frame auf (Subprozess). Start/Stop/Install/Uninstall laufen synchron im UI-Thread.
6. **`src/platform/service.rs:252`**: Windows `schtasks /delay` als `0000:{sec:02}`, Sekunden müssen 0–59 sein. `--delay 90` schlägt fehl.
7. **`src/app.rs:200`**: `std::env::set_var` in `run()`, in `main` und in einem Test. Die SAFETY-Annahme (keine Threads) ist nicht garantiert. Der Default überschreibt `WGPU_BACKEND` aus `oxifind.desktop`.

## Niedrig

8. **`src/backend.rs:125`**: `--delay` ohne `--service` wird ignoriert, Behandlung doppelt kopiert. `--renderer` wird nicht validiert.
9. **`src/platform/desktop.rs:16`**: Relativer Pfad wie `file.txt` ergibt `parent() == Some("")`.
10. **`src/platform/service.rs:275`**: Windows-Uninstall/-Stop und macOS-Start/-Stop verwerfen den Exit-Status. Ungenutzter `PathBuf`-Import.

## Status (nach Umsetzung)

| # | Status | Anmerkung |
|---|--------|-----------|
| 1 | Behoben, Befund korrigiert | `DirectorySource` liefert nie `Unsupported`, also kein Abbruch. Der echte Fehler: Der Watcher lief auf einer anderen Instanz als dem Polling, daher meldete der Dienst nie Änderungen. Fix: eine Quellinstanz, `IndexSource::start_watching`. Test: `crawler::start_watching_reports_changes_without_full_crawl`. |
| 2 | Behoben | `DirectorySource` hat keinen persistenten Journal-Checkpoint, ein Snapshot lässt sich dort nicht fortsetzen. Er wird jetzt verworfen und neu gecrawlt (`IndexSource::can_resume_from_snapshot`). NTFS-Snapshots bleiben unverändert. Test: `crawler::directory_snapshots_are_not_resumed`. |
| 3 | Behoben | systemd-Unit übergibt `--delay` einmal, kein `ExecStartPre`. Tests in `platform::service`. |
| 4 | Behoben (nicht auf macOS verifiziert) | Pfad wird als `osascript`-Argument übergeben, nicht mehr in den Skripttext interpoliert. |
| 5 | Behoben | Statusabfrage (alle 2 s, nur bei geöffnetem Menü) und Aktionen laufen in Hintergrund-Threads, Ergebnisse kommen über einen Channel. Während einer Aktion sind die Buttons deaktiviert. Nicht manuell in der GUI getestet. |
| 6 | Behoben (nicht unter Windows verifiziert) | `schtasks_delay_value` erzeugt `mmmm:ss`. Test in `platform::service`. |
| 7 | Behoben | Die Default-Entscheidung ist eine reine Funktion (`default_graphics_backend`), der Test setzt keine Umgebungsvariablen mehr. `env WGPU_BACKEND=gl` aus `oxifind.desktop` entfernt, der Default in der App deckt das ab. Das `set_var` in `main` für `--renderer` läuft vor Threads und bleibt. |
| 8 | Behoben | `--delay` ohne passende Service-Aktion wird abgelehnt, `--renderer` validiert (gl, vulkan, metal, dx12). Tests in `tests/ui_backend_test.rs`. |
| 9 | Behoben | `containing_folder` mappt `file.txt` auf `.`. Tests in `platform::desktop`. |
| 10 | Behoben | Windows Uninstall/Stop und macOS Start/Stop prüfen den Exit-Status, ungenutzter Import entfernt. |

Stand nach zweiter Runde: alle Befunde umgesetzt bis auf die oben genannten Verifikationslücken.

Windows-Prüfung (pcdrju, Windows 10, Branch `fix/service-review-findings`): `cargo build`, `cargo clippy --all-targets` ohne Warnungen und `cargo test` grün. Dabei gefundene, bereits vorhandene Windows-Compile-Fehler sind behoben (`elevation.rs` Crate-Pfad, Typ-Mismatch der Quellen in `run_service`). Der Test `retains_workspace_entries_and_reports_inaccessible_directories` überspringt sich auf pcdrju, weil der Testbenutzer Administrator mit `SeBackupPrivilege` ist.

Nicht geprüft: die tatsächliche Ausführung von `schtasks /create`, `/delete` und des Dienstes unter Windows (würde Tasks auf dem Rechner anlegen), macOS komplett.
