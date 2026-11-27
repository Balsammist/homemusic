//! Track details fetched on a worker; GTK widgets stay on the main context.
use crate::api::{Api, Track};
use gtk::{glib, prelude::*};
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::OnceLock,
    time::Duration,
};

const CLIENT: &str = "HomeMusic-Native/0.1 (https://github.com/justman-1/HomeMusic-public)";

thread_local! {
    static TEST_STATE: RefCell<Value> = const { RefCell::new(Value::Null) };
    static TEST_GENERATION: Cell<u64> = const { Cell::new(0) };
}

/// Minimal observable state for the existing local test socket; never includes lyric text.
pub(super) fn test_state() -> Value {
    TEST_STATE.with(|state| state.borrow().clone())
}

fn update_test_state(generation: u64, update: impl FnOnce(&mut Value)) {
    if generation != 0 && TEST_GENERATION.with(|current| current.get() == generation) {
        TEST_STATE.with(|state| update(&mut state.borrow_mut()));
    }
}

enum Event {
    Lyrics(Result<super::lyric_sources::Match, String>, Vec<String>),
    Translation(u64, Result<Vec<String>, String>),
    Info(Result<(String, Option<(String, String)>), String>),
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct LyricLine {
    pub(super) seconds: f64,
    pub(super) words: String,
    #[serde(default)]
    pub(super) timing: Vec<LyricWord>,
    #[serde(default)]
    pub(super) alternate: bool,
    #[serde(default)]
    pub(super) end: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct LyricWord {
    pub(super) begin: f64,
    pub(super) end: Option<f64>,
    pub(super) byte_end: usize,
}

#[derive(Clone)]
pub(super) enum Lyrics {
    Synced(Vec<LyricLine>),
    Plain(String),
    Notice(String),
}

pub(super) type LyricPrefetch = OnceLock<Result<super::lyric_sources::Match, String>>;

struct LyricRow {
    line: LyricLine,
    label: gtk::Label,
    subtitle: Option<gtk::Label>,
    translation: gtk::Label,
    container: super::lyric_display::LyricVisual,
    glyph_ends: Vec<usize>,
}

impl LyricLine {
    pub(super) fn glyph_ends(&self) -> Vec<usize> {
        self.words
            .char_indices()
            .map(|(i, c)| i + c.len_utf8())
            .collect()
    }

    pub(super) fn progress(
        &self,
        glyph_ends: &[usize],
        position: f64,
        next: Option<f64>,
        smooth: bool,
    ) -> (usize, u8) {
        let reveal = |start: usize, end: usize, begin: f64, finish: f64| {
            let phase = if position >= finish {
                1.
            } else if position <= begin {
                0.
            } else {
                (position - begin) / (finish - begin)
            };
            let glyphs = start as f64 + end.saturating_sub(start) as f64 * phase;
            let steps = (glyphs * 16.).round() as usize;
            (steps / 16, (steps % 16) as u8)
        };
        let glyphs = glyph_ends.len();
        let stated_end = self.end.filter(|end| *end > self.seconds);
        let line_end = match (stated_end, next.filter(|next| *next > self.seconds)) {
            (Some(end), Some(next)) => Some(end.min(next)),
            (end, next) => end.or(next),
        };
        if self.timing.is_empty() {
            if smooth {
                return reveal(0, glyphs, self.seconds, line_end.unwrap_or(self.seconds + 4.));
            }
            return (glyphs, 0);
        }
        let word = self.timing.partition_point(|word| word.begin <= position);
        let Some(index) = word.checked_sub(1) else {
            return (0, 0);
        };
        let current = &self.timing[index];
        let following = self.timing.get(word);
        let end = following.map_or(glyphs, |_| {
            glyph_ends.partition_point(|end| *end <= current.byte_end)
        });
        if !smooth {
            return (end, 0);
        }
        let previous = index.checked_sub(1).map(|i| &self.timing[i]);
        let start = glyph_ends.partition_point(|end| *end <= previous.map_or(0, |w| w.byte_end));
        let inferred_finish = current.begin
            + previous.map_or(0.7, |word| {
                (word.end.unwrap_or(current.begin) - word.begin).clamp(0.2, 1.2)
            });
        let deadline = following.map_or(f64::INFINITY, |word| word.begin)
            .min(line_end.unwrap_or(f64::INFINITY));
        let finish = current
            .end
            .filter(|end| *end > current.begin)
            .or_else(|| following.map(|word| word.begin))
            .or(stated_end)
            .unwrap_or(inferred_finish)
            .min(deadline);
        reveal(start, end, current.begin, finish)
    }
}

pub(super) fn paint_highlight(
    label: &gtk::Label,
    glyph_ends: &[usize],
    count: usize,
    fraction: f64,
) {
    let attributes = gtk::pango::AttrList::new();
    let end = |i: usize| glyph_ends[i] as u32;
    let alpha = |from: u32, to: u32, opacity: f64| {
        let mut attr = gtk::pango::AttrInt::new_foreground_alpha((opacity * 65535.).round() as u16);
        attr.set_start_index(from);
        attr.set_end_index(to);
        attributes.insert(attr);
    };
    if count < glyph_ends.len() {
        let start = count.checked_sub(1).map_or(0, end);
        let partial = end(count);
        alpha(start, partial, 0.48 + 0.52 * fraction);
        alpha(partial, u32::MAX, 0.48);
    }
    label.set_attributes(Some(&attributes));
}

pub(super) fn paint_lyric_sample(
    label: &gtk::Label,
    frame: i64,
    origin: &Cell<Option<i64>>,
    smooth: bool,
) {
    let start = origin.get().unwrap_or_else(|| {
        origin.set(Some(frame));
        frame
    });
    let text = label.text();
    let ends: Vec<_> = text.char_indices().map(|(i, c)| i + c.len_utf8()).collect();
    let phase = (frame - start).rem_euclid(3_600_000) as f64 / 2_800_000.;
    let reveal = phase.min(1.) * ends.len() as f64;
    if smooth {
        paint_highlight(label, &ends, reveal.floor() as usize, reveal.fract());
    } else {
        let last = ends
            .iter()
            .take(reveal.ceil() as usize)
            .enumerate()
            .filter(|&(_, &end)| text[..end].ends_with(' '))
            .last();
        let count = if phase >= 1. {
            ends.len()
        } else {
            last.map_or(0, |(i, _)| i + 1)
        };
        paint_highlight(label, &ends, count, 0.);
    }
}

pub fn show(
    parent: &gtk::ApplicationWindow,
    _api: Api,
    track: Track,
    position: impl Fn() -> f64 + 'static,
    seek: impl Fn(f64) + 'static,
    audio_source: Option<(String, Rc<dyn Fn()>)>,
    info_first: bool,
    navigation: Option<(Rc<dyn Fn()>, Rc<dyn Fn()>)>,
    prefetch: Option<std::sync::Arc<LyricPrefetch>>,
) {
    let artist_only = track.title.trim().is_empty()
        && track
            .album_title
            .as_deref()
            .is_none_or(|s| s.trim().is_empty());
    let album_only = track.title.trim().is_empty() && !artist_only;
    let heading_label = if artist_only {
        "Об исполнителе"
    } else if album_only {
        "Об альбоме"
    } else {
        "О треке"
    };
    let test_generation = if std::env::var_os("HOMEMUSIC_TEST").is_some() {
        let generation = TEST_GENERATION.with(|counter| {
            let next = counter.get().wrapping_add(1).max(1);
            counter.set(next);
            next
        });
        TEST_STATE.with(|state| *state.borrow_mut() = serde_json::json!({
            "title": if artist_only { &track.artist } else if album_only { track.album_title.as_deref().unwrap_or("") } else { &track.title },
            "synced": false, "line_count": 0, "active_index": null, "position": null
        }));
        generation
    } else {
        0
    };
    let window = gtk::Window::builder()
        .title(heading_label)
        .transient_for(parent)
        .modal(false)
        .default_width(720)
        .default_height(640)
        .build();
    window.set_decorated(false);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 16);
    root.set_margin_top(24);
    root.set_margin_bottom(24);
    root.set_margin_start(24);
    root.set_margin_end(24);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let header_title = text(heading_label);
    header_title.add_css_class("overline");
    header_title.set_selectable(false);
    header_title.set_hexpand(true);
    header.append(&header_title);
    let close = super::icon_button("window-close-symbolic", "Закрыть", "flat");
    let close_window = window.downgrade();
    close.connect_clicked(move |_| {
        if let Some(window) = close_window.upgrade() {
            window.close();
        }
    });
    header.append(&close);
    let fullscreen = super::icon_button("view-fullscreen-symbolic", "Полноэкранный текст", "flat");
    fullscreen.set_visible(!artist_only && !album_only);
    header.insert_child_after(&fullscreen, Some(&header_title));
    let fullscreen_window = window.downgrade();
    fullscreen.connect_clicked(move |_| {
        if let Some(window) = fullscreen_window.upgrade() {
            if window.is_fullscreen() {
                window.unfullscreen();
            } else {
                window.fullscreen();
            }
        }
    });
    let handle = gtk::WindowHandle::new();
    handle.set_child(Some(&header));
    root.append(&handle);
    let heading = text(if artist_only {
        &track.artist
    } else if album_only {
        track.album_title.as_deref().unwrap_or("Альбом")
    } else {
        &track.title
    });
    heading.add_css_class("title-1");
    root.append(&heading);
    let byline = text(&track.artist);
    byline.add_css_class("dim-label");
    if let Some((artist, album)) = navigation {
        if !track.title.is_empty() {
            heading.set_selectable(false);
            heading.set_focusable(true);
            heading.set_can_target(true);
            heading.add_css_class("media-link");
            heading.set_cursor_from_name(Some("pointer"));
            let callback = album.clone();
            let dialog = window.downgrade();
            let click = gtk::GestureClick::new();
            click.set_button(1);
            click.connect_released(move |_, n, _, _| {
                if n == 1 {
                    if let Some(dialog) = dialog.upgrade() {
                        dialog.close();
                        callback();
                    }
                }
            });
            heading.add_controller(click);
            let callback = album.clone();
            let dialog = window.downgrade();
            let keys = gtk::EventControllerKey::new();
            keys.connect_key_pressed(move |_, key, _, _| {
                if matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::space) {
                    if let Some(dialog) = dialog.upgrade() {
                        dialog.close();
                        callback();
                    }
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            });
            heading.add_controller(keys);
        }
        let links = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        for (title, callback) in [
            (track.artist.clone(), artist),
            (track.album_title.clone().unwrap_or_default(), album),
        ] {
            if title.is_empty() {
                continue;
            }
            let link = super::button(&title, None, "flat media-link");
            let dialog = window.downgrade();
            link.connect_clicked(move |_| {
                if let Some(dialog) = dialog.upgrade() {
                    dialog.close();
                    callback();
                }
            });
            links.append(&link);
        }
        root.append(&links);
    } else {
        root.append(&byline);
    }
    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    stack.set_transition_type(
        if gtk::Settings::default().is_some_and(|settings| settings.is_gtk_enable_animations()) {
            gtk::StackTransitionType::Crossfade
        } else {
            gtk::StackTransitionType::None
        },
    );
    stack.set_transition_duration(180);
    let switcher = gtk::StackSwitcher::new();
    switcher.set_stack(Some(&stack));
    switcher.add_css_class("panel-tabs");
    switcher.set_halign(gtk::Align::Start);
    root.append(&switcher);
    root.append(&stack);
    window.set_child(Some(&super::ripple::wrap(&root)));
    super::autoscroll::install(&window);

    let lyrics = text("Ищем текст песни…");
    lyrics.add_css_class("title-3");
    let appearance = super::settings::LyricsAppearance::current();
    let follow = appearance.follow;
    let offset = appearance.offset_seconds;
    let lyrics_box = gtk::Box::new(gtk::Orientation::Vertical, 18);
    appearance.decorate(&lyrics_box);
    lyrics_box.append(&lyrics);
    let plain_lyrics = text("");
    plain_lyrics.add_css_class("plain-lyrics");
    appearance.format_label(&plain_lyrics);
    plain_lyrics.set_visible(false);
    lyrics_box.append(&plain_lyrics);
    let plain_translation = text("");
    plain_translation.add_css_class("lyrics-subtitle");
    appearance.format_label(&plain_translation);
    plain_translation.set_visible(false);
    lyrics_box.append(&plain_translation);
    let source = gtk::LinkButton::with_label("", "");
    source.set_visible(false);
    let status = lyrics.downgrade();
    let fullscreen_control = fullscreen.downgrade();
    window.connect_fullscreened_notify(move |window| {
        let active = window.is_fullscreen();
        if let Some(status) = status.upgrade() {
            status.set_visible(!(appearance.hide_status && active));
        }
        if let Some(button) = fullscreen_control.upgrade() {
            let (icon, title) = if active {
                ("view-restore-symbolic", "Выйти из полноэкранного режима")
            } else {
                ("view-fullscreen-symbolic", "Полноэкранный текст")
            };
            super::set_md_icon(&button, icon);
            super::set_action_label(&button, title);
        }
    });
    source.set_halign(gtk::Align::Start);
    lyrics_box.append(&source);
    let lines_box = gtk::Box::new(gtk::Orientation::Vertical, appearance.spacing());
    lyrics_box.insert_child_after(&lines_box, Some(&lyrics));
    let lyrics_scroll = scroll(&lyrics_box);
    if !album_only && !artist_only {
        stack.add_titled(&lyrics_scroll, Some("lyrics"), "Текст песни");
    }
    let synced: Rc<RefCell<Vec<LyricRow>>> = Rc::default();
    let active = Rc::new(Cell::new(None::<usize>));
    let closed = Rc::new(Cell::new(false));
    let timer_lines = synced.clone();
    let timer_active = active.clone();
    let timer_closed = closed.clone();
    let timer_box = lines_box.clone();
    let timer_container = lyrics_box.clone();
    let scroll_animation = Cell::new(None::<(i64, f64, f64)>);
    let previous_paint = Cell::new(None);
    lyrics_scroll.add_tick_callback(move |timer_scroll, clock| {
        if timer_closed.get() {
            return glib::ControlFlow::Break;
        }
        let lines = timer_lines.borrow();
        if lines.is_empty() {
            return glib::ControlFlow::Continue;
        }
        let seconds = position();
        let now = seconds + offset;
        let current = if seconds.is_finite() && seconds >= 0.0 {
            active_line(&lines, now, appearance.real_timing)
        } else {
            None
        };
        let changed = timer_active.replace(current) != current;
        let motion =
            gtk::Settings::default().is_some_and(|settings| settings.is_gtk_enable_animations());
        let frame = clock.frame_time();
        if test_generation != 0 {
            update_test_state(test_generation, |state| {
                state["active_index"] = serde_json::json!(current);
                state["position"] = serde_json::json!(seconds.is_finite().then_some(seconds));
            });
        }
        if changed {
            previous_paint.set(None);
            let mut max_blur = 0.0f64;
            for (i, row) in lines.iter().enumerate() {
                let is_current = Some(i) == current;
                for class in ["karaoke-past", "karaoke-active", "karaoke-future"] {
                    row.label.remove_css_class(class);
                }
                row.label.add_css_class(if is_current {
                    "karaoke-active"
                } else if row.line.seconds <= now {
                    "karaoke-past"
                } else {
                    "karaoke-future"
                });
                appearance.format_line(&row.label, is_current, row.line.alternate);
                appearance.format_line(&row.translation, is_current, row.line.alternate);
                if let Some(subtitle) = &row.subtitle {
                    appearance.format_line(subtitle, is_current, row.line.alternate);
                }
                let blur = if appearance.soft_focus {
                    current.map_or(0., |at| {
                        (at.abs_diff(i).saturating_sub(1) as f64 * 0.6).min(2.4)
                    })
                } else {
                    0.
                };
                row.container.set_blur(blur);
                max_blur = max_blur.max(blur);
                row.label.set_attributes(None);
            }
            update_test_state(test_generation, |state| {
                state["max_blur"] = serde_json::json!(max_blur)
            });
            if follow
                && let Some(i) = current
                && let Some(bounds) = lines[i].container.compute_bounds(&timer_box)
            {
                let adjustment = timer_scroll.vadjustment();
                let top = timer_box
                    .compute_bounds(&timer_container)
                    .map(|b| f64::from(b.y()))
                    .unwrap_or(0.)
                    + f64::from(bounds.y());
                let target = (top - adjustment.page_size() * 0.35).clamp(
                    adjustment.lower(),
                    (adjustment.upper() - adjustment.page_size()).max(adjustment.lower()),
                );
                if motion {
                    scroll_animation.set(Some((frame, adjustment.value(), target)));
                } else {
                    adjustment.set_value(target);
                }
            }
        }
        if let Some((start, at, target)) = scroll_animation.get() {
            let t = if motion {
                ((frame - start) as f64 / 260_000.).clamp(0., 1.)
            } else {
                1.
            };
            timer_scroll
                .vadjustment()
                .set_value(at + (target - at) * (1. - (1. - t).powi(3)));
            if t >= 1. {
                scroll_animation.set(None);
            }
        }
        if appearance.animation != 0
            && let Some(i) = current
        {
            let row = &lines[i];
            let next = lines.get(i + 1).map(|next| next.line.seconds);
            let (count, step) = row.line.progress(
                &row.glyph_ends,
                now,
                next,
                appearance.animation == 2 && motion,
            );
            let state = (i, count, step);
            if previous_paint.replace(Some(state)) != Some(state) {
                paint_highlight(&row.label, &row.glyph_ends, count, f64::from(step) / 16.);
                update_test_state(test_generation, |test| {
                    test["highlight_glyph"] = serde_json::json!(count);
                    test["highlight_step"] = serde_json::json!(step);
                });
            }
        }
        glib::ControlFlow::Continue
    });

    let info_box = gtk::Box::new(gtk::Orientation::Vertical, 18);
    if let Some((description, action)) = audio_source {
        info_box.append(&text(&description));
        let b = super::button(
            "Источники аудио",
            Some("audio-volume-high-symbolic"),
            "flat",
        );
        b.connect_clicked(move |_| action());
        info_box.append(&b);
    }
    let metadata = text(&track_metadata(&track));
    info_box.append(&metadata);
    let info = text("Ищем статью в русской Wikipedia…");
    info_box.append(&info);
    let attribution = gtk::LinkButton::with_label("https://ru.wikipedia.org", "Wikipedia");
    attribution.set_halign(gtk::Align::Start);
    attribution.set_visible(false);
    info_box.append(&attribution);
    stack.add_titled(&scroll(&info_box), Some("info"), "Информация");
    if info_first || album_only || artist_only {
        stack.set_visible_child_name("info");
    }

    let (tx, rx) = async_channel::bounded(4);
    let translation_mode = super::read_pref("lyrics_translation").as_u64().unwrap_or(0);
    let translate_button = super::button(
        "Перевести через DeepL",
        Some("preferences-desktop-locale-symbolic"),
        "tonal",
    );
    translate_button.set_visible(false);
    translate_button.set_halign(gtk::Align::Start);
    lyrics_box.insert_child_after(&translate_button, Some(&source));
    let translation_status = text("");
    translation_status.add_css_class("lyrics-subtitle");
    translation_status.set_visible(false);
    lyrics_box.insert_child_after(&translation_status, Some(&translate_button));
    let translation_id = Rc::new(Cell::new(0u64));
    let original_lines = Rc::new(RefCell::new(Vec::<String>::new()));
    let translation_sender = tx.clone();
    let translation_counter = translation_id.clone();
    let input = original_lines.clone();
    let status_label = translation_status.clone();
    translate_button.connect_clicked(move |button| {
        let lines = input.borrow().clone();
        if lines.is_empty() {
            return;
        }
        let id = translation_counter.get().wrapping_add(1);
        translation_counter.set(id);
        button.set_sensitive(false);
        status_label.set_text("Переводим текст…");
        status_label.set_visible(true);
        let tx = translation_sender.clone();
        std::thread::spawn(move || {
            let result = super::lyric_translation::translate(&external_agent(), &lines);
            let _ = tx.send_blocking(Event::Translation(id, result));
        });
    });
    let translate = translate_button.clone();
    let close_rx = rx.clone();
    window.connect_close_request(move |_| {
        closed.set(true);
        close_rx.close();
        glib::Propagation::Proceed
    });
    let weak = window.downgrade();
    let seek: Rc<dyn Fn(f64)> = Rc::new(seek);
    glib::MainContext::default().spawn_local(async move {
        while let Ok(event) = rx.recv().await {
            if weak.upgrade().is_none() {
                break;
            }
            match event {
                Event::Lyrics(result, versions) => {
                    while let Some(child) = lines_box.first_child() {
                        lines_box.remove(&child);
                    }
                    synced.borrow_mut().clear();
                    plain_lyrics.set_visible(false);
                    plain_translation.set_visible(false);
                    translate.set_visible(false);
                    translate.set_sensitive(true);
                    translation_status.set_visible(false);
                    translation_id.set(translation_id.get().wrapping_add(1));
                    original_lines.borrow_mut().clear();
                    active.set(None);
                    source.set_visible(false);
                    let result = result.map(|found| {
                        if let Some(i) = found.source {
                            let (name, url, _) = super::lyric_sources::PROVIDERS[i];
                            source.set_uri(url);
                            source.set_label(&format!("Источник текста: {name}"));
                            source.set_visible(true);
                        }
                        found.lyrics
                    });
                    match result {
                        Ok(Lyrics::Synced(lines)) => {
                            update_test_state(test_generation, |state| {
                                state["synced"] = Value::Bool(true);
                                state["line_count"] = serde_json::json!(lines.len());
                            });
                            lyrics.set_text("Караоке · синхронизация с воспроизведением");
                            *original_lines.borrow_mut() =
                                lines.iter().map(|line| line.words.clone()).collect();
                            let mut rows = synced.borrow_mut();
                            for (i, mut line) in lines.into_iter().enumerate() {
                                let version = versions
                                    .get(i)
                                    .filter(|v| !v.is_empty() && **v != line.words);
                                let secondary = if let Some(romanized) = version {
                                    if appearance.romanize_main {
                                        line.timing.clear();
                                        Some(std::mem::replace(&mut line.words, romanized.clone()))
                                    } else {
                                        Some(romanized.clone())
                                    }
                                } else {
                                    None
                                };
                                let label = text(if line.words.is_empty() {
                                    "♪"
                                } else {
                                    &line.words
                                });
                                label.set_selectable(false);
                                appearance.format_line(&label, false, line.alternate);
                                if rows.is_empty() {
                                    update_test_state(test_generation, |state| {
                                        state["label_alignment"] =
                                            serde_json::json!(label.xalign());
                                    });
                                }
                                if appearance.seek {
                                    label.set_cursor_from_name(Some("pointer"));
                                    label.add_css_class("seek-lyric");
                                    label.set_focusable(true);
                                    label.set_tooltip_text(Some("Перейти к этой строке"));
                                    let click = gtk::GestureClick::new();
                                    click.set_button(1);
                                    let line_seek = seek.clone();
                                    let at = (line.seconds - offset).max(0.);
                                    click.connect_released(move |_, _, _, _| line_seek(at));
                                    label.add_controller(click);
                                    let keys = gtk::EventControllerKey::new();
                                    let line_seek = seek.clone();
                                    keys.connect_key_pressed(move |_, key, _, _| {
                                        if matches!(
                                            key,
                                            gtk::gdk::Key::Return | gtk::gdk::Key::space
                                        ) {
                                            line_seek(at);
                                            glib::Propagation::Stop
                                        } else {
                                            glib::Propagation::Proceed
                                        }
                                    });
                                    label.add_controller(keys);
                                }
                                label.add_css_class("karaoke-line");
                                label.add_css_class("karaoke-future");

                                let container = super::lyric_display::LyricVisual::new();
                                container.append(&label);
                                let subtitle = secondary.map(|words| {
                                    let secondary = text(&words);
                                    secondary.add_css_class("lyrics-subtitle");
                                    appearance.format_line(&secondary, false, line.alternate);
                                    container.append(&secondary);
                                    secondary
                                });
                                lines_box.append(&container);
                                let glyph_ends = line.glyph_ends();
                                let translation = text("");
                                translation.add_css_class("lyrics-subtitle");
                                translation.set_visible(false);
                                appearance.format_line(&translation, false, line.alternate);
                                container.append(&translation);
                                rows.push(LyricRow {
                                    line,
                                    label,
                                    subtitle,
                                    translation,
                                    container,
                                    glyph_ends,
                                });
                            }
                        }
                        Ok(Lyrics::Plain(words)) => {
                            update_test_state(test_generation, |state| {
                                state["synced"] = Value::Bool(false);
                                state["line_count"] = serde_json::json!(words.lines().count());
                            });
                            lyrics.set_text("Обычный текст · синхронизация недоступна");
                            *original_lines.borrow_mut() =
                                words.lines().map(str::to_owned).collect();
                            let text = if versions.is_empty() {
                                words
                            } else {
                                words
                                    .lines()
                                    .zip(versions.iter())
                                    .map(|(original, romanized)| {
                                        if original == romanized || romanized.is_empty() {
                                            original.to_string()
                                        } else if appearance.romanize_main {
                                            format!("{romanized}\n{original}")
                                        } else {
                                            format!("{original}\n{romanized}")
                                        }
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            };
                            plain_lyrics.set_text(&text);
                            plain_lyrics.set_visible(true);
                        }
                        Ok(Lyrics::Notice(message)) | Err(message) => lyrics.set_text(&message),
                    }
                    let can_translate =
                        translation_mode != 0 && !original_lines.borrow().is_empty();
                    translate.set_visible(can_translate);
                    if can_translate && translation_mode == 2 {
                        translate.emit_clicked();
                    }
                }
                Event::Translation(id, result) if id == translation_id.get() => {
                    translate.set_sensitive(true);
                    match result {
                        Ok(lines) => {
                            let rows = synced.borrow();
                            if rows.is_empty() {
                                plain_translation.set_text(&lines.join("\n"));
                                plain_translation.set_visible(true);
                            } else {
                                for (row, words) in rows.iter().zip(lines) {
                                    row.translation.set_text(&words);
                                    row.translation.set_visible(!words.trim().is_empty());
                                }
                            }
                            translation_status.set_text("Перевод DeepL");
                        }
                        Err(error) => translation_status.set_text(&error),
                    }
                }
                Event::Translation(..) => {}
                Event::Info(result) => match result {
                    Ok((content, link)) => {
                        info.set_text(&content);
                        if let Some((title, url)) = link {
                            attribution.set_uri(&url);
                            attribution.set_label(&format!("Wikipedia: {title}"));
                            attribution.set_visible(true);
                        }
                    }
                    Err(error) => info.set_text(&error),
                },
            }
        }
    });
    let retry = super::button(
        "Повторить загрузку текста",
        Some("view-refresh-symbolic"),
        "flat",
    );
    lyrics_box.append(&retry);
    let retry_track = track.clone();
    let retry_tx = tx.clone();
    retry.connect_clicked(move |_| {
        let tx = retry_tx.clone();
        let track = retry_track.clone();
        std::thread::spawn(move || {
            let result = fetch_lyrics(&external_agent(), &track);
            let _ = tx.send_blocking(lyrics_event(result, appearance));
        });
    });
    std::thread::spawn(move || {
        let agent = external_agent();
        if !track.title.trim().is_empty() {
            let result = if let Some(prefetch) = prefetch {
                // A downloaded companion may have appeared after playback started.
                saved_lyrics(&track).map(Ok).unwrap_or_else(|| {
                    prefetch.get_or_init(|| fetch_lyrics(&agent, &track)).clone()
                })
            } else {
                fetch_lyrics(&agent, &track)
            };
            if tx
                .send_blocking(lyrics_event(result, appearance))
                .is_err()
            {
                return;
            }
        }
        if tx.is_closed() {
            return;
        }
        let _ = tx.send_blocking(Event::Info(fetch_info(&agent, &track)));
    });
    window.present();
}

/// Open album information, falling back to its artist if no appropriate album article exists.
pub fn show_album(parent: &gtk::ApplicationWindow, api: Api, album_title: String, artist: String) {
    show(
        parent,
        api,
        Track {
            album_title: Some(album_title),
            artist,
            ..Track::default()
        },
        || f64::NAN,
        |_| {},
        None,
        true,
        None,
        None,
    );
}

/// Open artist information without requesting song lyrics.
pub fn show_artist(parent: &gtk::ApplicationWindow, api: Api, artist: String) {
    show(
        parent,
        api,
        Track {
            artist,
            ..Track::default()
        },
        || f64::NAN,
        |_| {},
        None,
        true,
        None,
        None,
    );
}

fn text(value: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(value));
    label.set_xalign(0.0);
    label.set_yalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_selectable(true);
    label
}

fn scroll(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .vexpand(true)
        .child(child)
        .build()
}

fn track_metadata(track: &Track) -> String {
    let mut fields = vec![format!("Исполнитель: {}", track.artist)];
    if let Some(album) = track.album_title.as_deref().filter(|s| !s.is_empty()) {
        fields.push(format!("Альбом: {album}"));
    }
    if let Some(duration) = track.duration_s.filter(|n| n.is_finite() && *n > 0.0) {
        let seconds = duration.round() as u64;
        fields.push(format!(
            "Длительность: {}:{:02}",
            seconds / 60,
            seconds % 60
        ));
    }
    fields.join("\n")
}

fn active_line(lines: &[LyricRow], position: f64, real_timing: bool) -> Option<usize> {
    let i = lines
        .partition_point(|row| row.line.seconds <= position)
        .checked_sub(1)?;
    if real_timing && lines[i].line.end.is_some_and(|end| position >= end) {
        return (0..i)
            .rev()
            .find(|j| lines[*j].line.end.is_some_and(|end| position < end));
    }
    Some(i)
}

fn saved_lyrics(track: &Track) -> Option<super::lyric_sources::Match> {
    use super::lyric_sources::Match;
    let directory = companion_directory(track);
    if let Ok(data) = std::fs::read_to_string(directory.join("lyrics.json"))
        && let Ok(lines) = serde_json::from_str::<Vec<LyricLine>>(&data)
        && !lines.is_empty()
    {
        return Some(Match::local(Lyrics::Synced(lines)));
    }
    if let Ok(lrc) = std::fs::read_to_string(directory.join("lyrics.lrc")) {
        let lines = parse_lrc(&lrc);
        if !lines.is_empty() {
            return Some(Match::local(Lyrics::Synced(lines)));
        }
    }
    if let Ok(text) = std::fs::read_to_string(directory.join("lyrics.txt"))
        && !text.trim().is_empty()
    {
        return Some(Match::local(Lyrics::Plain(text)));
    }
    None
}

pub(super) fn fetch_lyrics(
    agent: &ureq::Agent,
    track: &Track,
) -> Result<super::lyric_sources::Match, String> {
    use super::lyric_sources::Match;
    if let Some(found) = saved_lyrics(track) {
        return Ok(found);
    }
    if crate::config::test() {
        return Err("В изолированной проверке доступны сохранённые тексты.".into());
    }
    if track.artist.trim().is_empty() || track.title.trim().is_empty() {
        return Ok(Match::local(Lyrics::Notice(
            "Для поиска текста нужны название песни и исполнитель.".into(),
        )));
    }
    let settings = Value::Object(crate::config::read_settings().unwrap_or_default());
    super::lyric_sources::lookup(agent, track, &settings)
}

fn lyrics_event(
    result: Result<super::lyric_sources::Match, String>,
    appearance: super::settings::LyricsAppearance,
) -> Event {
    let versions = if let Ok(found) = &result
        && appearance.romanize
    {
        let lines: Vec<_> = match &found.lyrics {
            Lyrics::Synced(lines) => lines.iter().map(|line| line.words.as_str()).collect(),
            Lyrics::Plain(text) => text.lines().collect(),
            Lyrics::Notice(_) => Vec::new(),
        };
        romanize(&lines, appearance.romanize_script)
    } else {
        Vec::new()
    };
    Event::Lyrics(result, versions)
}

fn romanize(lines: &[&str], script: usize) -> Vec<String> {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    if lines.is_empty() {
        return Vec::new();
    }
    let transform = [
        "Any-Latin",
        "Cyrillic-Latin",
        "Han-Latin",
        "Katakana-Latin; Hiragana-Latin",
        "Hangul-Latin",
        "Greek-Latin",
        "Arabic-Latin",
        "Hebrew-Latin",
    ][script];
    let Ok(mut child) = Command::new("uconv")
        .args(["-f", "UTF-8", "-t", "UTF-8", "-x", transform])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return Vec::new();
    };
    let mut stdin = child.stdin.take().expect("piped input");
    let output = std::thread::scope(|scope| {
        scope.spawn(move || {
            for line in lines {
                if writeln!(stdin, "{}", line.replace(['\n', '\r'], " ")).is_err() {
                    break;
                }
            }
        });
        child.wait_with_output()
    });
    let Ok(output) = output else {
        return Vec::new();
    };
    let Ok(text) = String::from_utf8(output.stdout) else {
        return Vec::new();
    };
    let result: Vec<_> = text.lines().map(str::to_string).collect();
    if output.status.success() && result.len() == lines.len() {
        result
    } else {
        Vec::new()
    }
}

fn lrc_timestamp(timestamp: &str) -> Option<f64> {
    let (minutes, seconds) = timestamp.split_once(':')?;
    let minutes = minutes.parse::<u32>().ok()?;
    let seconds = seconds.parse::<f64>().ok()?;
    (0.0..60.0)
        .contains(&seconds)
        .then_some(minutes as f64 * 60. + seconds)
}

pub(super) fn parse_lrc(value: &str) -> Vec<LyricLine> {
    let offset = value
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("[offset:")?
                .strip_suffix(']')?
                .parse::<f64>()
                .ok()
        })
        .filter(|n| n.is_finite())
        .last()
        .unwrap_or(0.)
        / 1000.;
    let mut lines = Vec::new();
    for line in value.lines() {
        let mut rest = line.trim();
        let mut times = Vec::new();
        while let Some((tag, tail)) = rest.strip_prefix('[').and_then(|tag| tag.split_once(']')) {
            let Some(seconds) = lrc_timestamp(tag) else {
                break;
            };
            times.push(seconds);
            rest = tail.trim_start();
        }
        for &at in &times {
            let shift = offset + at - times[0];
            let mut words = String::new();
            let mut timing: Vec<LyricWord> = Vec::new();
            let mut begin = None;
            let mut tail = rest;
            while let Some((prefix, tagged)) = tail.split_once('<') {
                words.push_str(prefix);
                let Some((tag, suffix)) = tagged.split_once('>') else {
                    tail = &tail[prefix.len()..];
                    break;
                };
                let Some(time) = lrc_timestamp(tag) else {
                    words.push_str(&tail[prefix.len()..tail.len() - suffix.len()]);
                    tail = suffix;
                    continue;
                };
                let time = (time + shift).max(0.);
                if let Some(word_start) = begin
                    && timing.last().is_none_or(|word| word.byte_end < words.len())
                    && !words.is_empty()
                {
                    timing.push(LyricWord {
                        begin: word_start,
                        end: Some(time.max(word_start)),
                        byte_end: words.len(),
                    });
                }
                begin = Some(time);
                tail = suffix;
            }
            words.push_str(tail);
            if let Some(begin) = begin
                && timing.last().is_none_or(|word| word.byte_end < words.len())
                && !words.is_empty()
            {
                timing.push(LyricWord {
                    begin,
                    end: None,
                    byte_end: words.len(),
                });
            }
            timing.sort_by(|a, b| a.begin.total_cmp(&b.begin));
            let end = timing.last().and_then(|word| word.end);
            lines.push(LyricLine {
                seconds: (at + offset).max(0.),
                words,
                timing,
                end,
                alternate: false,
            });
        }
    }
    lines.sort_by(|a, b| a.seconds.total_cmp(&b.seconds));
    lines
}

pub(super) fn external_agent() -> ureq::Agent {
    // Keep external requests on the configured proxy; localhost API requests use Api separately.
    let proxy = std::env::var("HTTPS_PROXY")
        .or_else(|_| std::env::var("https_proxy"))
        .or_else(|_| std::env::var("ALL_PROXY"))
        .unwrap_or_else(|_| "http://127.0.0.1:2080".into());
    ureq::Agent::config_builder()
        .proxy(Some(ureq::Proxy::new(&proxy).unwrap_or_else(|_| {
            ureq::Proxy::new("http://127.0.0.1:2080").unwrap()
        })))
        .build()
        .into()
}

fn article_attempts(track: &Track) -> Vec<(&'static str, &str, &'static str)> {
    let mut attempts = Vec::new();
    // Exhaust every appropriate Russian article before displaying any English article.
    for language in ["ru", "en"] {
        for (name, kind) in [
            (Some(track.title.as_str()), "song"),
            (track.album_title.as_deref(), "album"),
            (Some(track.artist.as_str()), "artist"),
        ] {
            if let Some(name) = name.filter(|s| !s.trim().is_empty()) {
                attempts.push((language, name, kind));
            }
        }
    }
    attempts
}

fn wiki_pages(agent: &ureq::Agent, language: &str, query: &str, search: bool) -> Vec<Value> {
    let endpoint = format!("https://{language}.wikipedia.org/w/api.php");
    let mut request = agent
        .get(&endpoint)
        .header("User-Agent", CLIENT)
        .query("action", "query")
        .query("format", "json")
        .query("prop", "extracts|info|pageprops|langlinks")
        .query("lllang", "ru")
        .query("lllimit", "1")
        .query("exintro", "1")
        .query("explaintext", "1")
        .query("exchars", "5000")
        .query("inprop", "url")
        .query("redirects", "1")
        .query("utf8", "1");
    request = if search {
        request
            .query("generator", "search")
            .query("gsrsearch", query)
            .query("gsrlimit", "5")
            .query("gsrnamespace", "0")
    } else {
        request.query("titles", query)
    };
    let Ok(mut response) = request
        .config()
        .timeout_global(Some(Duration::from_secs(8)))
        .build()
        .call()
    else {
        return Vec::new();
    };
    let Ok(value) = response.body_mut().read_json::<Value>() else {
        return Vec::new();
    };
    let mut pages: Vec<Value> = value["query"]["pages"]
        .as_object()
        .map(|pages| pages.values().cloned().collect())
        .unwrap_or_default();
    pages.sort_by_key(|p| p["index"].as_u64().unwrap_or(u64::MAX));
    pages
}

fn russian_link(page: &Value) -> Option<&str> {
    page["langlinks"]
        .as_array()?
        .iter()
        .find(|link| link["lang"].as_str() == Some("ru"))?["*"]
        .as_str()
}

fn article_result(
    page: &Value,
    language: &str,
    name: &str,
    artist: &str,
    kind: &str,
    verified_translation: bool,
) -> Option<(String, Option<(String, String)>)> {
    let (title, extract, url) = (
        page["title"].as_str()?,
        page["extract"].as_str()?,
        page["fullurl"].as_str()?,
    );
    if extract.trim().is_empty()
        || page["pageprops"].get("disambiguation").is_some()
        || !url.starts_with(&format!("https://{language}.wikipedia.org/"))
    {
        return None;
    }
    // A langlink from an exact, artist-verified article establishes translated identity.
    // Still validate the translated article's type; never use arbitrary search snippets.
    if !(if verified_translation {
        article_kind_matches(extract, kind)
    } else {
        article_matches(title, extract, name, artist, kind)
    }) {
        return None;
    }
    let label = match kind {
        "song" => "О песне",
        "album" => "Об альбоме",
        _ => "Об исполнителе",
    };
    let fallback = if language == "en" {
        "Статья на английском (EN): подходящая русская статья не найдена.\n\n"
    } else {
        ""
    };
    Some((
        format!("{fallback}{label}: {title}\n\n{extract}"),
        Some((
            format!("{title} · {}", language.to_uppercase()),
            url.to_string(),
        )),
    ))
}

fn fetch_info(
    agent: &ureq::Agent,
    track: &Track,
) -> Result<(String, Option<(String, String)>), String> {
    if let Ok(bytes) = std::fs::read(companion_directory(track).join("info.json")) {
        if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
            if let Some(text) = value["text"].as_str().filter(|s| !s.trim().is_empty()) {
                let source =
                    serde_json::from_value::<Option<(String, String)>>(value["source"].clone())
                        .unwrap_or(None);
                return Ok((text.to_string(), source));
            }
        }
    }
    if crate::config::test() {
        return Err("Articles unavailable in isolated fixture tests".into());
    }
    if track.artist.trim().is_empty() {
        return Err("Для поиска статьи нужен исполнитель.".into());
    }
    // Cache English search results used to resolve translated Russian titles. They can
    // only be displayed after all three Russian stages have been exhausted.
    let mut english = std::collections::HashMap::new();
    let allow_english = std::fs::read(crate::config::settings_path())
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .is_some_and(|v| v["wiki_english"].as_bool() == Some(true));
    for (language, name, kind) in article_attempts(track) {
        if language == "en" && !allow_english {
            continue;
        }
        let query = if kind == "artist" {
            format!("\"{}\"", name.replace('"', ""))
        } else {
            format!(
                "\"{}\" \"{}\" {}",
                name.replace('"', ""),
                track.artist.replace('"', ""),
                match (language, kind) {
                    ("ru", "song") => "песня",
                    ("ru", "album") => "альбом",
                    _ => kind,
                }
            )
        };
        let pages = if language == "en" {
            english
                .remove(&kind)
                .unwrap_or_else(|| wiki_pages(agent, language, &query, true))
        } else {
            wiki_pages(agent, language, &query, true)
        };
        for page in &pages {
            if let Some(result) = article_result(page, language, name, &track.artist, kind, false) {
                return Ok(result);
            }
        }
        if language == "ru" {
            let query = if kind == "artist" {
                format!("\"{}\"", name.replace('"', ""))
            } else {
                format!(
                    "\"{}\" \"{}\" {kind}",
                    name.replace('"', ""),
                    track.artist.replace('"', "")
                )
            };
            let pages = wiki_pages(agent, "en", &query, true);
            for page in &pages {
                if article_result(page, "en", name, &track.artist, kind, false).is_none() {
                    continue;
                }
                if let Some(translated_title) = russian_link(page) {
                    for translated in wiki_pages(agent, "ru", translated_title, false) {
                        if let Some(result) =
                            article_result(&translated, "ru", name, &track.artist, kind, true)
                        {
                            return Ok(result);
                        }
                    }
                }
            }
            english.insert(kind, pages);
        }
    }
    Err("Подходящая статья о песне, альбоме или исполнителе не найдена либо Wikipedia недоступна. Метаданные показаны выше.".into())
}

fn normalized(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_lowercase().collect::<String>()
            } else {
                " ".into()
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn article_matches(title: &str, extract: &str, requested: &str, artist: &str, kind: &str) -> bool {
    let base = title.split(" (").next().unwrap_or(title);
    if !same(&normalized(base), &normalized(requested)) {
        return false;
    }
    let content = normalized(extract);
    let artist = normalized(artist);
    if artist.is_empty() || !format!(" {content} ").contains(&format!(" {artist} ")) {
        return false;
    }
    article_kind_matches(extract, kind)
}

fn article_kind_matches(extract: &str, kind: &str) -> bool {
    let intro = normalized(&extract.chars().take(700).collect::<String>());
    if kind == "song" {
        intro.split_whitespace().any(|w| {
            matches!(
                w,
                "song" | "single" | "песня" | "песни" | "сингл" | "композиция"
            )
        })
    } else if kind == "album" {
        intro
            .split_whitespace()
            .any(|w| matches!(w, "album" | "альбом" | "альбома"))
    } else if kind == "artist" {
        intro.split_whitespace().any(|w| {
            matches!(
                w,
                "singer"
                    | "songwriter"
                    | "musician"
                    | "rapper"
                    | "composer"
                    | "band"
                    | "duo"
                    | "певец"
                    | "певица"
                    | "музыкант"
                    | "рэпер"
                    | "композитор"
                    | "группа"
                    | "дуэт"
            )
        })
    } else {
        false
    }
}

fn same(left: &str, right: &str) -> bool {
    left.trim().to_lowercase() == right.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc_expands_multiple_timestamps_and_orders_lines() {
        let lines = parse_lrc("[ar:Someone]\n[00:20.50][00:05.125]Repeated\n[00:12]Middle");
        assert_eq!(
            lines,
            vec![
                LyricLine {
                    seconds: 5.125,
                    words: "Repeated".into(),
                    ..LyricLine::default()
                },
                LyricLine {
                    seconds: 12.0,
                    words: "Middle".into(),
                    ..LyricLine::default()
                },
                LyricLine {
                    seconds: 20.5,
                    words: "Repeated".into(),
                    ..LyricLine::default()
                },
            ]
        );
    }

    #[test]
    fn lrc_applies_millisecond_offset_and_preserves_instrumental_breaks() {
        let lines = parse_lrc("[offset:-500]\n[00:01.00]Hello\n[00:03.00]");
        assert_eq!(lines[0].seconds, 0.5);
        assert_eq!(
            lines[1],
            LyricLine {
                seconds: 2.5,
                words: String::new(),
                ..LyricLine::default()
            }
        );
    }

    #[test]
    fn plain_or_invalid_lyrics_never_acquire_fake_timestamps() {
        assert!(parse_lrc("Plain words\n[00:99]Invalid\n[00:NaN]Invalid").is_empty());
    }

    #[test]
    fn smooth_lyric_reveal_completes_the_whole_final_word_at_the_deadline() {
        let line = LyricLine {
            seconds: 1.,
            words: "Go now!".into(),
            timing: vec![
                LyricWord { begin: 1., end: Some(2.), byte_end: 3 },
                LyricWord { begin: 2., end: Some(2.1), byte_end: 6 },
            ],
            end: Some(2.02),
            ..LyricLine::default()
        };
        let ends = line.glyph_ends();
        assert_eq!(line.progress(&ends, 2., Some(4.), true), (3, 0));
        assert_eq!(line.progress(&ends, 2.015, Some(4.), true), (6, 0));
        assert_eq!(line.progress(&ends, 2.02, Some(4.), true), (7, 0));
    }

    #[test]
    fn unknown_final_word_end_follows_previous_word_duration() {
        let lines = parse_lrc("[00:00]<00:00.00>Stay <00:00.40>here\n[00:10]Next");
        let line = &lines[0];
        let ends = line.glyph_ends();
        assert_eq!(line.progress(&ends, 0.65, Some(10.), true), (7, 8));
        assert_eq!(line.progress(&ends, 1., Some(10.), true), (9, 0));
    }

    #[test]
    fn wikipedia_accepts_specific_song_and_album_articles() {
        assert!(article_matches(
            "Hello (Adele song)",
            "Hello is a song by Adele.",
            "Hello",
            "Adele",
            "song"
        ));
        assert!(article_matches(
            "25 (Adele album)",
            "25 is a studio album by Adele.",
            "25",
            "Adele",
            "album"
        ));
    }

    #[test]
    fn wikipedia_rejects_artist_wrong_song_and_wrong_artist() {
        assert!(!article_matches(
            "Adele",
            "Adele released the song Hello.",
            "Hello",
            "Adele",
            "song"
        ));
        assert!(!article_matches(
            "Hello",
            "Hello is a song by Lionel Richie.",
            "Hello",
            "Adele",
            "song"
        ));
        assert!(!article_matches(
            "Hello",
            "Hello is a company founded by Adele.",
            "Hello",
            "Adele",
            "song"
        ));
    }

    #[test]
    fn wikipedia_exhausts_russian_song_album_artist_before_english() {
        let track = Track {
            title: "Hello".into(),
            artist: "Adele".into(),
            album_title: Some("25".into()),
            ..Track::default()
        };
        assert_eq!(
            article_attempts(&track),
            vec![
                ("ru", "Hello", "song"),
                ("ru", "25", "album"),
                ("ru", "Adele", "artist"),
                ("en", "Hello", "song"),
                ("en", "25", "album"),
                ("en", "Adele", "artist")
            ]
        );
        let artist = Track {
            artist: "Adele".into(),
            ..Track::default()
        };
        assert_eq!(
            article_attempts(&artist),
            vec![("ru", "Adele", "artist"), ("en", "Adele", "artist")]
        );
    }

    #[test]
    fn wikipedia_artist_stage_requires_exact_artist_and_biography() {
        assert!(article_matches(
            "Adele",
            "Adele is an English singer and songwriter.",
            "Adele",
            "Adele",
            "artist"
        ));
        assert!(!article_matches(
            "Adele",
            "Adele released the song Hello.",
            "Hello",
            "Adele",
            "song"
        ));
        assert!(!article_matches(
            "Adele",
            "Adele is a town.",
            "Adele",
            "Adele",
            "artist"
        ));
        assert!(!article_matches(
            "Madonna",
            "Madonna is a singer.",
            "Adele",
            "Adele",
            "artist"
        ));
    }

    #[test]
    fn wikipedia_translated_title_requires_verified_russian_langlink() {
        let english = serde_json::json!({"title":"The Wall", "extract":"The Wall is an album by Pink Floyd.",
            "fullurl":"https://en.wikipedia.org/wiki/The_Wall", "langlinks":[{"lang":"ru", "*":"Стена (альбом)"}]});
        assert!(article_result(&english, "en", "The Wall", "Pink Floyd", "album", false).is_some());
        assert_eq!(russian_link(&english), Some("Стена (альбом)"));
        let russian = serde_json::json!({"title":"Стена (альбом)", "extract":"Стена — студийный альбом группы Пинк Флойд.",
            "fullurl":"https://ru.wikipedia.org/wiki/Стена"});
        assert!(article_result(&russian, "ru", "The Wall", "Pink Floyd", "album", false).is_none());
        assert!(article_result(&russian, "ru", "The Wall", "Pink Floyd", "album", true).is_some());
        let wrong = serde_json::json!({"title":"Стена", "extract":"Стена — вертикальная конструкция.",
            "fullurl":"https://ru.wikipedia.org/wiki/Стена"});
        assert!(article_result(&wrong, "ru", "The Wall", "Pink Floyd", "album", true).is_none());
    }

    #[test]
    fn wikipedia_marks_english_fallback_and_validates_source_host() {
        let page = serde_json::json!({"title":"Hello (Adele song)", "extract":"Hello is a song by Adele.",
            "fullurl":"https://en.wikipedia.org/wiki/Hello"});
        let (content, link) = article_result(&page, "en", "Hello", "Adele", "song", false).unwrap();
        assert!(content.contains("английском (EN)"));
        assert!(link.unwrap().0.ends_with("· EN"));
        assert!(article_result(&page, "ru", "Hello", "Adele", "song", false).is_none());
    }

    #[test]
    fn track_metadata_uses_real_track_duration() {
        let track = Track {
            artist: "Adele".into(),
            duration_s: Some(211.0),
            ..Track::default()
        };
        assert!(track_metadata(&track).contains("Длительность: 3:31"));
    }
}

/// Optional companion files for an explicitly requested permanent track download.
pub(super) fn save_extras(
    api: &Api,
    track: &Track,
    artwork: bool,
    lyrics: bool,
    info: bool,
) -> Result<(), String> {
    use std::fs;
    use std::os::unix::fs::DirBuilderExt;
    let directory = companion_directory(track);
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder.create(&directory).map_err(|e| e.to_string())?;
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    fs::write(
        directory.join("track.json"),
        serde_json::to_vec_pretty(track).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let agent = external_agent();
    let mut errors = vec![];
    if artwork && !track.cover().is_empty() {
        match api.image(&track.cover()) {
            Ok(bytes) => {
                fs::write(directory.join("cover.img"), bytes).map_err(|e| e.to_string())?;
            }
            Err(e) => errors.push(e),
        }
    }
    if lyrics {
        match fetch_lyrics(&agent, track).map(|found| found.lyrics) {
            Ok(Lyrics::Plain(text)) => {
                fs::write(directory.join("lyrics.txt"), text).map_err(|e| e.to_string())?
            }
            Ok(Lyrics::Synced(lines)) => {
                if lines
                    .iter()
                    .any(|line| !line.timing.is_empty() || line.alternate || line.end.is_some())
                {
                    fs::write(
                        directory.join("lyrics.json"),
                        serde_json::to_vec(&lines).map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| e.to_string())?;
                }
                let lrc = lines
                    .iter()
                    .map(|l| {
                        format!(
                            "[{:02}:{:05.2}]{}",
                            (l.seconds / 60.) as u64,
                            l.seconds % 60.,
                            l.words
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                fs::write(directory.join("lyrics.lrc"), lrc).map_err(|e| e.to_string())?;
            }
            Ok(Lyrics::Notice(e)) | Err(e) => errors.push(e),
        }
    }
    if info {
        match fetch_info(&agent, track) {
            Ok((text, source)) => fs::write(
                directory.join("info.json"),
                serde_json::to_vec_pretty(&serde_json::json!({"text":text,"source":source}))
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?,
            Err(e) => errors.push(e),
        }
    }
    for entry in fs::read_dir(&directory).into_iter().flatten().flatten() {
        crate::config::record_change(
            "CREATE/MODIFY",
            &entry.path(),
            "explicit track download companion data",
        );
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Трек заказан; часть дополнительных данных недоступна: {}",
            errors.join(" · ")
        ))
    }
}

fn companion_directory(track: &Track) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    track.reference.hash(&mut hash);
    let root = if crate::config::test() {
        crate::config::test_root().join("downloads")
    } else {
        crate::config::root().join("native/downloads")
    };
    root.join(format!("{:016x}", hash.finish()))
}

#[cfg(test)]
mod live_proxy_checks {
    use super::*;
    #[test]
    #[ignore = "requires the authorized local proxy and LRCLIB"]
    fn live_lyrics_via_configured_socks_proxy() {
        let track = Track {
            title: "Creep".into(),
            artist: "Radiohead".into(),
            ..Track::default()
        };
        match fetch_lyrics(&external_agent(), &track)
            .expect("lyrics request via configured proxy")
            .lyrics
        {
            Lyrics::Synced(lines) => assert!(!lines.is_empty()),
            Lyrics::Plain(text) => assert!(!text.trim().is_empty()),
            Lyrics::Notice(_) => panic!("known reference song must return lyrics"),
        }
    }
}
