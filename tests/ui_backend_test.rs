#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use oxifind::backend::{CliConfig, UiBackend};
use oxifind::engine::IndexScope;
use std::ffi::OsString;
use std::path::PathBuf;

#[test]
fn parses_ui_backend_from_args() {
    assert_eq!(UiBackend::parse("tui"), Some(UiBackend::Tui));
    assert_eq!(UiBackend::parse("TUI"), Some(UiBackend::Tui));
    assert_eq!(UiBackend::parse("terminal"), Some(UiBackend::Tui));
    assert_eq!(UiBackend::parse("ratatui"), Some(UiBackend::Tui));

    assert_eq!(UiBackend::parse("egui"), Some(UiBackend::Egui));
    assert_eq!(UiBackend::parse("gui"), Some(UiBackend::Egui));
    assert_eq!(UiBackend::parse("EGUI-UI"), Some(UiBackend::Egui));

    assert_eq!(UiBackend::parse("unknown"), None);
}

#[test]
fn parses_cli_flags_for_tui_and_scope() {
    let args = [OsString::from("--tui"), OsString::from(r"/home/test")];
    let config = CliConfig::parse(args).expect("valid cli flags");

    assert_eq!(config.ui, Some(UiBackend::Tui));
    assert_eq!(config.scope, IndexScope::Root(PathBuf::from("/home/test")));
}

#[test]
fn parses_cli_flags_for_ui_option() {
    let args = [OsString::from("--ui"), OsString::from("tui")];
    let config = CliConfig::parse(args).expect("valid cli flags");

    assert_eq!(config.ui, Some(UiBackend::Tui));
    assert_eq!(config.scope, IndexScope::LocalDrives);
}

#[test]
fn parses_cli_flags_for_gui() {
    let args = [OsString::from("--gui")];
    let config = CliConfig::parse(args).expect("valid cli flags");

    assert_eq!(config.ui, Some(UiBackend::Egui));
    assert_eq!(config.scope, IndexScope::LocalDrives);
}

#[test]
fn rejects_multiple_index_roots_with_flags() {
    let args = [
        OsString::from("--tui"),
        OsString::from("/first"),
        OsString::from("/second"),
    ];
    let err = CliConfig::parse(args).expect_err("should reject multiple roots");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}
