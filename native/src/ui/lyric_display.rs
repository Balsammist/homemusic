use super::{details, *};
use gtk::subclass::prelude::*;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

mod visual {
    use super::*;

    #[derive(Default)]
    pub struct LyricVisual(pub Cell<f64>);

    #[glib::object_subclass]
    impl ObjectSubclass for LyricVisual {
        const NAME: &'static str = "HomeMusicLyricVisual";
        type Type = super::LyricVisual;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for LyricVisual {}
    impl BoxImpl for LyricVisual {}
    impl WidgetImpl for LyricVisual {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let blur = self.0.get();
            if blur > 0. {
                snapshot.push_blur(blur);
                self.parent_snapshot(snapshot);
                snapshot.pop();
            } else {
                self.parent_snapshot(snapshot);
            }
        }
    }
}

glib::wrapper! {
    pub struct LyricVisual(ObjectSubclass<visual::LyricVisual>)
        @extends gtk::Widget, gtk::Box,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl LyricVisual {
    pub(super) fn new() -> Self {
        glib::Object::builder()
            .property("orientation", gtk::Orientation::Vertical)
            .property("spacing", 2)
            .build()
    }

    pub(super) fn set_blur(&self, radius: f64) {
        if self.imp().0.replace(radius) != radius {
            self.queue_draw();
        }
    }
}

pub(super) struct NowLyrics {
    button: gtk::Button,
    label: gtk::Label,
    enabled: Cell<bool>,
    generation: Arc<AtomicU64>,
    prefetch: RefCell<Option<(String, Arc<details::LyricPrefetch>)>>,
    lines: RefCell<Vec<details::LyricLine>>,
    visible_line: Cell<Option<usize>>,
    glyph_ends: RefCell<Vec<usize>>,
    paint: Cell<Option<(usize, u8)>>,
    offset: Cell<f64>,
}

impl NowLyrics {
    pub(super) fn new() -> Self {
        let button = gtk::Button::new();
        button.add_css_class("now-lyric-preview");
        button.set_widget_name("now-lyric-preview");
        button.update_property(&[gtk::accessible::Property::Label(
            "Открыть полный текст песни",
        )]);
        button.set_tooltip_text(Some("Открыть полный текст песни"));
        let label = label("", "now-lyric-line");
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_lines(3);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        button.set_child(Some(&label));
        button.set_visible(false);
        Self {
            button,
            label,
            enabled: Cell::new(false),
            generation: Arc::new(AtomicU64::new(0)),
            prefetch: RefCell::default(),
            lines: RefCell::default(),
            visible_line: Cell::new(None),
            glyph_ends: RefCell::default(),
            paint: Cell::new(None),
            offset: Cell::new(0.),
        }
    }

    pub(super) fn widget(&self) -> &gtk::Button {
        &self.button
    }

    pub(super) fn configure(&self, ui: &Ui, settings: &Value) {
        self.offset.set(
            settings["lyrics_offset"]
                .as_f64()
                .unwrap_or(0.)
                .clamp(-30., 30.),
        );
        let enabled = settings["lyrics_live_in_player"].as_bool().unwrap_or(false);
        if enabled != self.enabled.replace(enabled) {
            self.load(ui, ui.current.borrow().as_ref());
        }
    }

    pub(super) fn prefetch(&self, track: &Track) -> Option<Arc<details::LyricPrefetch>> {
        self.prefetch
            .borrow()
            .as_ref()
            .filter(|(reference, _)| *reference == track.reference)
            .map(|(_, prefetch)| prefetch.clone())
    }

    pub(super) fn load(&self, ui: &Ui, track: Option<&Track>) {
        self.prefetch.borrow_mut().take();
        self.lines.borrow_mut().clear();
        self.visible_line.set(None);
        self.button.set_visible(false);
        let generation = self
            .generation
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        if self.enabled.get()
            && let Some(track) = track.cloned()
        {
            let tx = ui.tx.clone();
            let latest = self.generation.clone();
            let prefetch = Arc::new(details::LyricPrefetch::new());
            self.prefetch.replace(Some((track.reference.clone(), prefetch.clone())));
            let job = move || {
                if latest.load(Ordering::Relaxed) != generation {
                    return;
                }
                let result = prefetch.get_or_init(|| {
                    details::fetch_lyrics(&details::external_agent(), &track)
                });
                let lines = match result {
                    Ok(found) => match &found.lyrics {
                        details::Lyrics::Synced(lines) => lines.clone(),
                        _ => Vec::new(),
                    },
                    Err(_) => Vec::new(),
                };
                let _ = tx.send_blocking(Event::NowLyrics(generation, lines));
            };
            ui.net.run(job);
        }
    }

    pub(super) fn loaded(&self, generation: u64, lines: Vec<details::LyricLine>, position: f64) {
        if self.generation.load(Ordering::Relaxed) == generation {
            *self.lines.borrow_mut() = lines;
            self.update(position);
        }
    }

    pub(super) fn update(&self, position: f64) {
        let lines = self.lines.borrow();
        let position = position + self.offset.get();
        let Some(i) = preview_line(&lines, position) else {
            return;
        };
        let line = &lines[i];
        if self.visible_line.replace(Some(i)) != Some(i) {
            self.label.set_text(&line.words);
            self.glyph_ends.replace(line.glyph_ends());
            self.paint.set(None);
            self.button.set_visible(true);
        }
        let ends = self.glyph_ends.borrow();
        let next = lines.get(i + 1).map(|line| line.seconds);
        let smooth = self.button.settings().is_gtk_enable_animations();
        let (count, step) = line.progress(&ends, position, next, smooth);
        if self.paint.replace(Some((count, step))) != Some((count, step)) {
            details::paint_highlight(&self.label, &ends, count, f64::from(step) / 16.);
        }
    }
}

fn preview_line(lines: &[details::LyricLine], position: f64) -> Option<usize> {
    let split = lines.partition_point(|line| line.seconds <= position);
    let has_words = |line: &details::LyricLine| !line.words.trim().is_empty();
    lines[..split]
        .iter()
        .rposition(has_words)
        .or_else(|| lines[split..].iter().position(has_words).map(|i| split + i))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_playing_lyric_preview_holds_the_last_nonempty_line_in_gaps() {
        let lines = details::parse_lrc(
            "[00:01]\n[00:05]<00:05>First<00:06>\n[00:09]\n[00:12]Next",
        );
        assert_eq!(preview_line(&lines, 0.), Some(1));
        assert_eq!(preview_line(&lines, 8.), Some(1));
        assert_eq!(preview_line(&lines, 10.), Some(1));
        assert_eq!(preview_line(&lines, 12.), Some(3));
        assert_eq!(preview_line(&lines, 20.), Some(3));
        assert_eq!(preview_line(&lines[..1], 20.), None);
    }
}
