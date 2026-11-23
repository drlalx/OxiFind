//! Background service and daemon management across Linux (systemd), Windows (Task Scheduler), and macOS (launchd).

use std::{io, path::Path};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceStatus {
    Active,
    Inactive,
    NotInstalled,
}

impl ServiceStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Active => "Aktiv (läuft im Hintergrund)",
            Self::Inactive => "Installiert (inaktiv/gestoppt)",
            Self::NotInstalled => "Nicht installiert",
        }
    }
}

/// Checks whether the background indexer service is installed and running.
pub fn service_status() -> ServiceStatus {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        linux::service_status()
    }
    #[cfg(windows)]
    {
        windows::service_status()
    }
    #[cfg(target_os = "macos")]
    {
        macos::service_status()
    }
}

/// Installs the background indexer service with a startup delay in seconds.
pub fn install_service(delay_seconds: u32) -> io::Result<()> {
    install_service_for_scope(delay_seconds, None)
}

/// Installs the background indexer service with an optional explicit root scope.
pub fn install_service_for_scope(delay_seconds: u32, root: Option<&Path>) -> io::Result<()> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        linux::install_service(delay_seconds, root)
    }
    #[cfg(windows)]
    {
        windows::install_service(delay_seconds, root)
    }
    #[cfg(target_os = "macos")]
    {
        macos::install_service(delay_seconds, root)
    }
}

/// Uninstalls and stops the background indexer service.
pub fn uninstall_service() -> io::Result<()> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        linux::uninstall_service()
    }
    #[cfg(windows)]
    {
        windows::uninstall_service()
    }
    #[cfg(target_os = "macos")]
    {
        macos::uninstall_service()
    }
}

/// Starts the installed service.
pub fn start_service() -> io::Result<()> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        linux::start_service()
    }
    #[cfg(windows)]
    {
        windows::start_service()
    }
    #[cfg(target_os = "macos")]
    {
        macos::start_service()
    }
}

/// Stops the running service.
pub fn stop_service() -> io::Result<()> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        linux::stop_service()
    }
    #[cfg(windows)]
    {
        windows::stop_service()
    }
    #[cfg(target_os = "macos")]
    {
        macos::stop_service()
    }
}

#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
fn format_systemd_arg(arg: &str) -> String {
    if arg.contains([' ', '\t', '"', '\\']) {
        let escaped = arg.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    } else {
        arg.to_owned()
    }
}

/// Builds the systemd user unit. The delay is passed once via `--delay`, so the service's own
/// startup delay is the only one applied (an `ExecStartPre` sleep would add a second delay).
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
pub fn systemd_unit_content(exe_path: &str, delay_seconds: u32) -> String {
    systemd_unit_content_with_root(exe_path, delay_seconds, None)
}

/// Builds the systemd user unit with an optional explicit `INDEX_ROOT` argument.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
pub fn systemd_unit_content_with_root(
    exe_path: &str,
    delay_seconds: u32,
    root: Option<&Path>,
) -> String {
    let exe_arg = format_systemd_arg(exe_path);
    let root_suffix = root.map_or_else(String::new, |path| {
        format!(" {}", format_systemd_arg(&path.display().to_string()))
    });
    format!(
        r#"[Unit]
Description=OxiFind Background Search Indexer
Documentation=https://github.com/oxifind/oxifind
After=default.target

[Service]
Type=simple
ExecStart={exe_arg} --service --delay {delay_seconds}{root_suffix}
Restart=on-failure
RestartSec=10
Nice=19
IOSchedulingClass=idle

[Install]
WantedBy=default.target
"#
    )
}

/// Formats the delay for `schtasks /delay`, which expects `mmmm:ss` (seconds must stay below 60).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn schtasks_delay_value(delay_seconds: u32) -> String {
    format!("{:04}:{:02}", delay_seconds / 60, delay_seconds % 60)
}

#[cfg(all(unix, not(target_os = "macos")))]
pub mod linux {
    use std::{
        fs, io,
        path::{Path, PathBuf},
        process::Command,
    };
    use super::{ServiceStatus, systemd_unit_content_with_root};

    pub fn user_unit_dir() -> io::Result<PathBuf> {
        if let Some(config_dir) = std::env::var_os("XDG_CONFIG_HOME") {
            Ok(PathBuf::from(config_dir).join("systemd/user"))
        } else if let Some(home) = std::env::var_os("HOME") {
            Ok(PathBuf::from(home).join(".config/systemd/user"))
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "HOME not found"))
        }
    }

    pub fn unit_file_path() -> io::Result<PathBuf> {
        Ok(user_unit_dir()?.join("oxifind.service"))
    }

    pub fn service_status() -> ServiceStatus {
        let Ok(path) = unit_file_path() else {
            return ServiceStatus::NotInstalled;
        };
        if !path.exists() {
            return ServiceStatus::NotInstalled;
        }

        let output = Command::new("systemctl")
            .args(["--user", "is-active", "oxifind.service"])
            .output();

        match output {
            Ok(out) => {
                let status_str = String::from_utf8_lossy(&out.stdout).trim().to_lowercase();
                if status_str == "active" {
                    ServiceStatus::Active
                } else {
                    ServiceStatus::Inactive
                }
            }
            Err(_) => ServiceStatus::Inactive,
        }
    }

    pub fn install_service(delay_seconds: u32, root: Option<&Path>) -> io::Result<()> {
        let unit_dir = user_unit_dir()?;
        fs::create_dir_all(&unit_dir)?;
        let unit_path = unit_dir.join("oxifind.service");

        let exe_path = std::env::current_exe()
            .unwrap_or_else(|_| PathBuf::from("oxifind"));
        let exe_str = exe_path.display().to_string();

        fs::write(
            &unit_path,
            systemd_unit_content_with_root(&exe_str, delay_seconds, root),
        )?;

        // Reload systemd & enable service
        let _ = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status();

        let status = Command::new("systemctl")
            .args(["--user", "enable", "--now", "oxifind.service"])
            .status()?;

        if !status.success() {
            return Err(io::Error::other(format!(
                "systemctl enable --now failed with exit code {:?}",
                status.code()
            )));
        }

        Ok(())
    }

    pub fn uninstall_service() -> io::Result<()> {
        let _ = Command::new("systemctl")
            .args(["--user", "stop", "oxifind.service"])
            .status();

        let _ = Command::new("systemctl")
            .args(["--user", "disable", "oxifind.service"])
            .status();

        if let Ok(path) = unit_file_path()
            && path.exists()
        {
            let _ = fs::remove_file(path);
        }

        let _ = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status();

        Ok(())
    }

    pub fn start_service() -> io::Result<()> {
        Command::new("systemctl")
            .args(["--user", "start", "oxifind.service"])
            .status()
            .and_then(|s| {
                if s.success() {
                    Ok(())
                } else {
                    Err(io::Error::other("Failed to start service"))
                }
            })
    }

    pub fn stop_service() -> io::Result<()> {
        Command::new("systemctl")
            .args(["--user", "stop", "oxifind.service"])
            .status()
            .and_then(|s| {
                if s.success() {
                    Ok(())
                } else {
                    Err(io::Error::other("Failed to stop service"))
                }
            })
    }
}

#[cfg(windows)]
pub mod windows {
    use std::{io, path::Path, process::Command};
    use super::{ServiceStatus, schtasks_delay_value};

    pub fn service_status() -> ServiceStatus {
        let output = Command::new("schtasks.exe")
            .args(["/query", "/tn", "OxiFind-Service"])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_lowercase();
                if stdout.contains("running") || stdout.contains("wird ausgeführt") {
                    ServiceStatus::Active
                } else {
                    ServiceStatus::Inactive
                }
            }
            _ => ServiceStatus::NotInstalled,
        }
    }

    pub fn install_service(delay_seconds: u32, root: Option<&Path>) -> io::Result<()> {
        let exe_path = std::env::current_exe()?;
        let exe_str = match root {
            Some(path) => format!("\"{}\" --service \"{}\"", exe_path.display(), path.display()),
            None => format!("\"{}\" --service", exe_path.display()),
        };

        let delay_str = schtasks_delay_value(delay_seconds);

        let status = Command::new("schtasks.exe")
            .args([
                "/create",
                "/tn",
                "OxiFind-Service",
                "/tr",
                &exe_str,
                "/sc",
                "onlogon",
                "/delay",
                &delay_str,
                "/f",
            ])
            .status()?;

        if !status.success() {
            return Err(io::Error::other(format!(
                "schtasks /create failed with code {:?}",
                status.code()
            )));
        }

        let _ = start_service();
        Ok(())
    }

    pub fn uninstall_service() -> io::Result<()> {
        let _ = stop_service();
        Command::new("schtasks.exe")
            .args(["/delete", "/tn", "OxiFind-Service", "/f"])
            .status()
            .and_then(|s| {
                if s.success() {
                    Ok(())
                } else {
                    Err(io::Error::other("Failed to delete scheduled task"))
                }
            })
    }

    pub fn start_service() -> io::Result<()> {
        Command::new("schtasks.exe")
            .args(["/run", "/tn", "OxiFind-Service"])
            .status()
            .and_then(|s| {
                if s.success() {
                    Ok(())
                } else {
                    Err(io::Error::other("Failed to start scheduled task"))
                }
            })
    }

    pub fn stop_service() -> io::Result<()> {
        Command::new("schtasks.exe")
            .args(["/end", "/tn", "OxiFind-Service"])
            .status()
            .and_then(|s| {
                if s.success() {
                    Ok(())
                } else {
                    Err(io::Error::other("Failed to stop scheduled task"))
                }
            })
    }
}

#[cfg(target_os = "macos")]
pub mod macos {
    use std::{
        fs, io,
        path::{Path, PathBuf},
        process::Command,
    };
    use super::ServiceStatus;

    pub fn plist_path() -> io::Result<PathBuf> {
        let home = std::env::var_os("HOME")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME not found"))?;
        Ok(PathBuf::from(home).join("Library/LaunchAgents/io.github.oxifind.plist"))
    }

    pub fn service_status() -> ServiceStatus {
        let Ok(path) = plist_path() else {
            return ServiceStatus::NotInstalled;
        };
        if !path.exists() {
            return ServiceStatus::NotInstalled;
        }

        let output = Command::new("launchctl")
            .args(["list", "io.github.oxifind"])
            .output();

        match output {
            Ok(out) if out.status.success() => ServiceStatus::Active,
            _ => ServiceStatus::Inactive,
        }
    }

    pub fn install_service(delay_seconds: u32, root: Option<&Path>) -> io::Result<()> {
        let path = plist_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let exe_path = std::env::current_exe()?;
        let root_arg = root.map_or_else(String::new, |p| {
            format!("\n        <string>{}</string>", p.display())
        });

        let plist_content = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>io.github.oxifind</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>--service</string>
        <string>--delay</string>
        <string>{}</string>{}
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>LowPriorityIO</key>
    <true/>
    <key>ProcessType</key>
    <string>Background</string>
</dict>
</plist>
"#,
            exe_path.display(),
            delay_seconds,
            root_arg
        );

        fs::write(&path, plist_content)?;

        let status = Command::new("launchctl")
            .args(["load", "-w", &path.display().to_string()])
            .status()?;

        if !status.success() {
            return Err(io::Error::other(format!(
                "launchctl load failed with code {:?}",
                status.code()
            )));
        }
        Ok(())
    }

    pub fn uninstall_service() -> io::Result<()> {
        let path = plist_path()?;
        if path.exists() {
            let _ = Command::new("launchctl")
                .args(["unload", "-w", &path.display().to_string()])
                .status();
            let _ = fs::remove_file(path);
        }
        Ok(())
    }

    pub fn start_service() -> io::Result<()> {
        Command::new("launchctl")
            .args(["start", "io.github.oxifind"])
            .status()
            .and_then(|s| {
                if s.success() {
                    Ok(())
                } else {
                    Err(io::Error::other("Failed to start launchd job"))
                }
            })
    }

    pub fn stop_service() -> io::Result<()> {
        Command::new("launchctl")
            .args(["stop", "io.github.oxifind"])
            .status()
            .and_then(|s| {
                if s.success() {
                    Ok(())
                } else {
                    Err(io::Error::other("Failed to stop launchd job"))
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{ServiceStatus, schtasks_delay_value, systemd_unit_content};

    #[test]
    fn service_status_labels_are_defined() {
        assert_eq!(ServiceStatus::Active.label(), "Aktiv (läuft im Hintergrund)");
        assert_eq!(ServiceStatus::Inactive.label(), "Installiert (inaktiv/gestoppt)");
        assert_eq!(ServiceStatus::NotInstalled.label(), "Nicht installiert");
    }

    #[test]
    fn schtasks_delay_uses_minutes_and_seconds() {
        assert_eq!(schtasks_delay_value(0), "0000:00");
        assert_eq!(schtasks_delay_value(15), "0000:15");
        assert_eq!(schtasks_delay_value(90), "0001:30");
        assert_eq!(schtasks_delay_value(3600), "0060:00");
    }

    #[test]
    fn systemd_unit_passes_delay_to_service_once() {
        let unit = systemd_unit_content("/usr/bin/oxifind", 0);
        assert!(unit.contains("ExecStart=/usr/bin/oxifind --service --delay 0\n"));
        assert!(!unit.contains("ExecStartPre"));

        let unit = systemd_unit_content("/usr/bin/oxifind", 60);
        assert!(unit.contains("ExecStart=/usr/bin/oxifind --service --delay 60\n"));
        assert!(!unit.contains("ExecStartPre"));
    }

    #[test]
    fn systemd_unit_with_root_appends_and_quotes_scope_root() {
        use std::path::Path;

        let unit = super::systemd_unit_content_with_root(
            "/usr/bin/oxifind",
            15,
            Some(Path::new("/home/user/My Docs")),
        );
        assert!(unit.contains(
            "ExecStart=/usr/bin/oxifind --service --delay 15 \"/home/user/My Docs\"\n"
        ));

        let plain = super::systemd_unit_content_with_root(
            "/usr/bin/oxifind",
            15,
            Some(Path::new("/home/user/Sync")),
        );
        assert!(plain.contains("ExecStart=/usr/bin/oxifind --service --delay 15 /home/user/Sync\n"));
    }
}
