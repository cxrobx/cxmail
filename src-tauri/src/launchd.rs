use crate::error::AppError;
use std::path::PathBuf;
use std::process::Command;

const LABEL: &str = "com.cxmail.app.helper";

/// Path to the LaunchAgent plist.
fn plist_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist"))
}

/// Resolve the path to the cxmail-helper binary.
/// In production: lives inside the .app bundle at Contents/MacOS/cxmail-helper
/// In dev: sibling binary in target/debug/
fn helper_binary_path() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    let dir = exe.parent().unwrap_or(std::path::Path::new("."));
    dir.join("cxmail-helper")
}

/// Check if the LaunchAgent plist is installed.
pub fn is_installed() -> bool {
    plist_path().exists()
}

/// Install the LaunchAgent plist and bootstrap it.
pub fn install_launch_agent() -> Result<(), AppError> {
    let helper = helper_binary_path();
    let plist = plist_path();

    // Ensure ~/Library/LaunchAgents/ exists
    if let Some(parent) = plist.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Ensure log directory exists
    let log_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("Library/Logs/CXMail");
    std::fs::create_dir_all(&log_dir)?;
    let log_path = log_dir.join("helper.log");

    // Build plist content
    let plist_content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>

    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>--sync</string>
    </array>

    <key>RunAtLoad</key>
    <true/>

    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>

    <key>ThrottleInterval</key>
    <integer>30</integer>

    <key>StandardErrorPath</key>
    <string>{}</string>

    <key>LimitLoadToSessionType</key>
    <string>Aqua</string>
</dict>
</plist>
"#,
        helper.display(),
        log_path.display(),
    );

    std::fs::write(&plist, plist_content)?;
    log::info!("LaunchAgent plist written to {:?}", plist);

    // Bootstrap into the current user's GUI session
    let uid = unsafe { libc::getuid() };
    let status = Command::new("launchctl")
        .args(["bootstrap", &format!("gui/{uid}"), &plist.to_string_lossy()])
        .status();

    match status {
        Ok(s) if s.success() => {
            log::info!("LaunchAgent bootstrapped successfully");
            Ok(())
        }
        Ok(s) => {
            // Exit code 36 = EALREADY (already loaded) — treat as success
            let code = s.code().unwrap_or(-1);
            if code == 36 {
                log::info!("LaunchAgent already loaded");
                Ok(())
            } else {
                log::warn!("launchctl bootstrap exited with code {}", code);
                // Plist is still written — it will load on next login
                Ok(())
            }
        }
        Err(e) => {
            log::warn!("Failed to run launchctl: {}", e);
            // Plist is still written — it will load on next login
            Ok(())
        }
    }
}

/// Uninstall the LaunchAgent: stop the service and remove the plist.
pub fn uninstall_launch_agent() -> Result<(), AppError> {
    let uid = unsafe { libc::getuid() };

    // Bootout (stop + unregister)
    let _ = Command::new("launchctl")
        .args(["bootout", &format!("gui/{uid}/{LABEL}")])
        .status();

    // Remove plist file
    let plist = plist_path();
    if plist.exists() {
        std::fs::remove_file(&plist)?;
        log::info!("LaunchAgent plist removed");
    }

    Ok(())
}
