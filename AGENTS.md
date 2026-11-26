# AGENTS Memory - OxiFind

## Globale Arbeitsweise

- Antworte immer auf Deutsch, außer Code (dieser soll auf Englisch sein)
- Halte Änderungen minimal und fokussiert (Atomic Commits)
- Bevorzuge funktionale Programmierung gegenüber OOP
- Test-driven Development: Neue Funktionalität zuerst durch Tests beschreiben und dann umsetzen
- Halte die Dokumentation aktuell
- Optimiere Token und Kontext aktiv mit RTK, Headroom und LLMLingua

## Build-, Test- & Compliance-Befehle

- **Build:** `cargo build`
- **Formatierung prüfen:** `cargo fmt --all -- --check`
- **Linting (No-Panic & Safety):** `cargo clippy --all-targets -- -D warnings`
- **Feature-Matrix prüfen:** `cargo check --no-default-features --features tui && cargo check --no-default-features --features egui-ui`
- **Tests ausführen:** `cargo test --all-targets`
- **Lizenzprüfung:** `cargo test --test license_compliance` (oder `cargo deny check licenses` via `deny.toml`)
- **CVE-/Sicherheitsprüfung:** `cargo test --test test_security_advisories` (oder `cargo deny check advisories` via `deny.toml`)

## Gold-Standard-Referenz

Siehe [rust-project-checklist.md](file:///home/adrichel/develop/tips-and-tricks/rust-project-checklist.md).
