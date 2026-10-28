//! Startup check for a newer Tuner release on GitHub. Tuner never replaces
//! its own binaries: packaged installs belong to the package manager, so the
//! notice says how to update for the way this copy was installed. See
//! ADR 0007 and `docs/sources.md` (`check_updates`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

/// `https://github.com/<owner>/<repo>`, from the workspace manifest.
const REPO: &str = env!("CARGO_PKG_REPOSITORY");
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// A newer release than this build, not yet dismissed by the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub version: String,
    /// Update step for this install, e.g. `update tuner-iptv-bin from the AUR`.
    pub how: String,
}

impl Update {
    /// The short form, for when the full notice does not fit.
    pub fn headline(&self) -> String {
        format!("Tuner {} is available", self.version)
    }

    /// Stop showing this version; a later release shows again. Best effort.
    pub fn dismiss(&self) {
        if let Some(path) = cache_file("dismissed-release") {
            let _ = path.parent().map(fs::create_dir_all);
            let _ = fs::write(path, &self.version);
        }
    }
}

impl std::fmt::Display for Update {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} · {}", self.headline(), self.how)
    }
}

/// A newer release, unless the user dismissed that version. Blocking (at
/// most one HTTPS request a day, the answer is cached), so call it off the
/// UI thread. Every failure is silent: a missed notice is harmless.
pub fn check() -> Option<Update> {
    let tag = latest_tag()?;
    let version = tag.trim_start_matches('v');
    let dismissed = cache_file("dismissed-release").and_then(|p| fs::read_to_string(p).ok());
    (newer(version, env!("CARGO_PKG_VERSION"))
        && dismissed.as_deref().map(str::trim) != Some(version))
    .then(|| Update {
        version: version.to_owned(),
        how: how_to_update(version),
    })
}

/// The latest release tag, from the cache file while it is under a day old.
fn latest_tag() -> Option<String> {
    let cache = cache_file("latest-release");
    if let Some(path) = &cache
        && let Ok(modified) = fs::metadata(path).and_then(|m| m.modified())
        && SystemTime::now()
            .duration_since(modified)
            .is_ok_and(|age| age < CHECK_EVERY)
    {
        return fs::read_to_string(path).ok().map(|t| t.trim().to_owned());
    }
    let tag = fetch_tag()?;
    if let Some(path) = cache {
        let _ = path.parent().map(fs::create_dir_all);
        let _ = fs::write(path, &tag);
    }
    Some(tag)
}

fn fetch_tag() -> Option<String> {
    let url = format!(
        "{}/releases/latest",
        REPO.replacen("https://github.com/", "https://api.github.com/repos/", 1)
    );
    let body = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("tuner/", env!("CARGO_PKG_VERSION")))
        .build()
        .get(&url)
        .set("Accept", "application/vnd.github+json")
        .call()
        .ok()?
        .into_string()
        .ok()?;
    tag_name(&body)
}

fn tag_name(body: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    json["tag_name"].as_str().map(str::to_owned)
}

/// `$XDG_CACHE_HOME/tuner/<name>`, falling back to `~/.cache`.
fn cache_file(name: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("tuner").join(name))
}

/// Plain `X.Y.Z` only: a tag that does not parse (a pre-release) never
/// triggers a notice.
fn newer(latest: &str, current: &str) -> bool {
    fn parse(v: &str) -> Option<Vec<u64>> {
        v.split('.').map(|n| n.parse().ok()).collect()
    }
    matches!((parse(latest), parse(current)), (Some(l), Some(c)) if l > c)
}

fn how_to_update(version: &str) -> String {
    let packaged = std::env::current_exe().is_ok_and(|exe| exe.starts_with("/usr"));
    if packaged {
        for pkg in ["tuner-iptv-bin", "tuner-iptv-git"] {
            if pacman_has(pkg) {
                return format!("update {pkg} from the AUR");
            }
        }
        if rpm_has("tuner-iptv") {
            return format!(
                "sudo dnf install {REPO}/releases/download/v{version}/tuner-iptv-{version}-1.x86_64.rpm"
            );
        }
    }
    // .deb (apt cannot install from a URL), tarballs and source builds.
    format!("{REPO}/releases/latest")
}

fn pacman_has(pkg: &str) -> bool {
    let prefix = format!("{pkg}-");
    fs::read_dir(Path::new("/var/lib/pacman/local")).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with(&prefix))
    })
}

fn rpm_has(pkg: &str) -> bool {
    Command::new("rpm")
        .args(["-q", pkg])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_numerically() {
        assert!(newer("0.2.0", "0.1.0"));
        assert!(newer("0.10.0", "0.9.3"));
        assert!(newer("1.0.0", "0.99.99"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1.0", "0.2.0"));
        assert!(!newer("0.3.0-rc1", "0.2.0"));
        assert!(!newer("garbage", "0.2.0"));
    }

    #[test]
    fn reads_the_tag_from_the_release_json() {
        let body = r#"{"tag_name":"v0.2.0","name":"Tuner 0.2.0","draft":false}"#;
        assert_eq!(tag_name(body).as_deref(), Some("v0.2.0"));
        assert_eq!(tag_name(r#"{"message":"Not Found"}"#), None);
        assert_eq!(tag_name("<html>"), None);
    }
}
