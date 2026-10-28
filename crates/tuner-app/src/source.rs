//! Fetching channels from a source: an M3U playlist (path or URL) or an
//! Xtream Codes login. Parsing itself lives in `tuner-core`.

use std::io::Read;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use tuner_core::xtream::{self, Action, Categories, Credentials};
use tuner_core::{Channel, Series, m3u};

use crate::batches::{self, Flow};

/// Something to load channels from.
#[derive(Debug, Clone)]
pub enum Source {
    /// Playlist file path or http(s) URL.
    M3u(String),
    Xtream {
        name: String,
        creds: Credentials,
    },
}

/// A source that is ready to list its channels: logged in, for Xtream.
/// Cheap to get compared to the channel list itself, so front-ends can set
/// up the account (stream limit, header) before the channels arrive.
#[derive(Debug, Clone)]
pub struct Opened {
    /// Provider's connection limit, when the source reports one (Xtream).
    pub max_connections: Option<u32>,
    /// Account name and details for the summary (Xtream).
    account: Option<(String, String)>,
    /// Container to request for live streams (Xtream).
    live_format: String,
}

impl Opened {
    /// One-line description for the status bar, once `channels` are known.
    pub fn summary(&self, channels: usize) -> String {
        match &self.account {
            Some((name, details)) => format!("{name}: {channels} live channels{details}"),
            None => format!("Loaded {channels} channels"),
        }
    }
}

pub struct Loaded {
    pub channels: Vec<Channel>,
    /// One-line description for the status bar.
    pub summary: String,
    /// Provider's connection limit, when the source reports one (Xtream).
    pub max_connections: Option<u32>,
}

/// Connections in use on an account, as reported by the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    pub active: u32,
    pub max: u32,
}

/// On-demand video library (movies and series). Catalogues are large, so
/// front-ends fetch each one the first time the user opens it, on a worker
/// thread, hence `Send + Sync`. Tests use fakes.
///
/// The `_batched` methods hand entries over in parts while the catalogue
/// downloads (see [`Source::channels`]); `on_batch` returning `Break`
/// abandons the rest. The plain methods collect everything.
pub trait Catalog: Send + Sync {
    /// Whether this source has movies and series at all.
    fn has_vod(&self) -> bool;
    fn movies_batched(&self, on_batch: &mut dyn FnMut(Vec<Channel>) -> Flow) -> Result<()>;
    fn series_batched(&self, on_batch: &mut dyn FnMut(Vec<Series>) -> Flow) -> Result<()>;
    fn movies(&self) -> Result<Vec<Channel>> {
        let mut all = Vec::new();
        self.movies_batched(&mut |batch| {
            all.extend(batch);
            Flow::Continue(())
        })?;
        Ok(all)
    }
    fn series(&self) -> Result<Vec<Series>> {
        let mut all = Vec::new();
        self.series_batched(&mut |batch| {
            all.extend(batch);
            Flow::Continue(())
        })?;
        Ok(all)
    }
    /// Episodes of the series with provider id `series_id`, in play order.
    fn episodes(&self, series_id: &str) -> Result<Vec<Channel>>;
}

impl Catalog for Source {
    fn has_vod(&self) -> bool {
        self.xtream_creds().is_some()
    }

    fn movies_batched(&self, on_batch: &mut dyn FnMut(Vec<Channel>) -> Flow) -> Result<()> {
        let (agent, creds, server) = self.vod_session()?;
        let categories = categories(&agent, &creds, Action::VodCategories)
            .with_context(|| format!("failed to fetch movie categories from {server}"))?;
        xtream_batches(&agent, &creds, Action::VodStreams, on_batch, |entry| {
            xtream::movie(entry, &categories, &creds)
        })
        .with_context(|| format!("failed to fetch movies from {server}"))
    }

    fn series_batched(&self, on_batch: &mut dyn FnMut(Vec<Series>) -> Flow) -> Result<()> {
        let (agent, creds, server) = self.vod_session()?;
        let categories = categories(&agent, &creds, Action::SeriesCategories)
            .with_context(|| format!("failed to fetch series categories from {server}"))?;
        xtream_batches(&agent, &creds, Action::Series, on_batch, |entry| {
            xtream::series(entry, &categories)
        })
        .with_context(|| format!("failed to fetch series from {server}"))
    }

    fn episodes(&self, series_id: &str) -> Result<Vec<Channel>> {
        let (agent, creds, server) = self.vod_session()?;
        let info = xtream_call(
            &agent,
            &creds,
            Some(Action::SeriesInfo),
            &[("series_id", series_id)],
        )
        .with_context(|| format!("failed to fetch episodes from {server}"))?;
        xtream::parse_episodes(&info, &creds)
            .with_context(|| format!("failed to parse episodes from {server}"))
    }
}

impl Source {
    /// Xtream login behind this source, including M3U `get.php` links.
    fn xtream_creds(&self) -> Option<Credentials> {
        match self {
            Source::Xtream { creds, .. } => Some(creds.clone()),
            Source::M3u(location) => xtream_from_m3u_url(location),
        }
    }

    fn vod_session(&self) -> Result<(ureq::Agent, Credentials, String)> {
        let creds = self
            .xtream_creds()
            .ok_or_else(|| anyhow!("movies and series need an Xtream source"))?;
        let server = host(creds.server()).to_string();
        Ok((agent(), creds, server))
    }

    /// Short, credential-free label for progress messages.
    pub fn label(&self) -> String {
        match self {
            Source::M3u(location) if is_url(location) => redact_url(location),
            Source::M3u(location) => location.clone(),
            Source::Xtream { name, creds } => format!("{name} ({})", host(creds.server())),
        }
    }

    /// Identifies the provider account, so stream limits apply across
    /// instances and across source types: an Xtream login and an M3U
    /// `get.php?username=…` URL for the same user on the same host share a
    /// key. Contains no password.
    pub fn account_key(&self) -> String {
        match self {
            Source::Xtream { creds, .. } => xtream_key(host(creds.server()), &creds.username),
            Source::M3u(location) if is_url(location) => match query_param(location, "username") {
                Some(user) => xtream_key(host(location), user),
                None => format!("m3u:{}", location.split('?').next().unwrap_or(location)),
            },
            Source::M3u(location) => format!("m3u:{location}"),
        }
    }

    /// Ask the provider how many connections are in use right now.
    /// `Ok(None)` when the source type cannot report it (M3U).
    /// For M3U `get.php?username=…&password=…` links (how Xtream providers
    /// hand out playlists) the same Xtream API is asked with those
    /// credentials, so the check covers them too.
    pub fn usage(&self) -> Result<Option<Usage>> {
        let Some(creds) = self.xtream_creds() else {
            return Ok(None);
        };
        let account = login(&agent(), &creds)?;
        Ok(account.max_connections.map(|max| Usage {
            active: clamp_u32(account.active_connections.unwrap_or(0)),
            max: clamp_u32(max),
        }))
    }

    /// Get ready to list channels: log in (Xtream). M3U sources have
    /// nothing to do until [`Source::channels`].
    pub fn open(&self) -> Result<Opened> {
        let Source::Xtream { name, creds } = self else {
            return Ok(Opened {
                max_connections: None,
                account: None,
                live_format: String::new(),
            });
        };
        let account = login(&agent(), creds)?;
        let mut details = match account.expires {
            Some(ts) => format!(" · expires {}", format_date(ts)),
            None => " · no expiry".to_string(),
        };
        if let Some(max) = account.max_connections {
            let active = account.active_connections.unwrap_or(0);
            details.push_str(&format!(" · {active}/{max} connections"));
        }
        Ok(Opened {
            max_connections: account.max_connections.map(clamp_u32),
            account: Some((name.clone(), details)),
            live_format: account.live_format().to_string(),
        })
    }

    /// Fetch the channels, handing them to `on_batch` in parts while they
    /// download, so a front-end can show the first ones long before a large
    /// list is complete. `on_batch` returning `Break` abandons the rest. An
    /// M3U playlist arrives as one batch.
    pub fn channels(
        &self,
        opened: &Opened,
        on_batch: &mut dyn FnMut(Vec<Channel>) -> Flow,
    ) -> Result<()> {
        match self {
            Source::M3u(location) => {
                let text = if is_url(location) {
                    fetch(agent().get(location))
                        .with_context(|| format!("failed to fetch {}", redact_url(location)))?
                } else {
                    std::fs::read_to_string(location)
                        .with_context(|| format!("failed to read {location}"))?
                };
                let channels = m3u::parse(&text)
                    .with_context(|| format!("failed to parse {}", self.label()))?;
                let _ = on_batch(channels);
                Ok(())
            }
            Source::Xtream { creds, .. } => {
                let agent = agent();
                let server = host(creds.server());
                let categories = categories(&agent, creds, Action::LiveCategories)
                    .with_context(|| format!("failed to fetch live categories from {server}"))?;
                let ext = &opened.live_format;
                xtream_batches(&agent, creds, Action::LiveStreams, on_batch, |entry| {
                    xtream::live_channel(entry, &categories, creds, ext)
                })
                .with_context(|| format!("failed to fetch live streams from {server}"))
            }
        }
    }

    /// [`Source::open`] and all of [`Source::channels`] in one go.
    pub fn load(&self) -> Result<Loaded> {
        let opened = self.open()?;
        let mut channels = Vec::new();
        self.channels(&opened, &mut |batch| {
            channels.extend(batch);
            Flow::Continue(())
        })?;
        Ok(Loaded {
            summary: opened.summary(channels.len()),
            max_connections: opened.max_connections,
            channels,
        })
    }
}

fn xtream_call(
    agent: &ureq::Agent,
    creds: &Credentials,
    action: Option<Action>,
    params: &[(&str, &str)],
) -> Result<String> {
    fetch(xtream_request(agent, creds, action, params))
}

fn xtream_request(
    agent: &ureq::Agent,
    creds: &Credentials,
    action: Option<Action>,
    params: &[(&str, &str)],
) -> ureq::Request {
    let mut req = agent
        .get(&creds.api_endpoint())
        .query("username", &creds.username)
        .query("password", &creds.password);
    if let Some(action) = action {
        req = req.query("action", action.as_str());
    }
    for (key, value) in params {
        req = req.query(key, value);
    }
    req
}

fn login(agent: &ureq::Agent, creds: &Credentials) -> Result<xtream::Account> {
    let server = host(creds.server());
    let body = xtream_call(agent, creds, None, &[])
        .with_context(|| format!("failed to log in to {server}"))?;
    xtream::parse_account(&body).with_context(|| format!("failed to log in to {server}"))
}

fn categories(agent: &ureq::Agent, creds: &Credentials, action: Action) -> Result<Categories> {
    let body = xtream_call(agent, creds, Some(action), &[])?;
    Ok(xtream::parse_categories(&body)?)
}

/// Stream the array answer to `action`, turning each element into an entry
/// with `entry` (dropping the ones it rejects) and passing them on in
/// batches.
fn xtream_batches<T>(
    agent: &ureq::Agent,
    creds: &Credentials,
    action: Action,
    on_batch: &mut dyn FnMut(Vec<T>) -> Flow,
    entry: impl Fn(&serde_json::Value) -> Option<T>,
) -> Result<()> {
    let body = call(xtream_request(agent, creds, Some(action), &[]))?;
    batches::read_array(body.into_reader(), &mut |values| {
        let entries: Vec<T> = values.iter().filter_map(&entry).collect();
        if entries.is_empty() {
            Flow::Continue(())
        } else {
            on_batch(entries)
        }
    })
}

/// Xtream credentials embedded in an M3U `get.php` URL, if any.
fn xtream_from_m3u_url(url: &str) -> Option<Credentials> {
    if !is_url(url) {
        return None;
    }
    let path = url.split('?').next()?;
    if !path.ends_with("/get.php") {
        return None;
    }
    let user = percent_decode(query_param(url, "username")?);
    let pass = percent_decode(query_param(url, "password")?);
    let (scheme, _) = url.split_once("://")?;
    Some(Credentials::new(
        format!("{scheme}://{}", host(url)),
        user,
        pass,
    ))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push((h * 16 + l) as u8);
                    i += 3;
                    continue;
                }
                _ => out.push(b'%'),
            },
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn xtream_key(host: &str, username: &str) -> String {
    format!("xtream:{}:{username}", host.to_ascii_lowercase())
}

fn query_param<'a>(url: &'a str, name: &str) -> Option<&'a str> {
    let (_, query) = url.split_once('?')?;
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
        .filter(|v| !v.is_empty())
}

fn clamp_u32(n: i64) -> u32 {
    n.clamp(0, i64::from(u32::MAX)) as u32
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(60))
        .user_agent(concat!("tuner/", env!("CARGO_PKG_VERSION")))
        .build()
}

/// Perform a GET and read the whole body.
fn fetch(req: ureq::Request) -> Result<String> {
    let mut body = String::new();
    call(req)?.into_reader().read_to_string(&mut body)?;
    Ok(body)
}

/// Perform a GET; the body is left to the caller. Errors never include the
/// request URL, because Xtream URLs carry the password.
fn call(req: ureq::Request) -> Result<ureq::Response> {
    req.call().map_err(|e| match e {
        ureq::Error::Status(code, _) => anyhow!("server returned HTTP {code}"),
        ureq::Error::Transport(t) => match t.message() {
            Some(msg) => anyhow!("{}: {msg}", t.kind()),
            None => anyhow!("{}", t.kind()),
        },
    })
}

fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

/// `scheme://host[:port]` with any userinfo, path and query removed.
fn redact_url(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("http", url));
    format!("{scheme}://{}", host(rest))
}

/// Host and port from a URL or `host:port/...` string, without userinfo.
/// Safe to display: never contains credentials.
pub fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    authority.rsplit_once('@').map_or(authority, |(_, h)| h)
}

/// Unix timestamp → `YYYY-MM-DD` (UTC), without pulling in a date crate.
fn format_date(ts: i64) -> String {
    // Howard Hinnant's days-to-civil algorithm.
    let z = ts.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_credentials_from_urls() {
        assert_eq!(
            redact_url("http://h:8080/get.php?username=u&password=p&type=m3u_plus"),
            "http://h:8080"
        );
        assert_eq!(redact_url("https://user:pw@h/list.m3u"), "https://h");
        assert_eq!(host("http://h:8080/live/u/p/1.ts"), "h:8080");
    }

    #[test]
    fn xtream_label_has_no_password() {
        let source = Source::Xtream {
            name: "p".into(),
            creds: Credentials::new("h:8080", "u", "secret"),
        };
        assert_eq!(source.label(), "p (h:8080)");
    }

    #[test]
    fn same_account_shares_a_key_across_source_types() {
        let xtream = Source::Xtream {
            name: "p".into(),
            creds: Credentials::new("HOST:8080", "alice", "secret"),
        };
        let m3u = Source::M3u(
            "http://host:8080/get.php?username=alice&password=secret&type=m3u_plus".into(),
        );
        assert_eq!(xtream.account_key(), "xtream:host:8080:alice");
        assert_eq!(xtream.account_key(), m3u.account_key());
        assert!(!m3u.account_key().contains("secret"));
        assert_eq!(
            Source::M3u("https://h/list.m3u?token=x".into()).account_key(),
            "m3u:https://h/list.m3u"
        );
    }

    #[test]
    fn m3u_get_php_links_reuse_xtream_credentials() {
        let creds = xtream_from_m3u_url(
            "https://line.example:8443/get.php?username=al%40ice&password=p%26w&type=m3u_plus",
        )
        .unwrap();
        assert_eq!(creds.server(), "https://line.example:8443");
        assert_eq!(creds.username, "al@ice");
        assert_eq!(creds.password, "p&w");
        assert!(xtream_from_m3u_url("https://h/list.m3u?username=a&password=b").is_none());
        assert!(xtream_from_m3u_url("https://h/get.php?username=a").is_none());
        assert_eq!(percent_decode("100%"), "100%");
    }

    #[test]
    fn only_xtream_logins_have_vod() {
        let xtream = Source::Xtream {
            name: "p".into(),
            creds: Credentials::new("h:8080", "u", "secret"),
        };
        assert!(xtream.has_vod());
        assert!(Source::M3u("https://h/get.php?username=a&password=b".into()).has_vod());
        assert!(!Source::M3u("https://h/list.m3u".into()).has_vod());
        assert!(!Source::M3u("/home/me/list.m3u".into()).has_vod());
        let err = Source::M3u("/home/me/list.m3u".into())
            .movies()
            .unwrap_err();
        assert!(err.to_string().contains("Xtream"));
    }

    #[test]
    fn formats_unix_dates() {
        assert_eq!(format_date(0), "1970-01-01");
        assert_eq!(format_date(951_782_400), "2000-02-29");
        assert_eq!(format_date(1_798_761_600), "2027-01-01");
    }
}
