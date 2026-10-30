//! Update checker: asks GitHub for the latest published release (drafts and
//! pre-releases don't count) and compares its version with this build.
//!
//! The request runs `curl` (built into macOS and Windows 10+) on a
//! background thread, so the app needs no HTTP or TLS library and startup
//! never waits for the network.

use std::sync::mpsc::{Receiver, channel};

/// The GitHub repository releases are published to.
pub const REPO: &str = "darshan5/superfastnotepad";

/// A release newer than this build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    /// Without the leading `v`, e.g. `0.2.0`.
    pub version: String,
    /// The release's web page.
    pub url: String,
}

/// The outcome of a check: a newer release, or `None` if this is the latest.
pub type CheckResult = Result<Option<Update>, String>;

/// This build's version.
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The numeric parts of `1.2.3` (a `v` prefix and `-beta` style suffixes are
/// ignored).
fn parts(v: &str) -> Vec<u64> {
    v.trim()
        .trim_start_matches('v')
        .split(['-', '+'])
        .next()
        .unwrap_or("")
        .split('.')
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

/// Is version `latest` newer than `current`?
pub fn is_newer(latest: &str, current: &str) -> bool {
    let (a, b) = (parts(latest), parts(current));
    let n = a.len().max(b.len());
    for i in 0..n {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    false
}

/// Reads GitHub's "latest release" JSON; `Some` if it's newer than
/// `current`.
pub fn parse_latest(json: &str, current: &str) -> CheckResult {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("Unexpected reply from GitHub: {e}"))?;
    let tag = v["tag_name"]
        .as_str()
        .ok_or_else(|| match v["message"].as_str() {
            // e.g. "Not Found" before the first release is published.
            Some(m) => format!("GitHub: {m}"),
            None => "No release found on GitHub".to_string(),
        })?;
    let version = tag.trim_start_matches('v').to_string();
    let url = v["html_url"]
        .as_str()
        .map_or_else(releases_page, str::to_string);
    Ok(is_newer(&version, current).then_some(Update { version, url }))
}

/// Where all releases are listed.
pub fn releases_page() -> String {
    format!("https://github.com/{REPO}/releases")
}

/// Asks GitHub for the latest release (blocking; call it off the UI thread).
pub fn check() -> CheckResult {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let mut cmd = std::process::Command::new("curl");
    cmd.args([
        "--silent",
        "--show-error",
        "--location",
        "--max-time",
        "15",
        "--header",
        "Accept: application/vnd.github+json",
        "--user-agent",
        concat!("superfastnotepad/", env!("CARGO_PKG_VERSION")),
        &url,
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashing up.
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("Couldn't run curl to check for updates: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() {
            "Couldn't reach GitHub".to_string()
        } else {
            format!("Couldn't reach GitHub: {err}")
        });
    }
    parse_latest(&String::from_utf8_lossy(&out.stdout), current_version())
}

/// Starts a check on a background thread; `done` runs when it finishes
/// (to wake the UI).
pub fn spawn_check(done: impl FnOnce() + Send + 'static) -> Receiver<CheckResult> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(check());
        done();
    });
    rx
}

/// Update checking as the app sees it.
#[derive(Default)]
pub struct UpdateState {
    /// The automatic check has been started this run.
    pub started: bool,
    pub job: Option<Receiver<CheckResult>>,
    /// A newer release, shown as "Update Available" in the File menu.
    pub available: Option<Update>,
    /// The result of the last check started from Settings, for display.
    pub status: Option<String>,
    /// The running check was started from Settings (so report the result).
    pub manual: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(is_newer("1.0", "0.9.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("1.0.0-beta", "1.0.0"));
    }

    #[test]
    fn reads_github_replies() {
        let newer = r#"{"tag_name": "v0.3.1", "html_url": "https://example.com/r/0.3.1"}"#;
        assert_eq!(
            parse_latest(newer, "0.1.0"),
            Ok(Some(Update {
                version: "0.3.1".into(),
                url: "https://example.com/r/0.3.1".into()
            }))
        );
        assert_eq!(parse_latest(newer, "0.3.1"), Ok(None));
        let none = r#"{"message": "Not Found", "status": "404"}"#;
        assert_eq!(parse_latest(none, "0.1.0"), Err("GitHub: Not Found".into()));
        assert!(parse_latest("<html>", "0.1.0").is_err());
    }

    /// Asks the real GitHub (network): `cargo test --release live_check -- --ignored --nocapture`
    #[test]
    #[ignore = "network"]
    fn live_check() {
        println!("{:?}", check());
    }
}
