//! Open-at-login via a user `LaunchAgent` plist + `launchctl`.
//!
//! No entitlements or Developer-ID signature needed, unlike `SMAppService`.
//! The plist points at the running executable, so a dev build and an
//! installed build each register themselves correctly.

use std::path::PathBuf;

pub const LABEL: &str = "com.zacharyzampa.shouci";

fn launch_agents_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| String::from("/tmp"));
    PathBuf::from(format!("{home}/Library/LaunchAgents"))
}

fn plist_path() -> PathBuf {
    launch_agents_dir().join(format!("{LABEL}.plist"))
}

fn current_uid() -> Result<String, String> {
    let output = std::process::Command::new("id")
        .arg("-u")
        .output()
        .map_err(|err| format!("cannot determine uid: {err}"))?;
    if !output.status.success() {
        return Err(String::from("cannot determine uid: id -u failed"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn launchctl(args: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new("launchctl")
        .args(args)
        .output()
        .map_err(|err| format!("cannot run launchctl: {err}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "launchctl {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// Whether open-at-login is currently configured (plist present).
#[must_use]
pub fn is_enabled() -> bool {
    plist_path().is_file()
}

/// Enables or disables open-at-login. Enabling writes the plist and
/// bootstraps it immediately; disabling boots it out and removes the file.
/// The executable path is resolved from the running process, so this works
/// from a staging build as well as the installed app.
///
/// # Errors
///
/// Returns a human-readable message when the plist cannot be written,
/// `launchctl` fails, or the executable path cannot be determined.
pub fn set_enabled(enabled: bool) -> Result<(), String> {
    let plist = plist_path();
    let uid = current_uid()?;
    let domain = format!("gui/{uid}");
    if enabled {
        let executable =
            std::env::current_exe().map_err(|err| format!("cannot locate executable: {err}"))?;
        let content = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>ProcessType</key>
    <string>Background</string>
</dict>
</plist>
"#,
            executable.display()
        );
        std::fs::create_dir_all(launch_agents_dir())
            .map_err(|err| format!("cannot create LaunchAgents dir: {err}"))?;
        // Boot out any stale registration first so re-enabling is idempotent.
        let _ignored = launchctl(&["bootout", &domain, &plist.to_string_lossy()]);
        std::fs::write(&plist, content)
            .map_err(|err| format!("cannot write {}: {err}", plist.display()))?;
        launchctl(&["bootstrap", &domain, &plist.to_string_lossy()])?;
        Ok(())
    } else {
        let _ignored = launchctl(&["bootout", &domain, &plist.to_string_lossy()]);
        if plist.is_file() {
            std::fs::remove_file(&plist)
                .map_err(|err| format!("cannot remove {}: {err}", plist.display()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_is_stable() {
        // The checkbox, the plist, and anyone tailing logs agree on this.
        assert_eq!(LABEL, "com.zacharyzampa.shouci");
    }

    #[test]
    fn plist_lives_in_user_agents() {
        assert!(
            plist_path()
                .to_string_lossy()
                .ends_with("Library/LaunchAgents/com.zacharyzampa.shouci.plist")
        );
    }
}
