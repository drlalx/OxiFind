#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use oxifind::backend::{CliConfig, UiBackend, detect_default_ui};

use std::{env, error::Error};

#[cfg(test)]
use oxifind::engine::IndexScope;
#[cfg(test)]
use std::{ffi::OsString, io};

/// Attaches to the parent console (e.g. CMD/PowerShell) on Windows when running in CLI/TUI mode.
///
/// Because release builds use `windows_subsystem = "windows"` to prevent a flashing console window
/// on double-click, CLI/TUI invocations must explicitly attach to an existing console (or allocate
/// one when `allocate_if_detached` is true, e.g. `--tui` launched from a shortcut).
#[cfg(windows)]
fn setup_console_for_cli(allocate_if_detached: bool) {
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AllocConsole, AttachConsole, GetStdHandle, STD_ERROR_HANDLE,
        STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
    };
    use windows::core::PCWSTR;

    // SAFETY: `AttachConsole` and `AllocConsole` are standard Win32 process-wide console APIs
    // with no pointer preconditions; failures are handled gracefully.
    let has_console = unsafe {
        let attached = AttachConsole(ATTACH_PARENT_PROCESS).is_ok();
        if attached {
            true
        } else if allocate_if_detached {
            AllocConsole().is_ok()
        } else {
            false
        }
    };

    if !has_console {
        return;
    }

    // SAFETY: `GetStdHandle`, `CreateFileW` (with valid NUL-terminated UTF-16 `CONIN$`/`CONOUT$`),
    // and `SetStdHandle` are called with valid arguments to rebind standard handles for `std::io`
    // and `crossterm` after attaching to a console in a `windows` subsystem binary.
    unsafe {
        let std_in = GetStdHandle(STD_INPUT_HANDLE);
        if std_in.is_err() || std_in.as_ref().is_ok_and(|h| h.is_invalid()) {
            let conin: Vec<u16> = "CONIN$\0".encode_utf16().collect();
            if let Ok(handle) = CreateFileW(
                PCWSTR(conin.as_ptr()),
                FILE_GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            ) {
                let _ = SetStdHandle(STD_INPUT_HANDLE, handle);
            }
        }

        let std_out = GetStdHandle(STD_OUTPUT_HANDLE);
        if std_out.is_err() || std_out.as_ref().is_ok_and(|h| h.is_invalid()) {
            let conout: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
            if let Ok(handle) = CreateFileW(
                PCWSTR(conout.as_ptr()),
                FILE_GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            ) {
                let _ = SetStdHandle(STD_OUTPUT_HANDLE, handle);
                let _ = SetStdHandle(STD_ERROR_HANDLE, handle);
            }
        }
    }
}

#[cfg(not(windows))]
fn setup_console_for_cli(_allocate_if_detached: bool) {}

fn main() {
    let config = match CliConfig::parse(env::args_os().skip(1)) {
        Ok(cfg) => cfg,
        Err(error) => {
            setup_console_for_cli(false);
            eprintln!("Application failed: {error}");
            std::process::exit(1);
        }
    };

    if config.help_requested {
        setup_console_for_cli(false);
        print_usage();
        return;
    }

    if let Some(action) = config.service_action {
        // Attach to parent console if invoked from CMD/PowerShell, but never pop up
        // a new console window when launched headless (e.g. via Task Scheduler).
        setup_console_for_cli(false);
        match action {
            oxifind::backend::ServiceAction::Run { delay_seconds } => {
                if let Err(error) = oxifind::engine::run_service(config.scope, delay_seconds) {
                    eprintln!("Service error: {error}");
                    std::process::exit(1);
                }
            }
            oxifind::backend::ServiceAction::Install { delay_seconds } => {
                let scope_root = match &config.scope {
                    oxifind::engine::IndexScope::Root(root) => Some(root.as_path()),
                    oxifind::engine::IndexScope::LocalDrives => None,
                };
                match oxifind::platform::service::install_service_for_scope(
                    delay_seconds,
                    scope_root,
                ) {
                    Ok(()) => println!(
                        "OxiFind background service successfully installed and enabled (startup delay: {delay_seconds}s)."
                    ),
                    Err(error) => {
                        eprintln!("Failed to install service: {error}");
                        std::process::exit(1);
                    }
                }
            }
            oxifind::backend::ServiceAction::Uninstall => {
                match oxifind::platform::service::uninstall_service() {
                    Ok(()) => println!(
                        "OxiFind background service successfully uninstalled and disabled."
                    ),
                    Err(error) => {
                        eprintln!("Failed to uninstall service: {error}");
                        std::process::exit(1);
                    }
                }
            }
            oxifind::backend::ServiceAction::Status => {
                let status = oxifind::platform::service::service_status();
                println!("OxiFind Service Status: {}", status.label());
            }
            oxifind::backend::ServiceAction::Start => {
                match oxifind::platform::service::start_service() {
                    Ok(()) => println!("OxiFind background service started."),
                    Err(error) => {
                        eprintln!("Failed to start service: {error}");
                        std::process::exit(1);
                    }
                }
            }
            oxifind::backend::ServiceAction::Stop => {
                match oxifind::platform::service::stop_service() {
                    Ok(()) => println!("OxiFind background service stopped."),
                    Err(error) => {
                        eprintln!("Failed to stop service: {error}");
                        std::process::exit(1);
                    }
                }
            }
        }
        return;
    }

    if let Some(ref renderer) = config.renderer {
        // SAFETY: Called early in process startup before multi-threaded worker pools or GUI loops start.
        unsafe {
            std::env::set_var("WGPU_BACKEND", renderer);
        }
    }

    let ui = config.ui.unwrap_or_else(detect_default_ui);

    let result: Result<(), Box<dyn Error + Send + Sync>> = match ui {
        #[cfg(feature = "egui-ui")]
        UiBackend::Egui => oxifind::app::run(config.scope),
        #[cfg(not(feature = "egui-ui"))]
        UiBackend::Egui => {
            setup_console_for_cli(false);
            eprintln!("Error: The 'egui' GUI backend was not compiled in this binary.");
            std::process::exit(1);
        }
        #[cfg(feature = "tui")]
        UiBackend::Tui => {
            setup_console_for_cli(true);
            oxifind::tui::run(config.scope)
        }
        #[cfg(not(feature = "tui"))]
        UiBackend::Tui => {
            setup_console_for_cli(false);
            eprintln!("Error: The 'tui' Terminal backend was not compiled in this binary.");
            std::process::exit(1);
        }
    };

    if let Err(error) = result {
        setup_console_for_cli(false);
        eprintln!("Application failed: {error}");
        std::process::exit(1);
    }
}

fn print_usage() {
    println!(
        "OxiFind - Fast multiplatform file search engine\n\n\
         Usage: oxifind [OPTIONS] [INDEX_ROOT]\n\
                oxifind service <install|uninstall|status|start|stop|run> [--delay <secs>]\n\n\
         Options:\n  \
           --tui                    Launch Terminal UI (Ratatui)\n  \
           --gui                    Launch Desktop GUI (egui)\n  \
           --ui <egui|tui>          Select active UI backend\n  \
           --gl, --opengl           Force OpenGL graphics backend (recommended for Linux Wayland)\n  \
           --vulkan                 Force Vulkan graphics backend\n  \
           --renderer <backend>     Set custom graphics backend (gl, vulkan)\n  \
           --service, --daemon      Run headless background indexer & watcher\n  \
           --delay <seconds>        Startup delay in seconds (default: 15 for service)\n  \
           --help, -h               Show this help message\n\n\
         Commands:\n  \
           service install          Install as background service (systemd / schtasks / launchd)\n  \
           service uninstall        Remove background service\n  \
           service status           Show current service state\n  \
           service start / stop     Start or stop background service\n\n\
         Arguments:\n  \
           [INDEX_ROOT]             Path or drive root to index (defaults to all local drives/filesystems)"
    );
}

#[cfg(test)]
/// Without an argument every local drive is indexed; otherwise the given directory or drive.
fn index_scope_from_args(arguments: impl IntoIterator<Item = OsString>) -> io::Result<IndexScope> {
    CliConfig::parse(arguments).map(|config| config.scope)
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::PathBuf};

    use super::{IndexScope, index_scope_from_args};

    #[test]
    fn indexes_all_local_drives_without_an_argument() {
        let scope = index_scope_from_args([]).unwrap();

        assert_eq!(scope, IndexScope::LocalDrives);
    }

    #[test]
    fn uses_the_single_argument_as_the_index_root() {
        let scope = index_scope_from_args([OsString::from(r"C:\search-root")]).unwrap();

        assert_eq!(scope, IndexScope::Root(PathBuf::from(r"C:\search-root")));
    }

    #[test]
    fn rejects_more_than_one_index_root() {
        let error = index_scope_from_args([
            OsString::from(r"C:\first-root"),
            OsString::from(r"D:\second-root"),
        ])
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }
}
