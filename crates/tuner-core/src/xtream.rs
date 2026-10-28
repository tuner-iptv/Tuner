//! Xtream Codes API: credentials, stream URL building and lenient response
//! parsing. Fetching is done by the caller; this module only sees strings.
//! See `docs/sources.md`.
//!
//! Panels disagree on JSON types (ids as numbers or strings, `null` where a
//! value is expected, `{}` instead of `[]`), so everything is read through
//! `serde_json::Value` with tolerant accessors instead of strict structs.

use std::collections::HashMap;

use serde_json::Value;

use crate::{Channel, Series};

#[derive(Debug, thiserror::Error)]
pub enum XtreamError {
    #[error("login failed: wrong username or password")]
    AuthFailed,
    #[error("account is not active (status: {0})")]
    Inactive(String),
    #[error("unexpected response from server: {0}")]
    Unexpected(&'static str),
    #[error("invalid JSON from server: {0}")]
    Json(#[from] serde_json::Error),
}

/// API actions tuner uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    LiveCategories,
    LiveStreams,
    VodCategories,
    VodStreams,
    SeriesCategories,
    Series,
    /// Needs a `series_id` parameter.
    SeriesInfo,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Action::LiveCategories => "get_live_categories",
            Action::LiveStreams => "get_live_streams",
            Action::VodCategories => "get_vod_categories",
            Action::VodStreams => "get_vod_streams",
            Action::SeriesCategories => "get_series_categories",
            Action::Series => "get_series",
            Action::SeriesInfo => "get_series_info",
        }
    }
}

/// Login for one Xtream server.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    server: String,
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("server", &self.server)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

impl Credentials {
    /// `server` may omit the scheme (`http://` is assumed) and may carry a
    /// trailing slash or a pasted `/player_api.php` / `/get.php` path.
    pub fn new(
        server: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        let mut server = server.into().trim().to_string();
        if !server.contains("://") {
            server.insert_str(0, "http://");
        }
        for suffix in ["/player_api.php", "/get.php"] {
            if let Some(stripped) = server.strip_suffix(suffix) {
                server.truncate(stripped.len());
            }
        }
        while server.ends_with('/') {
            server.pop();
        }
        Self {
            server,
            username: username.into(),
            password: password.into(),
        }
    }

    /// Base URL without credentials, e.g. `http://host:8080`.
    pub fn server(&self) -> &str {
        &self.server
    }

    /// Endpoint for API calls. Credentials and `action` go in the query
    /// string, which the HTTP client must add (and encode).
    pub fn api_endpoint(&self) -> String {
        format!("{}/player_api.php", self.server)
    }

    /// Playable URL for a live stream. `ext` is `ts` or `m3u8`.
    pub fn live_url(&self, stream_id: &str, ext: &str) -> String {
        self.media_url("live", stream_id, ext)
    }

    /// Playable URL for a movie (`stream_id` from `get_vod_streams`). `ext`
    /// is the entry's `container_extension`, e.g. `mkv`.
    pub fn movie_url(&self, stream_id: &str, ext: &str) -> String {
        self.media_url("movie", stream_id, ext)
    }

    /// Playable URL for one episode (`id` from `get_series_info`).
    pub fn episode_url(&self, episode_id: &str, ext: &str) -> String {
        self.media_url("series", episode_id, ext)
    }

    fn media_url(&self, kind: &str, id: &str, ext: &str) -> String {
        format!(
            "{}/{kind}/{}/{}/{}.{}",
            self.server,
            encode_segment(&self.username),
            encode_segment(&self.password),
            encode_segment(id),
            encode_segment(ext)
        )
    }
}

/// Account details from the login response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub status: String,
    /// Expiry as a Unix timestamp; `None` means unlimited or unknown.
    pub expires: Option<i64>,
    pub max_connections: Option<i64>,
    pub active_connections: Option<i64>,
    pub output_formats: Vec<String>,
}

impl Account {
    /// Container to request for live streams: `ts` when allowed (most
    /// robust for live TV in mpv), else the first format the panel allows.
    pub fn live_format(&self) -> &str {
        if self.output_formats.is_empty() || self.output_formats.iter().any(|f| f == "ts") {
            "ts"
        } else {
            &self.output_formats[0]
        }
    }
}

/// Parse the response of `player_api.php` without an `action` (login).
pub fn parse_account(json: &str) -> Result<Account, XtreamError> {
    let root: Value = serde_json::from_str(json)?;
    let info = root
        .get("user_info")
        .filter(|v| v.is_object())
        .ok_or(XtreamError::Unexpected("missing user_info"))?;

    if as_int(&info["auth"]) != Some(1) {
        return Err(XtreamError::AuthFailed);
    }
    let status = as_text(&info["status"]).unwrap_or_else(|| "Active".into());
    if !status.eq_ignore_ascii_case("active") {
        return Err(XtreamError::Inactive(status));
    }

    let output_formats = info["allowed_output_formats"]
        .as_array()
        .map(|a| a.iter().filter_map(as_text).collect())
        .unwrap_or_default();

    Ok(Account {
        status,
        expires: as_int(&info["exp_date"]).filter(|&t| t > 0),
        max_connections: as_int(&info["max_connections"]),
        active_connections: as_int(&info["active_cons"]),
        output_formats,
    })
}

/// `category_id` → `category_name`, from a `get_*_categories` response.
pub type Categories = HashMap<String, String>;

/// Build channels from `get_live_categories` and `get_live_streams`
/// responses. Streams without an id are skipped; unknown categories leave
/// the group empty.
pub fn parse_live(
    categories_json: &str,
    streams_json: &str,
    creds: &Credentials,
    ext: &str,
) -> Result<Vec<Channel>, XtreamError> {
    let categories = parse_categories(categories_json)?;
    let streams: Value = serde_json::from_str(streams_json)?;
    Ok(items(&streams)
        .filter_map(|s| live_channel(s, &categories, creds, ext))
        .collect())
}

/// One element of a `get_live_streams` response as a channel, or `None`
/// without a `stream_id`. For callers that read the array incrementally.
pub fn live_channel(
    stream: &Value,
    categories: &Categories,
    creds: &Credentials,
    ext: &str,
) -> Option<Channel> {
    let s = stream;
    let id = as_text(&s["stream_id"])?;
    let mut channel = Channel::new(
        as_text(&s["name"]).unwrap_or_else(|| format!("Stream {id}")),
        creds.live_url(&id, ext),
    );
    channel.group = category(s, categories);
    channel.logo = as_text(&s["stream_icon"]);
    channel.tvg_id = as_text(&s["epg_channel_id"]);
    channel.number = as_int(&s["num"]).and_then(|n| u32::try_from(n).ok());
    if as_int(&s["tv_archive"]) == Some(1) {
        channel.catchup_days = as_int(&s["tv_archive_duration"])
            .and_then(|d| u32::try_from(d).ok())
            .filter(|&d| d > 0);
    }
    Some(channel)
}

/// Build movies from `get_vod_categories` and `get_vod_streams` responses.
/// Movies are plain [`Channel`]s (category as group, poster as logo), so
/// they browse and play like live channels. Entries without an id are
/// skipped; a missing container defaults to `mp4`.
pub fn parse_movies(
    categories_json: &str,
    streams_json: &str,
    creds: &Credentials,
) -> Result<Vec<Channel>, XtreamError> {
    let categories = parse_categories(categories_json)?;
    let streams: Value = serde_json::from_str(streams_json)?;
    Ok(items(&streams)
        .filter_map(|s| movie(s, &categories, creds))
        .collect())
}

/// One element of a `get_vod_streams` response, or `None` without an id.
pub fn movie(stream: &Value, categories: &Categories, creds: &Credentials) -> Option<Channel> {
    let s = stream;
    let id = as_text(&s["stream_id"])?;
    let ext = as_text(&s["container_extension"]).unwrap_or_else(|| "mp4".into());
    let mut movie = Channel::new(
        as_text(&s["name"]).unwrap_or_else(|| format!("Movie {id}")),
        creds.movie_url(&id, &ext),
    );
    movie.group = category(s, categories);
    movie.logo = as_text(&s["stream_icon"]);
    Some(movie)
}

/// Build the series list from `get_series_categories` and `get_series`
/// responses. Entries without a `series_id` are skipped.
pub fn parse_series(categories_json: &str, series_json: &str) -> Result<Vec<Series>, XtreamError> {
    let categories = parse_categories(categories_json)?;
    let list: Value = serde_json::from_str(series_json)?;
    Ok(items(&list)
        .filter_map(|s| series(s, &categories))
        .collect())
}

/// One element of a `get_series` response, or `None` without a `series_id`.
pub fn series(entry: &Value, categories: &Categories) -> Option<Series> {
    let s = entry;
    let id = as_text(&s["series_id"])?;
    let release = as_text(&s["releaseDate"])
        .or_else(|| as_text(&s["release_date"]))
        .or_else(|| as_text(&s["year"]));
    Some(Series {
        name: as_text(&s["name"]).unwrap_or_else(|| format!("Series {id}")),
        id,
        group: category(s, categories),
        cover: as_text(&s["cover"]),
        plot: as_text(&s["plot"]),
        year: release.and_then(|r| year_of(&r)),
        rating: as_text(&s["rating"]).filter(|r| r.parse::<f32>().is_ok_and(|n| n > 0.0)),
    })
}

fn category(entry: &Value, categories: &Categories) -> Option<String> {
    as_text(&entry["category_id"]).and_then(|c| categories.get(&c).cloned())
}

/// Build one series' episodes from a `get_series_info` response, ordered by
/// season and episode number. Each season becomes a group ("Season 2"), so
/// the groups pane doubles as a season picker. `episodes` is an object keyed
/// by season on most panels and an array of arrays on some; both work.
pub fn parse_episodes(json: &str, creds: &Credentials) -> Result<Vec<Channel>, XtreamError> {
    let root: Value = serde_json::from_str(json)?;
    let seasons: Vec<(i64, &Value)> = match &root["episodes"] {
        Value::Object(map) => map
            .iter()
            .map(|(key, list)| (key.trim().parse().unwrap_or(0), list))
            .collect(),
        Value::Array(lists) => lists.iter().zip(1..).map(|(list, n)| (n, list)).collect(),
        _ => Vec::new(),
    };

    let mut episodes: Vec<(i64, i64, Channel)> = Vec::new();
    for (key, list) in seasons {
        for e in items(list) {
            let Some(id) = as_text(&e["id"]) else {
                continue;
            };
            let season = as_int(&e["season"]).unwrap_or(key);
            let number = as_int(&e["episode_num"]);
            let ext = as_text(&e["container_extension"]).unwrap_or_else(|| "mp4".into());
            let label = match number {
                Some(n) => format!("S{season:02}E{n:02}"),
                None => format!("S{season:02}"),
            };
            let name = match as_text(&e["title"]) {
                Some(title) => format!("{label}  {title}"),
                None => label,
            };
            let mut episode = Channel::new(name, creds.episode_url(&id, &ext));
            episode.group = Some(format!("Season {season}"));
            episode.logo = as_text(&e["info"]["movie_image"]);
            episodes.push((season, number.unwrap_or(i64::MAX), episode));
        }
    }
    episodes.sort_by_key(|(season, number, _)| (*season, *number));
    Ok(episodes.into_iter().map(|(_, _, e)| e).collect())
}

/// Parse a `get_live_categories` / `get_vod_categories` /
/// `get_series_categories` response.
pub fn parse_categories(json: &str) -> Result<Categories, XtreamError> {
    Ok(items(&serde_json::from_str(json)?)
        .filter_map(|c| Some((as_text(&c["category_id"])?, as_text(&c["category_name"])?)))
        .collect())
}

/// Leading four-digit year of `2019-05-01`, `2019` etc.
fn year_of(date: &str) -> Option<String> {
    let year = date.get(..4)?;
    (year.bytes().all(|b| b.is_ascii_digit()) && year != "0000").then(|| year.to_string())
}

/// Array elements, or nothing for the `{}`/`null` some panels send when empty.
fn items(v: &Value) -> impl Iterator<Item = &Value> {
    v.as_array().into_iter().flatten()
}

/// Non-empty string, or a number rendered as text.
fn as_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Integer from a number or a numeric string.
fn as_int(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Percent-encode everything except RFC 3986 unreserved characters.
fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds() -> Credentials {
        Credentials::new("example.com:8080/", "user", "p@ss/1")
    }

    #[test]
    fn normalises_server() {
        assert_eq!(creds().server(), "http://example.com:8080");
        assert_eq!(
            Credentials::new("https://h/player_api.php", "u", "p").server(),
            "https://h"
        );
        assert_eq!(
            creds().api_endpoint(),
            "http://example.com:8080/player_api.php"
        );
    }

    #[test]
    fn live_url_encodes_credentials() {
        assert_eq!(
            creds().live_url("42", "ts"),
            "http://example.com:8080/live/user/p%40ss%2F1/42.ts"
        );
    }

    #[test]
    fn debug_redacts_password() {
        assert!(!format!("{:?}", creds()).contains("p@ss"));
    }

    #[test]
    fn parses_active_account_with_string_fields() {
        let json = r#"{"user_info":{"auth":1,"status":"Active","exp_date":"1798761600",
            "max_connections":"2","active_cons":"0","allowed_output_formats":["m3u8","ts"]},
            "server_info":{"url":"example.com"}}"#;
        let account = parse_account(json).unwrap();
        assert_eq!(account.expires, Some(1_798_761_600));
        assert_eq!(account.max_connections, Some(2));
        assert_eq!(account.live_format(), "ts");
    }

    #[test]
    fn unlimited_account_and_m3u8_only() {
        let json = r#"{"user_info":{"auth":"1","status":"Active","exp_date":null,
            "allowed_output_formats":["m3u8"]}}"#;
        let account = parse_account(json).unwrap();
        assert_eq!(account.expires, None);
        assert_eq!(account.live_format(), "m3u8");
    }

    #[test]
    fn rejects_bad_login_and_expired_account() {
        assert!(matches!(
            parse_account(r#"{"user_info":{"auth":0}}"#),
            Err(XtreamError::AuthFailed)
        ));
        assert!(matches!(
            parse_account(r#"{"user_info":{"auth":1,"status":"Expired"}}"#),
            Err(XtreamError::Inactive(s)) if s == "Expired"
        ));
        assert!(matches!(
            parse_account("[]"),
            Err(XtreamError::Unexpected(_))
        ));
    }

    #[test]
    fn parses_live_streams_with_mixed_types() {
        let categories = r#"[{"category_id":"1","category_name":"News"},
            {"category_id":2,"category_name":"Sports"}]"#;
        let streams = r#"[
            {"num":1,"name":"BBC News","stream_id":101,"stream_icon":"http://x/bbc.png",
             "epg_channel_id":"bbcnews.uk","category_id":"1","tv_archive":1,"tv_archive_duration":"3"},
            {"num":2,"name":"Sky Sports","stream_id":"102","stream_icon":"",
             "epg_channel_id":null,"category_id":"2"},
            {"num":3,"name":null,"stream_id":103,"category_id":null},
            {"num":4,"name":"No id"}
        ]"#;
        let channels = parse_live(categories, streams, &creds(), "ts").unwrap();
        assert_eq!(channels.len(), 3);
        assert_eq!(channels[0].name, "BBC News");
        assert_eq!(channels[0].group.as_deref(), Some("News"));
        assert_eq!(channels[0].logo.as_deref(), Some("http://x/bbc.png"));
        assert_eq!(channels[0].tvg_id.as_deref(), Some("bbcnews.uk"));
        assert!(channels[0].url.ends_with("/live/user/p%40ss%2F1/101.ts"));
        assert_eq!(channels[0].number, Some(1));
        assert_eq!(channels[0].catchup_days, Some(3));
        assert_eq!(channels[1].catchup_days, None);
        assert_eq!(channels[1].group.as_deref(), Some("Sports"));
        assert_eq!(channels[1].logo, None);
        assert_eq!(channels[2].name, "Stream 103");
        assert_eq!(channels[2].group, None);
    }

    #[test]
    fn media_urls_use_their_own_path_and_encode_the_extension() {
        assert_eq!(
            creds().movie_url("7", "mkv"),
            "http://example.com:8080/movie/user/p%40ss%2F1/7.mkv"
        );
        assert_eq!(
            creds().episode_url("9", "mp4"),
            "http://example.com:8080/series/user/p%40ss%2F1/9.mp4"
        );
        assert!(creds().movie_url("7", "../x").ends_with("/7...%2Fx"));
    }

    #[test]
    fn parses_movies_with_mixed_types() {
        let categories = r#"[{"category_id":"5","category_name":"Action"}]"#;
        let streams = r#"[
            {"num":1,"name":"Heat (1995)","stream_id":11,"stream_icon":"http://x/heat.jpg",
             "category_id":"5","container_extension":"mkv"},
            {"stream_id":"12","name":null,"category_id":9,"stream_icon":""},
            {"name":"No id"}
        ]"#;
        let movies = parse_movies(categories, streams, &creds()).unwrap();
        assert_eq!(movies.len(), 2);
        assert_eq!(movies[0].name, "Heat (1995)");
        assert_eq!(movies[0].group.as_deref(), Some("Action"));
        assert_eq!(movies[0].logo.as_deref(), Some("http://x/heat.jpg"));
        assert!(movies[0].url.ends_with("/movie/user/p%40ss%2F1/11.mkv"));
        assert_eq!(movies[1].name, "Movie 12");
        assert_eq!(movies[1].group, None);
        assert!(movies[1].url.ends_with("/12.mp4"));
        assert_eq!(movies[1].stream_format(), "file (VOD)");
    }

    #[test]
    fn parses_series_list() {
        let categories = r#"[{"category_id":3,"category_name":"Drama"}]"#;
        let list = r#"[
            {"series_id":21,"name":"The Wire","cover":"http://x/c.jpg","plot":"Baltimore.",
             "releaseDate":"2002-06-02","rating":"9.3","category_id":"3"},
            {"series_id":"22","name":"","rating":"0","release_date":"0000-00-00"},
            {"name":"No id"}
        ]"#;
        let series = parse_series(categories, list).unwrap();
        assert_eq!(series.len(), 2);
        assert_eq!(series[0].id, "21");
        assert_eq!(series[0].group.as_deref(), Some("Drama"));
        assert_eq!(series[0].year.as_deref(), Some("2002"));
        assert_eq!(series[0].rating.as_deref(), Some("9.3"));
        assert_eq!(series[0].plot.as_deref(), Some("Baltimore."));
        assert_eq!(series[1].name, "Series 22");
        assert_eq!(
            (series[1].year.as_deref(), series[1].rating.as_deref()),
            (None, None)
        );
    }

    #[test]
    fn parses_episodes_keyed_by_season_in_numeric_order() {
        let json = r#"{"info":{"name":"x"},"episodes":{
            "10":[{"id":"300","episode_num":1,"title":"Ten","container_extension":"mkv"}],
            "2":[{"id":"202","episode_num":"2","title":"Two","season":2},
                 {"id":"201","episode_num":1,"title":null,"season":2,
                  "info":{"movie_image":"http://x/e.jpg"}},
                 {"title":"no id"}]
        }}"#;
        let episodes = parse_episodes(json, &creds()).unwrap();
        let names: Vec<_> = episodes.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["S02E01", "S02E02  Two", "S10E01  Ten"]);
        assert_eq!(episodes[0].group.as_deref(), Some("Season 2"));
        assert_eq!(episodes[0].logo.as_deref(), Some("http://x/e.jpg"));
        assert!(episodes[0].url.ends_with("/series/user/p%40ss%2F1/201.mp4"));
        assert!(episodes[2].url.ends_with("/300.mkv"));
    }

    #[test]
    fn parses_episodes_given_as_array_of_arrays() {
        let json = r#"{"episodes":[[{"id":1,"episode_num":1}],[{"id":2,"episode_num":1}]]}"#;
        let episodes = parse_episodes(json, &creds()).unwrap();
        assert_eq!(episodes[0].group.as_deref(), Some("Season 1"));
        assert_eq!(episodes[1].group.as_deref(), Some("Season 2"));
    }

    #[test]
    fn empty_vod_responses_mean_no_items() {
        assert!(parse_movies("{}", "{}", &creds()).unwrap().is_empty());
        assert!(parse_series("[]", "null").unwrap().is_empty());
        assert!(
            parse_episodes(r#"{"episodes":[]}"#, &creds())
                .unwrap()
                .is_empty()
        );
        assert!(parse_episodes("{}", &creds()).unwrap().is_empty());
    }

    #[test]
    fn empty_object_means_no_items() {
        assert!(parse_live("{}", "{}", &creds(), "ts").unwrap().is_empty());
    }
}
