#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

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
fn parses_cli_flags_for_service() {
    use oxifind::backend::ServiceAction;

    let args = [
        OsString::from("--service"),
        OsString::from("--delay"),
        OsString::from("20"),
    ];
    let config = CliConfig::parse(args).expect("valid service flags");
    assert_eq!(
        config.service_action,
        Some(ServiceAction::Run { delay_seconds: 20 })
    );

    let args2 = [
        OsString::from("service"),
        OsString::from("install"),
        OsString::from("--delay=30"),
    ];
    let config2 = CliConfig::parse(args2).expect("valid service subcommand");
    assert_eq!(
        config2.service_action,
        Some(ServiceAction::Install { delay_seconds: 30 })
    );

    let args3 = [OsString::from("service"), OsString::from("status")];
    let config3 = CliConfig::parse(args3).expect("valid service status");
    assert_eq!(config3.service_action, Some(ServiceAction::Status));
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

#[test]
fn parses_cli_flags_for_graphics_renderer() {
    let args = [OsString::from("--gl")];
    let config = CliConfig::parse(args).expect("valid --gl flag");
    assert_eq!(config.renderer.as_deref(), Some("gl"));

    let args_opengl = [OsString::from("--opengl")];
    let config_opengl = CliConfig::parse(args_opengl).expect("valid --opengl flag");
    assert_eq!(config_opengl.renderer.as_deref(), Some("gl"));

    let args_vulkan = [OsString::from("--vulkan")];
    let config_vulkan = CliConfig::parse(args_vulkan).expect("valid --vulkan flag");
    assert_eq!(config_vulkan.renderer.as_deref(), Some("vulkan"));

    let args_custom = [OsString::from("--renderer"), OsString::from("metal")];
    let config_custom = CliConfig::parse(args_custom).expect("valid --renderer flag");
    assert_eq!(config_custom.renderer.as_deref(), Some("metal"));

    let args_custom_eq = [OsString::from("--renderer=dx12")];
    let config_custom_eq = CliConfig::parse(args_custom_eq).expect("valid --renderer= flag");
    assert_eq!(config_custom_eq.renderer.as_deref(), Some("dx12"));
}

#[test]
fn rejects_delay_without_service_action() {
    let args = [OsString::from("--delay"), OsString::from("5")];
    let error = CliConfig::parse(args).expect_err("--delay alone must not be silently ignored");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);

    let args = [
        OsString::from("service"),
        OsString::from("status"),
        OsString::from("--delay=5"),
    ];
    let error = CliConfig::parse(args).expect_err("--delay for status must be rejected");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn accepts_delay_before_or_after_service_flag() {
    use oxifind::backend::ServiceAction;

    let args = [
        OsString::from("--delay"),
        OsString::from("5"),
        OsString::from("--service"),
    ];
    let config = CliConfig::parse(args).expect("delay before --service");
    assert_eq!(
        config.service_action,
        Some(ServiceAction::Run { delay_seconds: 5 })
    );

    let args = [
        OsString::from("--delay=0"),
        OsString::from("service"),
        OsString::from("run"),
    ];
    let config = CliConfig::parse(args).expect("delay before service run");
    assert_eq!(
        config.service_action,
        Some(ServiceAction::Run { delay_seconds: 0 })
    );
}

#[test]
fn rejects_unknown_renderer() {
    let args = [OsString::from("--renderer"), OsString::from("foo")];
    let error = CliConfig::parse(args).expect_err("unknown renderer must be rejected");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}
