//! Lenient M3U / M3U8 (extended M3U) playlist parser.
//!
//! Real-world IPTV playlists are messy, so the parser favours recovering over
//! rejecting: missing `#EXTM3U` headers, unknown directives, CRLF line endings,
//! a UTF-8 BOM and bare URL lists are all accepted. See `docs/m3u-format.md`.

use crate::Channel;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("playlist contains no channels")]
    Empty,
}

/// Parse an M3U playlist into channels, in file order.
pub fn parse(input: &str) -> Result<Vec<Channel>, ParseError> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let mut channels = Vec::new();
    let mut pending: Option<Channel> = None;
    let mut pending_group: Option<String> = None;

    for line in input.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            pending = Some(parse_extinf(rest));
            pending_group = None;
        } else if let Some(group) = line.strip_prefix("#EXTGRP:") {
            pending_group = non_empty(group.trim());
        } else if line.starts_with('#') {
            // #EXTM3U, #EXTVLCOPT, #KODIPROP, comments: not needed yet.
        } else {
            let mut channel = pending
                .take()
                .unwrap_or_else(|| Channel::new(String::new(), String::new()));
            channel.url = line.to_string();
            if channel.group.is_none() {
                channel.group = pending_group.take();
            }
            if channel.name.is_empty() {
                channel.name = line.to_string();
            }
            channels.push(channel);
        }
    }

    if channels.is_empty() {
        return Err(ParseError::Empty);
    }
    Ok(channels)
}

/// Parse the part of an `#EXTINF:` line after the colon, e.g.
/// `-1 tvg-id="bbc1" group-title="UK, News",BBC One`.
fn parse_extinf(rest: &str) -> Channel {
    let (info, name) = split_on_unquoted_comma(rest);
    let mut channel = Channel::new(name.trim(), String::new());
    let mut tvg_name = None;

    for (key, value) in parse_attrs(info) {
        let value = non_empty(&value);
        match key.as_str() {
            "group-title" => channel.group = value,
            "tvg-logo" => channel.logo = value,
            "tvg-id" => channel.tvg_id = value,
            "tvg-name" => tvg_name = value,
            "tvg-chno" => channel.number = value.and_then(|v| v.parse().ok()),
            "catchup-days" | "tvg-rec" => {
                channel.catchup_days = value.and_then(|v| v.parse().ok()).filter(|&d| d > 0)
            }
            _ => {}
        }
    }
    if channel.name.is_empty() {
        channel.name = tvg_name.unwrap_or_default();
    }
    channel
}

/// Split at the first comma outside double quotes. Attribute values such as
/// `group-title="UK, News"` may contain commas; the display name follows the
/// first comma that is not quoted.
fn split_on_unquoted_comma(s: &str) -> (&str, &str) {
    let mut in_quotes = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => return (&s[..i], &s[i + 1..]),
            _ => {}
        }
    }
    (s, "")
}

/// Extract `key=value` / `key="value"` pairs. Keys are lowercased. Tokens
/// without `=` (such as the leading duration `-1`) are skipped.
fn parse_attrs(info: &str) -> Vec<(String, String)> {
    let mut attrs = Vec::new();
    let mut rest = info;
    while let Some(eq) = rest.find('=') {
        let key = rest[..eq]
            .rsplit(char::is_whitespace)
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let after = &rest[eq + 1..];
        let (value, remaining) = match after.strip_prefix('"') {
            Some(quoted) => match quoted.find('"') {
                Some(end) => (&quoted[..end], &quoted[end + 1..]),
                None => (quoted, ""),
            },
            None => {
                let end = after.find(char::is_whitespace).unwrap_or(after.len());
                (&after[..end], &after[end..])
            }
        };
        if !key.is_empty() {
            attrs.push((key, value.to_string()));
        }
        rest = remaining;
    }
    attrs
}

fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_extended_playlist() {
        let input = "#EXTM3U\n\
            #EXTINF:-1 tvg-id=\"bbc1.uk\" tvg-chno=\"101\" catchup-days=\"7\" tvg-logo=\"http://x/logo.png\" group-title=\"UK, News\",BBC One\n\
            http://example.com/bbc1.m3u8\n\
            #EXTINF:-1 group-title=Sports,Sky Sports\n\
            http://example.com/sky\n";
        let channels = parse(input).unwrap();
        assert_eq!(channels.len(), 2);
        assert_eq!(
            channels[0],
            Channel {
                name: "BBC One".into(),
                url: "http://example.com/bbc1.m3u8".into(),
                group: Some("UK, News".into()),
                logo: Some("http://x/logo.png".into()),
                tvg_id: Some("bbc1.uk".into()),
                number: Some(101),
                catchup_days: Some(7),
            }
        );
        assert_eq!(channels[1].group.as_deref(), Some("Sports"));
    }

    #[test]
    fn accepts_crlf_bom_and_missing_header() {
        let input = "\u{feff}#EXTINF:-1,One\r\nhttp://a\r\n";
        let channels = parse(input).unwrap();
        assert_eq!(channels[0].name, "One");
        assert_eq!(channels[0].url, "http://a");
    }

    #[test]
    fn uses_extgrp_when_no_group_title() {
        let input = "#EXTINF:-1,One\n#EXTGRP:Movies\n#EXTVLCOPT:http-user-agent=x\nhttp://a\n";
        assert_eq!(parse(input).unwrap()[0].group.as_deref(), Some("Movies"));
    }

    #[test]
    fn falls_back_to_tvg_name_then_url() {
        let input = "#EXTINF:-1 tvg-name=\"Named\",\nhttp://a\nhttp://bare\n";
        let channels = parse(input).unwrap();
        assert_eq!(channels[0].name, "Named");
        assert_eq!(channels[1].name, "http://bare");
        assert_eq!(channels[1].group, None);
    }

    #[test]
    fn empty_playlist_is_an_error() {
        assert_eq!(parse("#EXTM3U\n\n"), Err(ParseError::Empty));
    }
}
