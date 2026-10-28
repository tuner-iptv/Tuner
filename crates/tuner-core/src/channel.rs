/// A single playable stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    /// Display name.
    pub name: String,
    /// Stream URL handed to the player as-is.
    pub url: String,
    /// Category, from `group-title` or `#EXTGRP`.
    pub group: Option<String>,
    /// Logo URL, from `tvg-logo`.
    pub logo: Option<String>,
    /// EPG identifier, from `tvg-id`. Kept now so the EPG can join on it later.
    pub tvg_id: Option<String>,
    /// Channel number, from `tvg-chno` (M3U) or `num` (Xtream).
    pub number: Option<u32>,
    /// Days of catch-up (replay) the provider keeps, if any.
    pub catchup_days: Option<u32>,
}

impl Channel {
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            group: None,
            logo: None,
            tvg_id: None,
            number: None,
            catchup_days: None,
        }
    }

    /// Human-readable container guessed from the URL's extension.
    pub fn stream_format(&self) -> &'static str {
        let path = self.url.split(['?', '#']).next().unwrap_or(&self.url);
        let file = path.rsplit('/').next().unwrap_or(path);
        match file
            .rsplit_once('.')
            .map(|(_, ext)| ext.to_ascii_lowercase())
        {
            Some(ext) if ext == "ts" => "MPEG-TS",
            Some(ext) if ext == "m3u8" || ext == "m3u" => "HLS",
            Some(ext) if ext == "mp4" || ext == "mkv" => "file (VOD)",
            _ => "unknown",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guesses_stream_format() {
        let format = |url| Channel::new("x", url).stream_format();
        assert_eq!(format("http://h/live/u/p/1.ts"), "MPEG-TS");
        assert_eq!(format("http://h/a/index.M3U8?token=1"), "HLS");
        assert_eq!(format("http://h.example/stream"), "unknown");
    }
}
