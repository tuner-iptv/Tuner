//! Reading a large JSON array incrementally. Xtream catalogues are one array
//! of tens of thousands of entries (often tens of megabytes); waiting for the
//! whole body before showing anything makes the UI sit on a spinner. Instead
//! the elements are parsed as the bytes arrive and handed on in batches.

use std::fmt;
use std::io::{BufReader, Read};
use std::ops::ControlFlow;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use serde::Deserializer as _;
use serde::de::{self, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::Value;

/// A batch is handed on once it is this old, so the first entries show up
/// quickly however slow the connection is…
const FLUSH_AFTER: Duration = Duration::from_millis(150);
/// …or this large, so a fast connection doesn't build one huge batch.
const MAX_BATCH: usize = 5_000;

/// What a batch consumer says after each batch: `Break` stops reading, e.g.
/// because the user switched account and nobody wants the rest.
pub type Flow = ControlFlow<()>;

/// Parse the JSON array in `reader`, calling `on_batch` with consecutive
/// runs of its elements. `{}` and `null` (what some panels send for "no
/// entries") yield nothing. Returns `Ok` early if `on_batch` breaks.
pub(crate) fn read_array(
    reader: impl Read,
    on_batch: &mut dyn FnMut(Vec<Value>) -> Flow,
) -> Result<()> {
    let mut de = serde_json::Deserializer::from_reader(BufReader::new(reader));
    let mut stopped = false;
    let result = de.deserialize_any(Elements {
        on_batch,
        stopped: &mut stopped,
    });
    if stopped {
        return Ok(());
    }
    result
        .and_then(|()| de.end())
        .map_err(|e| anyhow!("invalid JSON from server: {e}"))
}

struct Elements<'a> {
    on_batch: &'a mut dyn FnMut(Vec<Value>) -> Flow,
    stopped: &'a mut bool,
}

impl<'de> Visitor<'de> for Elements<'_> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a JSON array")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        let mut batch = Vec::new();
        let mut since = Instant::now();
        while let Some(element) = seq.next_element::<Value>()? {
            batch.push(element);
            if batch.len() >= MAX_BATCH || since.elapsed() >= FLUSH_AFTER {
                if (self.on_batch)(std::mem::take(&mut batch)).is_break() {
                    // Abandon the rest of the body; `read_array` reports Ok.
                    *self.stopped = true;
                    return Err(de::Error::custom("stopped"));
                }
                since = Instant::now();
            }
        }
        if !batch.is_empty() && (self.on_batch)(batch).is_break() {
            *self.stopped = true;
        }
        Ok(())
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(())
    }

    fn visit_unit<E: de::Error>(self) -> Result<(), E> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(json: &str) -> Result<Vec<Value>> {
        let mut all = Vec::new();
        read_array(json.as_bytes(), &mut |batch| {
            all.extend(batch);
            Flow::Continue(())
        })?;
        Ok(all)
    }

    #[test]
    fn reads_every_element_in_order() {
        let all = collect(r#"[{"id":1},{"id":"2"}, 3 ]"#).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[1]["id"], "2");
    }

    #[test]
    fn empty_answers_are_lenient() {
        assert!(collect("[]").unwrap().is_empty());
        assert!(collect(r#"{}"#).unwrap().is_empty());
        assert!(collect(r#"{"a":1}"#).unwrap().is_empty());
        assert!(collect("null").unwrap().is_empty());
    }

    #[test]
    fn broken_json_is_an_error() {
        let err = collect(r#"[{"id":1},"#).unwrap_err();
        assert!(err.to_string().contains("invalid JSON"));
        assert!(collect(r#"[1] x"#).is_err());
        assert!(collect(r#""text""#).is_err());
    }

    #[test]
    fn large_arrays_come_in_several_batches() {
        let json = format!("[{}]", vec!["{}"; MAX_BATCH * 2 + 1].join(","));
        let mut sizes = Vec::new();
        read_array(json.as_bytes(), &mut |batch| {
            sizes.push(batch.len());
            Flow::Continue(())
        })
        .unwrap();
        assert_eq!(sizes.iter().sum::<usize>(), MAX_BATCH * 2 + 1);
        assert!(sizes.len() >= 3);
    }

    #[test]
    fn breaking_stops_reading_without_an_error() {
        let json = format!("[{}", vec!["{}"; MAX_BATCH + 1].join(","));
        let mut batches = 0;
        // Truncated on purpose: reading must stop before reaching the end.
        read_array(json.as_bytes(), &mut |_| {
            batches += 1;
            Flow::Break(())
        })
        .unwrap();
        assert_eq!(batches, 1);
    }
}
