/// A series in the catalogue. Not playable itself: its episodes (fetched on
/// demand, see `xtream::parse_episodes`) are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Series {
    /// Provider id, used to fetch the episodes.
    pub id: String,
    pub name: String,
    /// Category.
    pub group: Option<String>,
    /// Cover image URL.
    pub cover: Option<String>,
    pub plot: Option<String>,
    /// First-release year.
    pub year: Option<String>,
    /// Provider rating as shown by the panel, e.g. `8.4`.
    pub rating: Option<String>,
}
