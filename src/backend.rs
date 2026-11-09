use std::{ffi::OsString, io, path::PathBuf};

use crate::engine::IndexScope;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiBackend {
    Egui,
    Tui,
}

impl UiBackend {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_lowercase().as_str() {
            "egui" | "egui-ui" | "gui" => Some(Self::Egui),
            "tui" | "tui-ui" | "terminal" | "ratatui" => Some(Self::Tui),
            _ => None,
        }
    }

    pub fn is_compiled(self) -> bool {
        match self {
            Self::Egui => cfg!(feature = "egui-ui"),
            Self::Tui => cfg!(feature = "tui"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Egui => "egui",
            Self::Tui => "tui",
        }
    }
}

pub fn detect_default_ui() -> UiBackend {
    #[cfg(all(unix, feature = "tui"))]
    {
        if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return UiBackend::Tui;
        }
    }

    #[cfg(feature = "egui-ui")]
    {
        UiBackend::Egui
    }

    #[cfg(all(not(feature = "egui-ui"), feature = "tui"))]
    {
        UiBackend::Tui
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceAction {
    Run { delay_seconds: u32 },
    Install { delay_seconds: u32 },
    Uninstall,
    Status,
    Start,
    Stop,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliConfig {
    pub ui: Option<UiBackend>,
    pub scope: IndexScope,
    pub help_requested: bool,
    pub service_action: Option<ServiceAction>,
    pub renderer: Option<String>,
}

const SUPPORTED_RENDERERS: [&str; 4] = ["gl", "vulkan", "metal", "dx12"];

fn parse_delay(value: &str) -> io::Result<u32> {
    value
        .parse::<u32>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid number for --delay"))
}

fn parse_renderer(value: &str) -> io::Result<String> {
    if SUPPORTED_RENDERERS.contains(&value) {
        Ok(value.to_owned())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "Unknown renderer: {value}. Valid values: {}",
                SUPPORTED_RENDERERS.join(", ")
            ),
        ))
    }
}

/// Applies a startup delay to the service actions that take one; other actions are returned unchanged.
fn with_delay(action: Option<ServiceAction>, delay_seconds: u32) -> Option<ServiceAction> {
    match action {
        Some(ServiceAction::Run { .. }) => Some(ServiceAction::Run { delay_seconds }),
        Some(ServiceAction::Install { .. }) => Some(ServiceAction::Install { delay_seconds }),
        other => other,
    }
}

impl CliConfig {
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> io::Result<Self> {
        let mut iter = args.into_iter();
        let mut ui = None;
        let mut scope_path = None;
        let mut help_requested = false;
        let mut service_action = None;
        let mut delay_seconds = 15;
        let mut delay_given = false;
        let mut renderer = None;

        while let Some(arg) = iter.next() {
            let arg_str = arg.to_string_lossy();
            if arg_str == "--help" || arg_str == "-h" {
                help_requested = true;
                continue;
            }
            if arg_str == "--tui" {
                ui = Some(UiBackend::Tui);
                continue;
            }
            if arg_str == "--gui" {
                ui = Some(UiBackend::Egui);
                continue;
            }
            if arg_str == "--gl" || arg_str == "--opengl" {
                renderer = Some("gl".to_owned());
                continue;
            }
            if arg_str == "--vulkan" {
                renderer = Some("vulkan".to_owned());
                continue;
            }
            if arg_str == "--renderer" {
                let val = iter.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "Missing value for --renderer")
                })?;
                renderer = Some(parse_renderer(&val.to_string_lossy())?);
                continue;
            }
            if let Some(val_str) = arg_str.strip_prefix("--renderer=") {
                renderer = Some(parse_renderer(val_str)?);
                continue;
            }
            if arg_str == "--service" || arg_str == "--daemon" {
                service_action = Some(ServiceAction::Run { delay_seconds });
                continue;
            }
            if arg_str == "--delay" {
                let val = iter.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "Missing value for --delay")
                })?;
                delay_seconds = parse_delay(&val.to_string_lossy())?;
                delay_given = true;
                service_action = with_delay(service_action, delay_seconds);
                continue;
            }
            if let Some(val_str) = arg_str.strip_prefix("--delay=") {
                delay_seconds = parse_delay(val_str)?;
                delay_given = true;
                service_action = with_delay(service_action, delay_seconds);
                continue;
            }
            if arg_str == "service" {
                let sub = iter.next().ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Missing service subcommand: install, uninstall, status, start, stop, or run",
                    )
                })?;
                let sub_str = sub.to_string_lossy();
                service_action = match sub_str.as_ref() {
                    "install" => Some(ServiceAction::Install { delay_seconds }),
                    "uninstall" => Some(ServiceAction::Uninstall),
                    "status" => Some(ServiceAction::Status),
                    "start" => Some(ServiceAction::Start),
                    "stop" => Some(ServiceAction::Stop),
                    "run" => Some(ServiceAction::Run { delay_seconds }),
                    other => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!("Unknown service subcommand: {other}"),
                        ));
                    }
                };
                continue;
            }
            if arg_str == "--ui" || arg_str == "--feature" {
                let val = iter.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "Missing value for --ui")
                })?;
                let val_str = val.to_string_lossy();
                ui = Some(UiBackend::parse(&val_str).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("Unknown UI backend: {val_str}. Valid values: egui, tui"),
                    )
                })?);
                continue;
            }
            if let Some(val_str) = arg_str.strip_prefix("--ui=") {
                ui = Some(UiBackend::parse(val_str).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("Unknown UI backend: {val_str}. Valid values: egui, tui"),
                    )
                })?);
                continue;
            }

            // If it's another option, reject or treat as positional
            if arg_str.starts_with('-') {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("Unknown option: {arg_str}"),
                ));
            }

            if scope_path.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Usage: oxifind [--tui|--gui|--ui <backend>] [--service] [INDEX_ROOT]",
                ));
            }

            scope_path = Some(PathBuf::from(arg));
        }

        if delay_given && !matches!(service_action, Some(ServiceAction::Run { .. } | ServiceAction::Install { .. })) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "--delay requires --service, 'service run' or 'service install'",
            ));
        }

        let scope = scope_path.map_or(IndexScope::LocalDrives, IndexScope::Root);

        Ok(Self {
            ui,
            scope,
            help_requested,
            service_action,
            renderer,
        })
    }
}
