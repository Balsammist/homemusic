use super::*;

pub(super) const SEEK_INSET: f64 = 12.;
pub(super) const SLIDER_HEIGHT: i32 = 44;
#[derive(Clone, Copy)]
pub(super) enum Bar {
    Progress,
    Volume,
}

pub(super) const SEEK_STYLES: &[&str] =
    &["Material 3 · XS", "Тонкая линия", "Волна", "Material 3 · S"];

/// GtkScale owns input and accessibility. The same painter serves the player and preview.
pub(super) fn slider(scale: &gtk::Scale, timeline: &timeline::Timeline) -> gtk::Overlay {
    let root = gtk::Overlay::new();
    root.add_css_class("audio-control");
    let canvas = &timeline.canvas;
    canvas.add_css_class("theme-painted");
    canvas.set_content_height(SLIDER_HEIGHT);
    canvas.set_hexpand(true);
    root.set_child(Some(canvas));
    scale.set_opacity(0.001);
    scale.set_height_request(SLIDER_HEIGHT);
    scale.update_property(&[gtk::accessible::Property::Label("Позиция воспроизведения")]);
    root.add_overlay(scale);
    root.set_measure_overlay(scale, true);
    let fraction = timeline.fraction.clone();
    let repaint = canvas.downgrade();
    scale.connect_has_focus_notify(move |_| {
        if let Some(canvas) = repaint.upgrade() {
            canvas.queue_draw();
        }
    });
    let scale = scale.downgrade();
    canvas.set_draw_func(move |widget, cr, w, h| {
        let focused = scale
            .upgrade()
            .is_some_and(|scale| scale.has_visible_focus());
        draw_progress(widget, cr, w, h, fraction.get(), focused, Bar::Progress);
    });
    root
}

#[derive(Clone, Copy, PartialEq)]
pub(super) struct Bars {
    progress: u32,
    volume: u32,
    amplitude: f64,
    wavelength: f64,
    wave_speed: f64,
}

impl Bars {
    pub(super) fn from_settings(settings: &Value, progress: u32) -> Self {
        let option = |key: &str, fallback: usize| {
            settings[key]
                .as_u64()
                .filter(|value| *value < 3)
                .map_or(fallback, |value| value as usize)
        };
        Self {
            progress,
            volume: settings["volume_style"]
                .as_u64()
                .filter(|value| *value < 4)
                .map_or(1, |value| value as u32),
            amplitude: [2., 4., 7.][option("wave_height", 1)],
            wavelength: [22., 32., 48.][option("wave_length", 1)],
            wave_speed: [0., 28., 52.][option("wave_speed", 1)],
        }
    }

    fn variant(self, bar: Bar) -> u32 {
        match bar {
            Bar::Progress => self.progress,
            Bar::Volume => self.volume,
        }
    }

    pub(super) fn install(self) -> bool {
        BARS.with(|bars| bars.replace(self) != self)
    }
}

thread_local! {
    static BARS: Cell<Bars> = Cell::new(Bars {
        progress: 0, volume: 1, amplitude: 4., wavelength: 32., wave_speed: 28.,
    });
    static PHASE: Cell<f64> = const { Cell::new(0.) };
}

pub(super) fn repaint_wave(ui: &Ui, moving: bool, frame: i64) {
    let options = BARS.get();
    if !moving
        || !motion::enabled(ui.motion_mode.get())
        || options.wave_speed == 0.
        || (options.progress != 2 && options.volume != 2)
    {
        ui.wave_frame.set(None);
        return;
    }
    let Some(previous) = ui.wave_frame.replace(Some(frame)) else {
        return;
    };
    PHASE.set(
        (PHASE.get()
            + options.wave_speed * ((frame - previous) as f64 / 1_000_000.).clamp(0., 0.05))
            % options.wavelength,
    );
    if options.progress == 2 {
        ui.timeline.canvas.queue_draw();
    }
    if options.volume == 2
        && let Some(canvas) = ui
            .volume_control
            .parent()
            .and_downcast::<gtk::Overlay>()
            .and_then(|overlay| overlay.child())
    {
        canvas.queue_draw();
    }
}

pub(super) fn progress_preview(bar: Bar) -> gtk::DrawingArea {
    let canvas = gtk::DrawingArea::new();
    canvas.add_css_class("theme-painted");
    canvas.add_css_class("slider-preview");
    canvas.set_content_width(180);
    canvas.set_content_height(SLIDER_HEIGHT);
    canvas.set_hexpand(matches!(bar, Bar::Progress));
    if matches!(bar, Bar::Volume) {
        canvas.set_halign(gtk::Align::Start);
    }
    let ticking = Rc::new(Cell::new(false));
    canvas.set_draw_func(move |widget, cr, w, h| {
        draw_progress(widget, cr, w, h, 0.58, false, bar);
        if preview_animated(widget, bar) && !ticking.replace(true) {
            let ticking = ticking.clone();
            widget.add_tick_callback(move |widget, _| {
                if !preview_animated(widget, bar) {
                    ticking.set(false);
                    return glib::ControlFlow::Break;
                }
                widget.queue_draw();
                glib::ControlFlow::Continue
            });
        }
    });
    canvas
}

fn preview_animated(widget: &gtk::DrawingArea, bar: Bar) -> bool {
    let options = BARS.get();
    widget.is_mapped()
        && options.variant(bar) == 2
        && options.wave_speed > 0.
        && motion::enabled(1)
        && widget
            .ancestor(gtk::ScrolledWindow::static_type())
            .is_none_or(|scroll| {
                widget.compute_bounds(&scroll).is_some_and(|bounds| {
                    bounds.y() < scroll.height() as f32 && bounds.y() + bounds.height() > 0.
                })
            })
}

pub(super) fn test_state() -> Value {
    let bars = BARS.get();
    json!({"progress":bars.progress, "volume":bars.volume, "phase":PHASE.get()})
}

pub(super) fn fraction(painter: &gtk::DrawingArea, input: &gtk::Scale, x: f64) -> f64 {
    let point = gtk::graphene::Point::new(x as f32, 0.);
    let x = input
        .compute_point(painter, &point)
        .map_or(x, |point| f64::from(point.x()));
    ((x - SEEK_INSET) / (painter.width() as f64 - 2. * SEEK_INSET).max(1.)).clamp(0., 1.)
}

#[allow(deprecated)]
fn draw_progress(
    canvas: &gtk::DrawingArea,
    cr: &gtk::cairo::Context,
    w: i32,
    h: i32,
    ratio: f64,
    focused: bool,
    bar: Bar,
) {
    let options = BARS.get();
    let style = options.variant(bar);
    let width = w as f64;
    if width <= SEEK_INSET * 2. {
        return;
    }
    let start = SEEK_INSET;
    let end = width - SEEK_INSET;
    let x = start + (end - start) * ratio.clamp(0., 1.);
    let y = h as f64 / 2.;
    let context = canvas.style_context();
    let color = |name: &str| context.lookup_color(name).unwrap_or(gdk::RGBA::WHITE);
    let active = if canvas
        .root()
        .and_downcast::<gtk::Window>()
        .is_some_and(|root| root.has_css_class("player-tonal"))
    {
        color("hm_on_secondary_container")
    } else {
        color("hm_primary")
    };
    let inactive = if canvas
        .root()
        .and_downcast::<gtk::Window>()
        .is_some_and(|root| root.has_css_class("player-tonal"))
    {
        color("hm_surface_container_highest")
    } else {
        color("hm_secondary_container")
    };
    let tint = |c: gdk::RGBA| {
        cr.set_source_rgba(
            c.red().into(),
            c.green().into(),
            c.blue().into(),
            c.alpha().into(),
        )
    };
    let thickness: f64 = match style {
        1 if matches!(bar, Bar::Volume) => 6.,
        1 | 2 => 4.,
        3 => 24.,
        _ => 16.,
    };
    let thin_volume = style == 1 && matches!(bar, Bar::Volume);
    let gap = if thin_volume { 0. } else { 8. };
    let track_start = 2.;
    let track_end = width - track_start;
    let left_end = x - gap;
    let right_start = x + gap;
    if ratio > 0. && left_end > track_start {
        tint(active);
        if style == 2 {
            cr.set_line_width(thickness);
            cr.set_line_cap(gtk::cairo::LineCap::Round);
            let length = (left_end - track_start - thickness).max(0.);
            cr.move_to(track_start + thickness / 2., y);
            let phase = if canvas.has_css_class("slider-preview")
                && gtk::Settings::default()
                    .is_some_and(|settings| settings.is_gtk_enable_animations())
            {
                (glib::monotonic_time() as f64 / 1_000_000. * options.wave_speed)
                    % options.wavelength
            } else {
                PHASE.get()
            };
            let steps = (length / 2.5).ceil().clamp(1., 800.) as u32;
            for step in 1..=steps {
                let d = length * f64::from(step) / f64::from(steps);
                let envelope = (d.min(length - d) / 6.).clamp(0., 1.);
                cr.line_to(
                    track_start + thickness / 2. + d,
                    y + options.amplitude
                        * envelope
                        * ((d - phase) * std::f64::consts::TAU / options.wavelength).sin(),
                );
            }
            let _ = cr.stroke();
        } else {
            track_segment(cr, track_start, left_end, y, thickness, 8., 2.);
        }
    }
    if ratio < 1. && right_start < track_end {
        tint(inactive);
        track_segment(cr, right_start, track_end, y, thickness, 2., 8.);
        if !thin_volume && end - right_start >= 4. {
            tint(active);
            cr.arc(end, y, 2., 0., std::f64::consts::TAU);
            let _ = cr.fill();
        }
    }
    if focused {
        cr.set_source_rgba(
            active.red().into(),
            active.green().into(),
            active.blue().into(),
            0.15,
        );
        track_segment(cr, x - 7., x + 7., y, h as f64, 6., 6.);
    }
    tint(active);
    let handle_height = if thin_volume {
        12.
    } else if style == 1 || style == 2 {
        24.
    } else {
        f64::from(h.min(SLIDER_HEIGHT))
    };
    track_segment(cr, x - 2., x + 2., y, handle_height, 2., 2.);
}

/// Inner and outer corners have separate radii, including at the slider endpoints.
fn track_segment(
    cr: &gtk::cairo::Context,
    left: f64,
    right: f64,
    y: f64,
    height: f64,
    left_radius: f64,
    right_radius: f64,
) {
    let cap = ((right - left) / 2.).min(height / 2.);
    let a = left_radius.min(cap);
    let b = right_radius.min(cap);
    let top = y - height / 2.;
    let bottom = y + height / 2.;
    let q = std::f64::consts::FRAC_PI_2;
    cr.new_sub_path();
    cr.arc(right - b, top + b, b, -q, 0.);
    cr.arc(right - b, bottom - b, b, 0., q);
    cr.arc(left + a, bottom - a, a, q, q * 2.);
    cr.arc(left + a, top + a, a, q * 2., q * 3.);
    cr.close_path();
    let _ = cr.fill();
}

pub(super) fn volume(scale: &gtk::Scale) -> gtk::Overlay {
    let root = gtk::Overlay::new();
    root.add_css_class("audio-control");
    root.set_size_request(100, SLIDER_HEIGHT);
    root.set_hexpand(true);
    let canvas = gtk::DrawingArea::new();
    canvas.set_content_width(100);
    canvas.set_hexpand(true);
    canvas.add_css_class("theme-painted");
    canvas.set_content_height(SLIDER_HEIGHT);
    root.set_child(Some(&canvas));
    scale.set_opacity(0.001);
    scale.set_width_request(100);
    scale.set_hexpand(true);
    scale.set_height_request(SLIDER_HEIGHT);
    root.add_overlay(scale);
    root.set_measure_overlay(scale, true);
    // Claim primary pointer sequences before GtkRange's stepper/auto-repeat.
    // GestureDrag keeps the pointer grab through both edges and release outside.
    let drag = gtk::GestureDrag::new();
    drag.set_button(1);
    drag.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = scale.downgrade();
    let painter = canvas.downgrade();
    drag.connect_drag_begin(move |g, x, _| {
        g.set_state(gtk::EventSequenceState::Claimed);
        if let (Some(scale), Some(canvas)) = (weak.upgrade(), painter.upgrade()) {
            scale.grab_focus();
            scale.set_value(fraction(&canvas, &scale, x) * scale.adjustment().upper());
        }
    });
    let weak = scale.downgrade();
    let painter = canvas.downgrade();
    drag.connect_drag_update(move |g, dx, _| {
        if let (Some(scale), Some(canvas), Some((start, _))) =
            (weak.upgrade(), painter.upgrade(), g.start_point())
        {
            scale.set_value(fraction(&canvas, &scale, start + dx) * scale.adjustment().upper());
        }
    });
    scale.add_controller(drag);
    let weak = scale.downgrade();
    canvas.set_draw_func(move |widget, cr, w, h| {
        if let Some(scale) = weak.upgrade() {
            let ratio = scale.value() / scale.adjustment().upper().max(1.);
            draw_progress(
                widget,
                cr,
                w,
                h,
                ratio,
                scale.has_visible_focus(),
                Bar::Volume,
            );
        }
    });
    let painter = canvas.downgrade();
    scale.connect_value_changed(move |_| {
        if let Some(canvas) = painter.upgrade() {
            canvas.queue_draw();
        }
    });
    let painter = canvas.downgrade();
    scale.connect_has_focus_notify(move |_| {
        if let Some(canvas) = painter.upgrade() {
            canvas.queue_draw();
        }
    });
    scale
        .adjustment()
        .connect_changed(move |_| canvas.queue_draw());
    root
}
