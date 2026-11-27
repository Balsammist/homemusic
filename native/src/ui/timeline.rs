//! The drawable fraction is independent of the pipeline clock and GTK input range.
use super::*;

const SEEK_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Default)]
pub(super) struct Timeline {
    pub(super) fraction: Rc<Cell<f64>>,
    pub(super) canvas: gtk::DrawingArea,
    duration: Cell<f64>,
    sample: Cell<f64>,
    pending: Cell<Option<(f64, Instant)>>,
    preview: Cell<Option<f64>>,
    tween: Cell<Option<(f64, Instant)>>,
    running: Cell<bool>,
    label_seconds: Cell<(u64, u64)>,
}

impl Timeline {
    fn transition(&self) {
        self.tween.set(Some((self.fraction.get(), Instant::now())));
    }

    fn sample(&self, confirmed: Option<f64>) -> f64 {
        if let Some((requested, expires)) = self.pending.get() {
            if let Some(value) = confirmed.filter(|value| (value - requested).abs() < 0.35) {
                self.pending.set(None);
                self.sample.set(value);
            } else if Instant::now() < expires {
                return self.preview.get().unwrap_or(requested);
            } else {
                self.pending.set(None);
                if let Some(confirmed) = confirmed {
                    self.sample.set(confirmed);
                }
                self.transition();
            }
        }
        if let Some(confirmed) = confirmed {
            self.sample.set(self.sample.get().max(confirmed));
        }
        self.preview.get().unwrap_or(self.sample.get())
    }
}

impl Ui {
    pub(super) fn install_seek(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.seek.connect_change_value(move |_, _, position| {
            if let Some(ui) = weak.upgrade() {
                ui.seek_to(position);
            }
            glib::Propagation::Stop
        });

        let drag = gtk::GestureDrag::new();
        drag.set_button(1);
        drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        let origin = Rc::new(Cell::new(None::<(f64, u64)>));
        let weak = Rc::downgrade(self);
        let start = origin.clone();
        drag.connect_drag_begin(move |gesture, x, _| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if let Some(ui) = weak.upgrade() {
                start.set(Some((x, ui.resolve.get())));
                ui.seek.grab_focus();
            }
        });
        let weak = Rc::downgrade(self);
        let start = origin.clone();
        drag.connect_drag_update(move |_, dx, _| {
            if let (Some(ui), Some((x, generation))) = (weak.upgrade(), start.get())
                && generation == ui.resolve.get()
                && (dx.abs() >= 3. || ui.timeline.preview.get().is_some())
            {
                ui.timeline.preview.set(Some(ui.seek_coordinate(x + dx)));
                ui.timeline.tween.set(None);
                ui.show_timeline();
            }
        });
        let weak = Rc::downgrade(self);
        let start = origin.clone();
        drag.connect_drag_end(move |_, dx, _| {
            if let (Some(ui), Some((x, generation))) = (weak.upgrade(), start.take())
                && generation == ui.resolve.get()
            {
                ui.timeline.preview.set(None);
                ui.seek_to(ui.seek_coordinate(x + dx));
            }
        });
        let weak = Rc::downgrade(self);
        drag.connect_cancel(move |_, _| {
            origin.set(None);
            if let Some(ui) = weak.upgrade() {
                ui.timeline.preview.set(None);
                ui.timeline.transition();
                ui.show_timeline();
            }
        });
        self.seek.add_controller(drag);
    }

    fn seek_coordinate(&self, x: f64) -> f64 {
        material::fraction(&self.timeline.canvas, &self.seek, x) * self.timeline.duration.get()
    }

    pub(super) fn seek_to(self: &Rc<Self>, position: f64) -> bool {
        let duration = self.timeline.duration.get();
        if !position.is_finite() {
            return false;
        }
        let position = position.clamp(0., duration);
        if self.resolving_pause.get().is_some()
            || duration <= 0.
            || !self
                .player
                .command(&["seek", &position.to_string(), "absolute"])
        {
            self.show_timeline();
            return false;
        }
        self.restore_position.set(None);
        self.timeline
            .pending
            .set(Some((position, Instant::now() + SEEK_TIMEOUT)));
        self.timeline.transition();
        self.show_timeline();
        true
    }

    pub(super) fn reset_timeline(self: &Rc<Self>, duration: f64, position: f64) {
        self.timeline.duration.set(duration.max(0.));
        self.timeline.sample.set(position.max(0.));
        self.timeline.pending.set(None);
        self.timeline.preview.set(None);
        self.timeline.transition();
        self.show_timeline();
    }

    pub(super) fn position_to_restore(&self) -> f64 {
        self.restore_position
            .get()
            .or_else(|| self.timeline.pending.get().map(|(position, _)| position))
            .or_else(|| self.player.position())
            .unwrap_or(self.timeline.sample.get())
    }

    pub(super) fn timeline_duration(&self) -> f64 {
        self.timeline.duration.get()
    }

    pub(super) fn update_timeline_duration(&self, duration: f64) {
        if duration > 0. && (self.timeline.duration.get() - duration).abs() >= 0.01 {
            self.timeline.duration.set(duration);
            self.timeline.transition();
        }
    }

    fn render_timeline(&self) -> bool {
        let timeline = &self.timeline;
        let idle = self.player.flag("idle-active");
        let playing = !idle && !self.player.flag("pause");
        material::repaint_wave(self, playing, glib::monotonic_time());
        let confirmed = if idle || self.restore_position.get().is_some() {
            None
        } else {
            self.player.position()
        };
        let position = timeline.sample(confirmed);
        if self.now_lyrics.widget().is_visible() {
            self.now_lyrics.update(position);
        }
        let duration = timeline.duration.get();
        let target = (position / duration.max(0.001)).clamp(0., 1.);
        let next = if let Some((start, since)) = timeline.tween.get() {
            let mode = self.motion_mode.get();
            let animations = gtk::Settings::default().is_some_and(|s| s.is_gtk_enable_animations());
            let ms = if mode == 2 { 220. } else { 160. };
            let progress = (since.elapsed().as_secs_f64() * 1000. / ms).min(1.);
            if mode == 0 || !animations || progress == 1. {
                timeline.tween.set(None);
                target
            } else {
                start + (target - start) * (1. - (1. - progress).powi(3))
            }
        } else {
            target
        };
        if (timeline.fraction.replace(next) - next).abs() > f64::EPSILON {
            timeline.canvas.queue_draw();
        }
        if (self.seek.adjustment().upper() - duration.max(1.)).abs() >= 0.01 {
            self.seek.set_range(0., duration.max(1.));
        }
        if (self.seek.value() - position).abs() >= 0.1 {
            self.seek.set_value(position);
        }
        let clock = (position as u64, duration as u64);
        let previous = timeline.label_seconds.replace(clock);
        if clock.0 != previous.0 {
            self.position.set_text(&time_text(position));
        }
        if clock.1 != previous.1 {
            self.duration.set_text(&time_text(duration));
        }
        self.seek
            .set_sensitive(!idle && self.restore_position.get().is_none() && duration > 0.);
        timeline.tween.get().is_some() || playing
    }

    pub(super) fn show_timeline(self: &Rc<Self>) {
        if !self.render_timeline() || self.timeline.running.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        self.timeline.canvas.add_tick_callback(move |canvas, _| {
            let Some(ui) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if canvas.is_mapped() && ui.render_timeline() {
                glib::ControlFlow::Continue
            } else {
                ui.timeline.running.set(false);
                glib::ControlFlow::Break
            }
        });
    }
}
