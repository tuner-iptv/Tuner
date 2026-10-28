//! `~/.config/tuner/config.toml`: saved sources (M3U, Xtream logins) and mpv
//! defaults. Read at startup and written by the front-ends' account
//! managers. Format and credential handling: `docs/sources.md`.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tuner_core::xtream::Credentials;

use crate::guard::LimitMode;
use crate::source::Source;

#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Colour scheme of the GUI: a built-in theme name, or `custom` (the
    /// default) for `theme.toml` beside this file. Only the GUI reads it, and
    /// it validates the name, so this crate stays free of UI knowledge. Must
    /// stay the first field: TOML wants plain values before tables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// Look for a newer release on GitHub at startup (at most once a day).
    /// Unset means yes. See `update`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_updates: Option<bool>,
    #[serde(default, rename = "source", skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<SourceConfig>,
    #[serde(default, skip_serializing_if = "MpvConfig::is_empty")]
    pub mpv: MpvConfig,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MpvConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

impl MpvConfig {
    fn is_empty(&self) -> bool {
        self.program.is_none() && self.args.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SourceConfig {
    pub name: String,
    /// Streams this account may play at once. Defaults to the provider's
    /// `max_connections` (Xtream) or 1. See `docs/stream-limits.md`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_streams: Option<u32>,
    /// What to do when the provider reports the account as full.
    #[serde(default, skip_serializing_if = "LimitMode::is_default")]
    pub limit_mode: LimitMode,
    #[serde(flatten)]
    pub kind: SourceKind,
}

/// `Debug` is implemented by hand so passwords never reach logs.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SourceKind {
    M3u {
        /// File path or http(s) URL.
        location: String,
    },
    Xtream {
        server: String,
        username: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        password: Option<String>,
        /// Shell command printing the password, e.g. `pass show iptv/provider`.
        #[serde(skip_serializing_if = "Option::is_none")]
        password_command: Option<String>,
    },
}

impl std::fmt::Debug for SourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceKind::M3u { location } => {
                f.debug_struct("M3u").field("location", location).finish()
            }
            SourceKind::Xtream {
                server,
                username,
                password,
                password_command,
            } => f
                .debug_struct("Xtream")
                .field("server", server)
                .field("username", username)
                .field("password", &password.as_ref().map(|_| "<redacted>"))
                .field("password_command", password_command)
                .finish(),
        }
    }
}

impl Config {
    /// Default location: `$XDG_CONFIG_HOME/tuner/config.toml`, falling back
    /// to `~/.config/tuner/config.toml`.
    pub fn default_path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("tuner").join("config.toml"))
    }

    /// Load `path`. A missing file is an empty config; a malformed one is an
    /// error.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).with_context(|| format!("invalid {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("failed to read {}", path.display())),
        }
    }

    pub fn parse(text: &str) -> Result<Self> {
        Ok(toml::from_str(text)?)
    }

    pub fn to_toml(&self) -> Result<String> {
        Ok(format!(
            "# tuner config. Edit freely, or manage accounts in the app.\n\
             # Comments are not preserved when tuner saves this file.\n\n{}",
            toml::to_string_pretty(self)?
        ))
    }

    /// Write atomically with mode 0600 (it may hold passwords). The previous
    /// file is kept as `config.toml.bak`.
    pub fn save(&self, path: &Path) -> Result<()> {
        let text = self.to_toml()?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)
                .with_context(|| format!("failed to create {}", dir.display()))?;
        }
        if path.exists() {
            let backup = path.with_extension("toml.bak");
            fs::copy(path, &backup)
                .with_context(|| format!("failed to back up to {}", backup.display()))?;
        }
        let tmp = path.with_extension("toml.tmp");
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .with_context(|| format!("failed to write {}", tmp.display()))?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, path).with_context(|| format!("failed to replace {}", path.display()))?;
        Ok(())
    }

    pub fn source(&self, name: &str) -> Option<&SourceConfig> {
        self.sources.iter().find(|s| s.name == name)
    }

    /// Add `source`, or replace the one called `original` (which may be a
    /// rename). Validates before changing anything.
    pub fn upsert(&mut self, original: Option<&str>, source: SourceConfig) -> Result<()> {
        source.validate()?;
        let clash = self
            .sources
            .iter()
            .any(|s| s.name == source.name && Some(s.name.as_str()) != original);
        if clash {
            bail!("an account named '{}' already exists", source.name);
        }
        match original.and_then(|o| self.sources.iter().position(|s| s.name == o)) {
            Some(i) => self.sources[i] = source,
            None => self.sources.push(source),
        }
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.sources.len();
        self.sources.retain(|s| s.name != name);
        self.sources.len() != before
    }

    /// Move `name` to the top, making it what `tuner` opens by default.
    pub fn make_default(&mut self, name: &str) {
        if let Some(i) = self.sources.iter().position(|s| s.name == name) {
            let source = self.sources.remove(i);
            self.sources.insert(0, source);
        }
    }

    /// Pick what to open. `arg` is a configured source name, else a playlist
    /// path/URL. Without `arg`, the first configured source is used.
    pub fn resolve(&self, arg: Option<&str>) -> Result<Resolved> {
        let configured = match arg {
            Some(arg) => match self.sources.iter().find(|s| s.name == arg) {
                Some(source) => source,
                None => {
                    return Ok(Resolved {
                        source: Source::M3u(arg.to_string()),
                        max_streams: None,
                        limit_mode: LimitMode::default(),
                    });
                }
            },
            None => match self.sources.first() {
                Some(source) => source,
                None => bail!(
                    "no playlist given and no sources configured\n\
                     usage: tuner <playlist path or URL>, or add a [[source]] to {}\n\
                     see docs/sources.md",
                    Self::default_path()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "~/.config/tuner/config.toml".into())
                ),
            },
        };
        configured.resolve()
    }
}

/// A source chosen by [`Config::resolve`], with its per-source settings.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub source: Source,
    /// Explicit `max_streams` from the config, if set.
    pub max_streams: Option<u32>,
    pub limit_mode: LimitMode,
}

impl SourceConfig {
    /// Turn this entry into something to open (runs `password_command`).
    pub fn resolve(&self) -> Result<Resolved> {
        Ok(Resolved {
            source: self.to_source()?,
            max_streams: self.max_streams,
            limit_mode: self.limit_mode,
        })
    }

    /// Check the entry is complete, without contacting anything.
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            bail!("name is required");
        }
        match &self.kind {
            SourceKind::M3u { location } if location.trim().is_empty() => {
                bail!("playlist URL or file is required")
            }
            SourceKind::M3u { .. } => {}
            SourceKind::Xtream {
                server,
                username,
                password,
                password_command,
            } => {
                if server.trim().is_empty() {
                    bail!("server is required");
                }
                if username.trim().is_empty() {
                    bail!("username is required");
                }
                let has = |v: &Option<String>| v.as_deref().is_some_and(|s| !s.is_empty());
                match (has(password), has(password_command)) {
                    (true, true) => bail!("set either a password or a password command, not both"),
                    (false, false) => bail!("a password or a password command is required"),
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn to_source(&self) -> Result<Source> {
        Ok(match &self.kind {
            SourceKind::M3u { location } => Source::M3u(location.clone()),
            SourceKind::Xtream {
                server,
                username,
                password,
                password_command,
            } => {
                let password = match (password, password_command) {
                    (Some(p), None) => p.clone(),
                    (None, Some(cmd)) => run_password_command(cmd).with_context(|| {
                        format!("source '{}': password_command failed", self.name)
                    })?,
                    (Some(_), Some(_)) => bail!(
                        "source '{}': set either password or password_command, not both",
                        self.name
                    ),
                    (None, None) => bail!(
                        "source '{}': xtream needs password or password_command",
                        self.name
                    ),
                };
                Source::Xtream {
                    name: self.name.clone(),
                    creds: Credentials::new(server, username, password),
                }
            }
        })
    }
}

fn run_password_command(cmd: &str) -> Result<String> {
    let output = Command::new("sh").arg("-c").arg(cmd).output()?;
    if !output.status.success() {
        bail!("exited with {}", output.status);
    }
    let text = String::from_utf8(output.stdout).context("output is not UTF-8")?;
    // Like `pass`, take the first line only.
    let password = text.lines().next().unwrap_or_default().to_string();
    if password.is_empty() {
        bail!("printed an empty password");
    }
    Ok(password)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
        [mpv]
        args = ["--hwdec=auto"]

        [[source]]
        name = "provider"
        max_streams = 2
        limit_mode = "block"
        type = "xtream"
        server = "http://example.com:8080"
        username = "user"
        password_command = "echo secret; echo ignored"

        [[source]]
        name = "free"
        type = "m3u"
        location = "https://example.com/list.m3u"
    "#;

    #[test]
    fn first_source_is_the_default() {
        let config = Config::parse(SAMPLE).unwrap();
        assert_eq!(config.mpv.args, ["--hwdec=auto"]);
        assert_eq!(config.resolve(None).unwrap().max_streams, Some(2));
        assert_eq!(config.resolve(None).unwrap().limit_mode, LimitMode::Block);
        assert_eq!(
            config.resolve(Some("free")).unwrap().limit_mode,
            LimitMode::Confirm
        );
        match config.resolve(None).unwrap().source {
            Source::Xtream { name, creds } => {
                assert_eq!(name, "provider");
                assert_eq!(creds.username, "user");
                assert_eq!(creds.password, "secret");
            }
            other => panic!("expected xtream, got {other:?}"),
        }
    }

    #[test]
    fn arg_selects_by_name_else_is_a_playlist() {
        let config = Config::parse(SAMPLE).unwrap();
        assert!(
            matches!(config.resolve(Some("free")).unwrap().source, Source::M3u(l) if l == "https://example.com/list.m3u")
        );
        assert!(
            matches!(config.resolve(Some("x.m3u")).unwrap().source, Source::M3u(l) if l == "x.m3u")
        );
    }

    #[test]
    fn empty_config_needs_an_argument() {
        assert!(Config::default().resolve(None).is_err());
    }

    #[test]
    fn xtream_requires_exactly_one_password_option() {
        let base = "[[source]]\nname='p'\ntype='xtream'\nserver='h'\nusername='u'\n";
        assert!(Config::parse(base).unwrap().resolve(None).is_err());
        let both = format!("{base}password='a'\npassword_command='echo b'\n");
        assert!(Config::parse(&both).unwrap().resolve(None).is_err());
    }

    fn xtream(name: &str) -> SourceConfig {
        SourceConfig {
            name: name.into(),
            max_streams: None,
            limit_mode: LimitMode::Confirm,
            kind: SourceKind::Xtream {
                server: "http://h:8080".into(),
                username: "u".into(),
                password: Some("secret".into()),
                password_command: None,
            },
        }
    }

    #[test]
    fn round_trips_through_toml() {
        let config = Config::parse(SAMPLE).unwrap();
        let again = Config::parse(&config.to_toml().unwrap()).unwrap();
        assert_eq!(config, again);
    }

    #[test]
    fn upsert_adds_renames_and_rejects_duplicates() {
        let mut config = Config::default();
        config.upsert(None, xtream("a")).unwrap();
        config.upsert(None, xtream("b")).unwrap();
        assert!(config.upsert(None, xtream("a")).is_err());
        config.upsert(Some("a"), xtream("c")).unwrap();
        let names: Vec<_> = config.sources.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["c", "b"]);
        assert!(config.upsert(Some("c"), xtream("b")).is_err());
        config.make_default("b");
        assert_eq!(config.sources[0].name, "b");
        assert!(config.remove("c"));
        assert!(!config.remove("c"));
    }

    #[test]
    fn validation_catches_incomplete_accounts() {
        let mut s = xtream(" ");
        assert!(s.validate().is_err());
        s.name = "x".into();
        assert!(s.validate().is_ok());
        if let SourceKind::Xtream { password, .. } = &mut s.kind {
            *password = Some(String::new());
        }
        assert!(s.validate().is_err());
        let m3u = SourceConfig {
            kind: SourceKind::M3u {
                location: "".into(),
            },
            ..xtream("m")
        };
        assert!(m3u.validate().is_err());
    }

    #[test]
    fn debug_never_shows_passwords() {
        assert!(!format!("{:?}", xtream("a")).contains("secret"));
    }

    #[test]
    fn save_is_private_and_keeps_a_backup() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("tuner-cfg-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("config.toml");
        let mut config = Config::default();
        config.upsert(None, xtream("a")).unwrap();
        config.save(&path).unwrap();
        config.upsert(None, xtream("b")).unwrap();
        config.save(&path).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(Config::load(&path).unwrap(), config);
        assert_eq!(
            Config::load(&path.with_extension("toml.bak"))
                .unwrap()
                .sources
                .len(),
            1
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn theme_is_optional_and_survives_a_save() {
        assert_eq!(Config::parse(SAMPLE).unwrap().theme, None);
        assert!(!Config::default().to_toml().unwrap().contains("theme"));

        let text = format!("theme = \"catppuccin-mocha\"\n{SAMPLE}");
        let config = Config::parse(&text).unwrap();
        assert_eq!(config.theme.as_deref(), Some("catppuccin-mocha"));
        // Saving from the accounts window must not drop it.
        let again = Config::parse(&config.to_toml().unwrap()).unwrap();
        assert_eq!(again, config);
    }

    #[test]
    fn rejects_unknown_top_level_keys() {
        assert!(Config::parse("sources = []").is_err());
    }
}
