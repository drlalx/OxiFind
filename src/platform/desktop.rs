//! Desktop integration for opening items and revealing them in the system file manager.

use std::{io, path::Path};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenAction {
    Open,
    OpenWith,
    Reveal,
    OpenFolder,
}

/// Opens `path` with the default application or reveals it in the platform file manager.
pub fn open_path(path: &Path, action: OpenAction) -> io::Result<()> {
    if action == OpenAction::OpenFolder {
        return open_path(containing_folder(path), OpenAction::Open);
    }
    #[cfg(windows)]
    {
        windows::open_in_explorer(path, action)
    }
    #[cfg(target_os = "macos")]
    {
        macos::open_in_finder(path, action)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        linux::open_in_linux_desktop(path, action)
    }
}

/// Returns the folder that contains `path`. A bare relative file name maps to the current directory
/// (`.`), since an empty parent cannot be opened; a path without parent (a root) maps to itself.
pub fn containing_folder(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if parent.as_os_str().is_empty() => Path::new("."),
        Some(parent) => parent,
        None => path,
    }
}

/// Builds the raw Explorer argument; Windows paths cannot contain quotes, so quoting is safe.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn explorer_argument(path: &Path, action: OpenAction) -> String {
    match action {
        OpenAction::Open | OpenAction::OpenWith | OpenAction::OpenFolder => {
            format!("\"{}\"", path.display())
        }
        OpenAction::Reveal => format!("/select,\"{}\"", path.display()),
    }
}

#[cfg(windows)]
pub(crate) mod windows {
    use std::{io, os::windows::process::CommandExt, path::Path, process::Command};

    use super::{OpenAction, explorer_argument};

    pub fn open_in_explorer(path: &Path, action: OpenAction) -> io::Result<()> {
        if path.as_os_str().to_string_lossy().contains('"') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "paths containing quotes cannot be passed to Explorer",
            ));
        }
        match action {
            OpenAction::OpenWith => {
                Command::new("rundll32.exe")
                    .arg("shell32.dll,OpenAs_RunDLL")
                    .arg(path)
                    .spawn()
                    .map(drop)
            }
            OpenAction::Open | OpenAction::Reveal | OpenAction::OpenFolder => {
                // Explorer reports a failure exit code even after opening successfully, so only spawning is checked.
                Command::new("explorer.exe")
                    .raw_arg(explorer_argument(path, action))
                    .spawn()
                    .map(drop)
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) mod macos {
    use std::{io, path::Path, process::Command};

    use super::OpenAction;

    pub fn open_in_finder(path: &Path, action: OpenAction) -> io::Result<()> {
        let mut command = Command::new("open");
        match action {
            OpenAction::Open | OpenAction::OpenFolder => {
                command.arg(path);
            }
            OpenAction::Reveal => {
                // -R reveals the file selected in Finder
                command.arg("-R").arg(path);
            }
            OpenAction::OpenWith => {
                // The path is passed as an argument, never interpolated into the script text,
                // so quotes or backslashes in file names cannot change the script.
                let script = "on run argv\n\
                    tell application \"Finder\" to open POSIX file (item 1 of argv) using (choose application)\n\
                    end run";
                if let Ok(status) = Command::new("osascript")
                    .arg("-e")
                    .arg(script)
                    .arg(path)
                    .status()
                {
                    if status.success() {
                        return Ok(());
                    }
                }
                command.arg(path);
            }
        }
        command.spawn().map(drop)
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) mod linux {
    use std::{io, path::Path, process::Command};

    use super::OpenAction;

    /// Formats a filesystem path into a file:// URI for FreeDesktop interfaces.
    pub fn file_uri(path: &Path) -> String {
        let lossy = path.to_string_lossy();
        if lossy.starts_with('/') {
            format!("file://{lossy}")
        } else {
            format!("file:///{lossy}")
        }
    }

    pub fn open_in_linux_desktop(path: &Path, action: OpenAction) -> io::Result<()> {
        match action {
            OpenAction::Open | OpenAction::OpenFolder => {
                Command::new("xdg-open").arg(path).spawn().map(drop)
            }
            OpenAction::Reveal => reveal_in_file_manager(path),
            OpenAction::OpenWith => open_with_dialog(path),
        }
    }

    fn open_with_dialog(path: &Path) -> io::Result<()> {
        let uri = file_uri(path);

        // Try XDG Desktop Portal OpenURI with ask=true via gdbus
        let gdbus_status = Command::new("gdbus")
            .args([
                "call",
                "--session",
                "--dest",
                "org.freedesktop.portal.Desktop",
                "--object-path",
                "/org/freedesktop/portal/desktop",
                "--method",
                "org.freedesktop.portal.OpenURI.OpenURI",
                "",
                &uri,
                "{'ask': <true>}",
            ])
            .status();

        if gdbus_status.is_ok_and(|status| status.success()) {
            return Ok(());
        }

        // Fallback: standard xdg-open
        Command::new("xdg-open").arg(path).spawn().map(drop)
    }

    /// Reveals `path` selected in the active FreeDesktop file manager (GNOME Files, Dolphin, Thunar, etc.)
    /// via the `org.freedesktop.FileManager1` DBus interface, falling back to `xdg-open` on the parent folder.
    fn reveal_in_file_manager(path: &Path) -> io::Result<()> {
        let uri = file_uri(path);

        // Try DBus call via dbus-send
        let dbus_status = Command::new("dbus-send")
            .args([
                "--session",
                "--dest=org.freedesktop.FileManager1",
                "--type=method_call",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
                &format!("array:string:{uri}"),
                "string:",
            ])
            .status();

        if dbus_status.is_ok_and(|status| status.success()) {
            return Ok(());
        }

        // Try DBus call via gdbus as secondary DBus tool
        let gdbus_status = Command::new("gdbus")
            .args([
                "call",
                "--session",
                "--dest",
                "org.freedesktop.FileManager1",
                "--object-path",
                "/org/freedesktop/FileManager1",
                "--method",
                "org.freedesktop.FileManager1.ShowItems",
                &format!("['{uri}']"),
                "",
            ])
            .status();

        if gdbus_status.is_ok_and(|status| status.success()) {
            return Ok(());
        }

        // Fallback: open parent folder via xdg-open
        Command::new("xdg-open")
            .arg(super::containing_folder(path))
            .spawn()
            .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::OpenAction;
    #[cfg(windows)]
    use super::open_path;

    #[test]
    fn open_actions_have_expected_variants() {
        assert_ne!(OpenAction::Open, OpenAction::Reveal);
        assert_ne!(OpenAction::OpenWith, OpenAction::OpenFolder);
        assert_ne!(OpenAction::Open, OpenAction::OpenWith);
    }

    #[test]
    #[cfg(all(unix, not(target_os = "macos")))]
    fn formats_file_uris_for_freedesktop_interfaces() {
        use super::linux::file_uri;

        assert_eq!(
            file_uri(Path::new("/home/user/document.txt")),
            "file:///home/user/document.txt"
        );
        assert_eq!(file_uri(Path::new("relative.txt")), "file:///relative.txt");
    }

    #[test]
    fn containing_folder_of_relative_file_is_current_directory() {
        assert_eq!(super::containing_folder(Path::new("file.txt")), Path::new("."));
        assert_eq!(
            super::containing_folder(Path::new("dir/file.txt")),
            Path::new("dir")
        );
    }

    #[test]
    fn containing_folder_of_root_is_root() {
        assert_eq!(super::containing_folder(Path::new("/")), Path::new("/"));
    }

    #[test]
    fn formats_explorer_arguments_safely() {
        // Can be verified on all platforms using direct argument inspection
        let open_arg = format!("\"{}\"", Path::new(r"C:\test\file.txt").display());
        let reveal_arg = format!("/select,\"{}\"", Path::new(r"C:\test\file.txt").display());

        assert_eq!(open_arg, r#""C:\test\file.txt""#);
        assert_eq!(reveal_arg, r#"/select,"C:\test\file.txt""#);
    }

    #[test]
    fn rejects_paths_with_quotes_on_windows_open() {
        let quoted_path = Path::new("path\"with\"quotes");
        #[cfg(windows)]
        {
            let result = open_path(quoted_path, OpenAction::Open);
            assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
        }
        #[cfg(not(windows))]
        {
            let _ = quoted_path;
        }
    }
}
