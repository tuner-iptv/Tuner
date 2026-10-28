use std::collections::HashMap;

use crate::{Channel, Series};

/// Something a [`Library`] can list, group and search by name.
pub trait Entry {
    fn name(&self) -> &str;
    fn group(&self) -> Option<&str>;
}

impl Entry for Channel {
    fn name(&self) -> &str {
        &self.name
    }
    fn group(&self) -> Option<&str> {
        self.group.as_deref()
    }
}

impl Entry for Series {
    fn name(&self) -> &str {
        &self.name
    }
    fn group(&self) -> Option<&str> {
        self.group.as_deref()
    }
}

/// All loaded entries (channels by default) plus derived indexes for browsing.
/// Entries can arrive in parts ([`Library::extend`]); indices already handed
/// out stay valid because entries are only ever appended.
#[derive(Debug, Clone)]
pub struct Library<T = Channel> {
    items: Vec<T>,
    groups: Vec<String>,
    /// Entries per group, aligned with `groups`.
    sizes: Vec<usize>,
    /// Group name → position in `groups`. Catalogues have tens of thousands
    /// of entries and hundreds of categories, so lookups must not scan.
    positions: HashMap<String, usize>,
}

impl<T> Default for Library<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            groups: Vec::new(),
            sizes: Vec::new(),
            positions: HashMap::new(),
        }
    }
}

impl<T: Entry> Library<T> {
    pub fn new(items: Vec<T>) -> Self {
        let mut library = Self::default();
        library.extend(items);
        library
    }

    /// Append entries, e.g. the next batch of a catalogue that is still
    /// downloading. Returns where the new entries start in
    /// [`Library::items`]. New groups go after the known ones.
    pub fn extend(&mut self, items: Vec<T>) -> usize {
        let start = self.items.len();
        for group in items.iter().filter_map(Entry::group) {
            match self.positions.get(group) {
                Some(&at) => self.sizes[at] += 1,
                None => {
                    self.positions.insert(group.to_string(), self.groups.len());
                    self.groups.push(group.to_string());
                    self.sizes.push(1);
                }
            }
        }
        self.items.extend(items);
        start
    }

    pub fn items(&self) -> &[T] {
        &self.items
    }

    /// Distinct groups in order of first appearance in the playlist.
    pub fn groups(&self) -> &[String] {
        &self.groups
    }

    /// Number of entries in each of [`Library::groups`], in the same order.
    pub fn group_sizes(&self) -> &[usize] {
        &self.sizes
    }

    /// Indices into [`Library::items`] matching `group` (if any) and whose
    /// name contains `query`, case-insensitively. Playlist order is kept.
    pub fn filter(&self, query: &str, group: Option<&str>) -> Vec<usize> {
        self.filter_from(0, query, group)
    }

    /// [`Library::filter`] over the entries from index `start` on, so a
    /// view can add just the matches of a newly appended batch.
    pub fn filter_from(&self, start: usize, query: &str, group: Option<&str>) -> Vec<usize> {
        let query = query.to_lowercase();
        self.items
            .iter()
            .enumerate()
            .skip(start)
            .filter(|(_, c)| group.is_none_or(|g| c.group() == Some(g)))
            .filter(|(_, c)| query.is_empty() || c.name().to_lowercase().contains(&query))
            .map(|(i, _)| i)
            .collect()
    }
}

impl Library<Channel> {
    pub fn channels(&self) -> &[Channel] {
        &self.items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library() -> Library {
        let mut news = Channel::new("BBC News", "a");
        news.group = Some("News".into());
        let mut sport = Channel::new("Sky Sports", "b");
        sport.group = Some("Sports".into());
        let mut news2 = Channel::new("CNN", "c");
        news2.group = Some("News".into());
        Library::new(vec![news, sport, news2, Channel::new("Ungrouped", "d")])
    }

    #[test]
    fn groups_are_unique_in_playlist_order() {
        assert_eq!(library().groups(), ["News", "Sports"]);
    }

    #[test]
    fn filters_by_group_and_case_insensitive_query() {
        let lib = library();
        assert_eq!(lib.filter("", None), [0, 1, 2, 3]);
        assert_eq!(lib.filter("", Some("News")), [0, 2]);
        assert_eq!(lib.filter("sky", None), [1]);
        assert_eq!(lib.filter("NEWS", Some("News")), [0]);
    }

    #[test]
    fn extending_keeps_indices_and_counts_groups() {
        let mut lib = library();
        assert_eq!(lib.group_sizes(), [2, 1]);
        let mut movie = Channel::new("BBC Films", "e");
        movie.group = Some("Movies".into());
        let mut more_news = Channel::new("Al Jazeera", "f");
        more_news.group = Some("News".into());
        assert_eq!(lib.extend(vec![movie, more_news]), 4);
        assert_eq!(lib.groups(), ["News", "Sports", "Movies"]);
        assert_eq!(lib.group_sizes(), [3, 1, 1]);
        assert_eq!(lib.filter("", Some("News")), [0, 2, 5]);
        assert_eq!(lib.filter_from(4, "bbc", None), [4]);
        assert_eq!(lib.filter_from(4, "", Some("News")), [5]);
    }

    #[test]
    fn libraries_can_hold_series() {
        let show = |name: &str, group: &str| Series {
            id: name.into(),
            name: name.into(),
            group: Some(group.into()),
            cover: None,
            plot: None,
            year: None,
            rating: None,
        };
        let lib = Library::new(vec![show("The Wire", "Drama"), show("Friends", "Comedy")]);
        assert_eq!(lib.groups(), ["Drama", "Comedy"]);
        assert_eq!(lib.filter("wire", Some("Drama")), [0]);
        assert_eq!(lib.items()[1].id, "Friends");
    }
}
