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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliConfig {
    pub ui: Option<UiBackend>,
    pub scope: IndexScope,
    pub help_requested: bool,
}

impl CliConfig {
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> io::Result<Self> {
        let mut iter = args.into_iter();
        let mut ui = None;
        let mut scope_path = None;
        let mut help_requested = false;

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
                    "Usage: oxifind [--tui|--gui|--ui <backend>] [INDEX_ROOT]",
                ));
            }

            scope_path = Some(PathBuf::from(arg));
        }

        let scope = scope_path.map_or(IndexScope::LocalDrives, IndexScope::Root);

        Ok(Self {
            ui,
            scope,
            help_requested,
        })
    }
}
