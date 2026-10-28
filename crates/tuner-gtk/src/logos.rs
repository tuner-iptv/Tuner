//! Channel logos: fetched once per URL on a worker thread, cached as
//! textures, and applied to whichever `Image` still wants them (list rows
//! are recycled while scrolling). Images have a fixed pixel size, so a huge
//! logo never changes the layout.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

/// Logos larger than this are ignored.
const MAX_BYTES: u64 = 2 * 1024 * 1024;

/// A fixed-size logo with crisp channel initials while artwork is unavailable.
pub struct Logo {
    pub widget: gtk::Overlay,
    image: gtk::Image,
    fallback: gtk::Label,
}

impl Logo {
    pub fn new(size: i32) -> Self {
        let image = gtk::Image::builder()
            .pixel_size(size)
            .width_request(size)
            .height_request(size)
            .build();
        let fallback = gtk::Label::new(Some("TV"));
        fallback.add_css_class("logo-initials");
        let widget = gtk::Overlay::new();
        widget.set_child(Some(&image));
        widget.add_overlay(&fallback);
        let weak = fallback.downgrade();
        image.connect_paintable_notify(move |image| {
            if let Some(fallback) = weak.upgrade() {
                fallback.set_visible(image.paintable().is_none());
            }
        });
        Self {
            widget,
            image,
            fallback,
        }
    }

    /// Reconstitute the handles when GTK binds a recycled channel row.
    pub fn from_widget(widget: gtk::Overlay) -> Self {
        let image = widget
            .child()
            .and_downcast::<gtk::Image>()
            .expect("logo image");
        let fallback = widget
            .last_child()
            .and_downcast::<gtk::Label>()
            .expect("logo initials");
        Self {
            widget,
            image,
            fallback,
        }
    }
}

fn initials(name: &str) -> String {
    let text: String = name
        .split_whitespace()
        .filter_map(|word| word.chars().find(|c| c.is_alphanumeric()))
        .take(2)
        .flat_map(char::to_uppercase)
        .take(2)
        .collect();
    if text.is_empty() { "TV".into() } else { text }
}

#[derive(Clone, Default)]
pub struct Logos {
    inner: Rc<RefCell<Inner>>,
}

#[derive(Default)]
struct Inner {
    /// `None` = fetch or decode failed; don't retry this session.
    cache: HashMap<String, Option<gdk::Texture>>,
    waiting: HashMap<String, Vec<glib::WeakRef<gtk::Image>>>,
}

impl Logos {
    /// Fetch artwork in the background, keeping channel initials for missing,
    /// loading or unavailable logos. Always reset both for recycled rows.
    pub fn show(&self, url: Option<&str>, name: &str, logo: &Logo) {
        let image = &logo.image;
        logo.fallback.set_text(&initials(name));
        image.set_paintable(None::<&gdk::Paintable>);
        let url = url.filter(|u| u.starts_with("http://") || u.starts_with("https://"));
        // The widget name records which logo this (possibly recycled) image
        // currently wants.
        image.set_widget_name(url.unwrap_or_default());
        let Some(url) = url else {
            return;
        };

        let mut inner = self.inner.borrow_mut();
        if let Some(cached) = inner.cache.get(url) {
            if let Some(texture) = cached {
                image.set_paintable(Some(texture));
            }
            return;
        }
        let waiters = inner.waiting.entry(url.to_string()).or_default();
        waiters.push(image.downgrade());
        if waiters.len() > 1 {
            return; // already being fetched
        }
        drop(inner);

        let this = self.clone();
        let url = url.to_string();
        glib::spawn_future_local(async move {
            let fetch_url = url.clone();
            let bytes = gio::spawn_blocking(move || fetch(&fetch_url))
                .await
                .ok()
                .flatten();
            let texture =
                bytes.and_then(|b| gdk::Texture::from_bytes(&glib::Bytes::from_owned(b)).ok());
            let waiters = {
                let mut inner = this.inner.borrow_mut();
                inner.cache.insert(url.clone(), texture.clone());
                inner.waiting.remove(&url).unwrap_or_default()
            };
            let Some(texture) = texture else {
                return;
            };
            for image in waiters.iter().filter_map(glib::WeakRef::upgrade) {
                if image.widget_name() == url {
                    image.set_paintable(Some(&texture));
                }
            }
        });
    }
}

fn fetch(url: &str) -> Option<Vec<u8>> {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    let agent = AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5))
            .timeout_read(Duration::from_secs(10))
            .user_agent(concat!("tuner/", env!("CARGO_PKG_VERSION")))
            .build()
    });
    let response = agent.get(url).call().ok()?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::initials;

    #[test]
    fn channel_initials_handle_unicode_punctuation_and_empty_names() {
        for (name, expected) in [
            ("Demo News HD", "DN"),
            ("Arts & Music", "AM"),
            ("Échos İstanbul", "Éİ"),
            ("  24 News  ", "2N"),
            ("Nature", "N"),
            ("— &", "TV"),
            ("", "TV"),
        ] {
            assert_eq!(initials(name), expected);
        }
    }
}
