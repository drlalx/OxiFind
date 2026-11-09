//! Detects and requests administrator/root rights across platforms.

use std::{ffi::OsString, io};

/// Returns whether the process runs with elevated (administrator/root) privileges.
#[cfg(windows)]
pub fn is_elevated() -> bool {
    use std::mem;

    // `::windows` names the crate: a local `mod windows` in this file would shadow it.
    use ::windows::Win32::{
        Foundation::{CloseHandle, HANDLE},
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    let mut token = HANDLE::default();
    // SAFETY: the current-process pseudo handle is always valid; `token` receives a new handle.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.is_err() {
        return false;
    }

    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0_u32;
    let size = u32::try_from(mem::size_of::<TOKEN_ELEVATION>()).unwrap_or(u32::MAX);
    // SAFETY: the buffer is a `TOKEN_ELEVATION` of exactly the size passed in.
    let queried = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some((&raw mut elevation).cast()),
            size,
            &mut returned,
        )
    };
    // SAFETY: `token` was opened above and is not used afterwards.
    let _ = unsafe { CloseHandle(token) };

    queried.is_ok() && elevation.TokenIsElevated != 0
}

#[cfg(unix)]
pub fn is_elevated() -> bool {
    // SAFETY: geteuid has no preconditions and is always valid to query on Unix.
    unsafe { libc::geteuid() == 0 }
}

#[cfg(not(any(windows, unix)))]
pub fn is_elevated() -> bool {
    false
}

/// Starts this executable again with the same arguments through an elevation prompt.
pub fn restart_elevated(arguments: &[OsString]) -> io::Result<()> {
    #[cfg(windows)]
    {
        windows::restart_elevated(arguments)
    }
    #[cfg(target_os = "macos")]
    {
        macos::restart_elevated(arguments)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        linux::restart_elevated(arguments)
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = arguments;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "restarting as administrator is not supported on this platform",
        ))
    }
}

#[cfg(windows)]
mod windows {
    use std::{ffi::OsString, io};
    use windows::{
        Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        core::{HSTRING, PCWSTR, w},
    };

    pub fn restart_elevated(arguments: &[OsString]) -> io::Result<()> {
        let executable = HSTRING::from(std::env::current_exe()?.as_os_str());
        let parameters = HSTRING::from(command_line(arguments));
        // SAFETY: every string is a valid NUL-terminated wide string for the duration of the call.
        let instance = unsafe {
            ShellExecuteW(
                None,
                w!("runas"),
                &executable,
                &parameters,
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        if instance.0 as usize > 32 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    pub fn command_line(arguments: &[OsString]) -> String {
        arguments
            .iter()
            .map(|argument| quote_argument(&argument.to_string_lossy()))
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn quote_argument(argument: &str) -> String {
        if !argument.is_empty() && !argument.contains([' ', '\t', '"']) {
            return argument.to_owned();
        }

        let mut quoted = String::from('"');
        let mut backslashes = 0_usize;
        for character in argument.chars() {
            if character == '\\' {
                backslashes += 1;
                continue;
            }
            let escapes = if character == '"' {
                backslashes * 2 + 1
            } else {
                backslashes
            };
            quoted.push_str(&"\\".repeat(escapes));
            quoted.push(character);
            backslashes = 0;
        }
        quoted.push_str(&"\\".repeat(backslashes * 2));
        quoted.push('"');
        quoted
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod linux {
    use std::{ffi::OsString, io, process::Command};

    pub fn restart_elevated(arguments: &[OsString]) -> io::Result<()> {
        let current_exe = std::env::current_exe()?;
        Command::new("pkexec")
            .arg(current_exe)
            .args(arguments)
            .spawn()
            .map(drop)
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::{ffi::OsString, io, process::Command};

    pub fn restart_elevated(arguments: &[OsString]) -> io::Result<()> {
        let current_exe = std::env::current_exe()?;
        let mut command_line = current_exe.to_string_lossy().into_owned();
        for arg in arguments {
            command_line.push(' ');
            command_line.push_str(&arg.to_string_lossy());
        }
        let script = format!(
            "do shell script \"{}\" with administrator privileges",
            command_line.replace('\\', "\\\\").replace('"', "\\\"")
        );
        Command::new("osascript")
            .args(["-e", &script])
            .spawn()
            .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::is_elevated;

    #[test]
    fn queries_elevation_state_without_panic() {
        // Querying must succeed deterministically on the running OS
        let _ = is_elevated();
    }

    #[test]
    #[cfg(windows)]
    fn quotes_arguments_for_windows_command_line() {
        use super::windows::{command_line, quote_argument};
        use std::ffi::OsString;

        assert_eq!(quote_argument(r"C:\"), r"C:\");
        assert_eq!(
            quote_argument(r"C:\Program Files\"),
            r#""C:\Program Files\\""#
        );
        assert_eq!(
            command_line(&[OsString::from(r"C:\My Files"), OsString::from("x")]),
            r#""C:\My Files" x"#
        );
    }
}
