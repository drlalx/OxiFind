#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use oxifind::backend::{CliConfig, UiBackend, detect_default_ui};

use std::{env, error::Error};

#[cfg(test)]
use oxifind::engine::IndexScope;
#[cfg(test)]
use std::{ffi::OsString, io};

fn main() {
    let config = match CliConfig::parse(env::args_os().skip(1)) {
        Ok(cfg) => cfg,
        Err(error) => {
            eprintln!("Application failed: {error}");
            std::process::exit(1);
        }
    };

    if config.help_requested {
        print_usage();
        return;
    }

    let ui = config.ui.unwrap_or_else(detect_default_ui);

    let result: Result<(), Box<dyn Error + Send + Sync>> = match ui {
        #[cfg(feature = "egui-ui")]
        UiBackend::Egui => oxifind::app::run(config.scope),
        #[cfg(not(feature = "egui-ui"))]
        UiBackend::Egui => {
            eprintln!("Error: The 'egui' GUI backend was not compiled in this binary.");
            std::process::exit(1);
        }
        #[cfg(feature = "tui")]
        UiBackend::Tui => oxifind::tui::run(config.scope),
        #[cfg(not(feature = "tui"))]
        UiBackend::Tui => {
            eprintln!("Error: The 'tui' Terminal backend was not compiled in this binary.");
            std::process::exit(1);
        }
    };

    if let Err(error) = result {
        eprintln!("Application failed: {error}");
        std::process::exit(1);
    }
}

fn print_usage() {
    println!(
        "OxiFind - Fast multiplatform file search engine\n\n\
         Usage: oxifind [OPTIONS] [INDEX_ROOT]\n\n\
         Options:\n  \
           --tui                    Launch Terminal UI (Ratatui)\n  \
           --gui                    Launch Desktop GUI (egui)\n  \
           --ui <egui|tui>          Select active UI backend\n  \
           --help, -h               Show this help message\n\n\
         Arguments:\n  \
           [INDEX_ROOT]             Path or drive root to index (defaults to all local drives)"
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
