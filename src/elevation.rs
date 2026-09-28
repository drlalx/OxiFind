//! Detects and requests administrator rights, which raw NTFS volume access requires.

use std::{ffi::OsString, io};

/// Returns whether the process runs with an elevated (administrator) token.
#[cfg(windows)]
pub fn is_elevated() -> bool {
    use std::mem;

    use windows::Win32::{
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

#[cfg(not(windows))]
pub fn is_elevated() -> bool {
    false
}

/// Starts this executable again with the same arguments through the UAC elevation prompt.
#[cfg(windows)]
pub fn restart_elevated(arguments: &[OsString]) -> io::Result<()> {
    use windows::{
        Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        core::{HSTRING, PCWSTR, w},
    };

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
    // ShellExecuteW reports success with a value greater than 32.
    if instance.0 as usize > 32 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(windows))]
pub fn restart_elevated(_arguments: &[OsString]) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "restarting as administrator requires Windows",
    ))
}

/// Joins arguments into a command line that `CommandLineToArgvW` splits back into the same values.
fn command_line(arguments: &[OsString]) -> String {
    arguments
        .iter()
        .map(|argument| quote_argument(&argument.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_argument(argument: &str) -> String {
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
        // Backslashes are only special directly before a quote, where each must be doubled.
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

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{command_line, quote_argument};

    #[test]
    fn leaves_plain_arguments_unquoted() {
        assert_eq!(quote_argument(r"C:\"), r"C:\");
        assert_eq!(quote_argument(r"D:\data\logs"), r"D:\data\logs");
    }

    #[test]
    fn quotes_arguments_with_spaces_and_doubles_trailing_backslashes() {
        assert_eq!(
            quote_argument(r"C:\Program Files\"),
            r#""C:\Program Files\\""#
        );
        assert_eq!(quote_argument(""), r#""""#);
    }

    #[test]
    fn escapes_quotes_and_the_backslashes_before_them() {
        assert_eq!(quote_argument(r#"a\"b c"#), r#""a\\\"b c""#);
    }

    #[test]
    fn joins_arguments_with_spaces() {
        assert_eq!(
            command_line(&[OsString::from(r"C:\My Files"), OsString::from("x")]),
            r#""C:\My Files" x"#
        );
    }
}
