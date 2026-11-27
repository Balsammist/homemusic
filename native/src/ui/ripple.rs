use gtk::{cairo, gdk, glib, graphene, prelude::*};
use std::{
    cell::RefCell,
    f64::consts::{FRAC_PI_2, PI, TAU},
    rc::Rc,
};

const EXPAND_US: i64 = 280_000;
const FADE_AFTER_US: i64 = 110_000;
const FINISH_US: i64 = 340_000;

thread_local! {
    static TEST_PRESS: RefCell<serde_json::Value> = const { RefCell::new(serde_json::Value::Null) };
}

pub(super) fn test_press() -> serde_json::Value {
    TEST_PRESS.with(|press| press.borrow().clone())
}

struct Ripple {
    target: glib::WeakRef<gtk::Widget>,
    origin: graphene::Point,
    started: i64,
    test: bool,
}

pub(super) fn dropdown(options: &[&str]) -> gtk::DropDown {
    let control = gtk::DropDown::from_strings(options);
    // The list opens in its own native surface, outside the window's paint layer.
    let popover = std::iter::successors(control.first_child(), |child| child.next_sibling())
        .find_map(|child| child.downcast::<gtk::Popover>().ok());
    if let Some(popover) = popover
        && let Some(content) = popover.child()
    {
        popover.set_child(gtk::Widget::NONE);
        popover.set_child(Some(&wrap(&content)));
    }
    control
}

/// One passive input observer and one paint layer per window or popover.
pub(super) fn wrap(content: &impl IsA<gtk::Widget>) -> gtk::Overlay {
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(content));
    let canvas = gtk::DrawingArea::new();
    canvas.set_can_target(false);
    canvas.set_accessible_role(gtk::AccessibleRole::Presentation);
    overlay.add_overlay(&canvas);
    overlay.set_clip_overlay(&canvas, true);

    let ripple = Rc::new(RefCell::new(None::<Ripple>));
    let draw_state = ripple.clone();
    canvas.set_draw_func(move |canvas, cr, _, _| {
        if let Some(press) = draw_state.borrow().as_ref() {
            draw(canvas, cr, press);
        }
    });
    let clicks = gtk::EventControllerLegacy::new();
    clicks.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak_canvas = canvas.downgrade();
    clicks.connect_event(move |controller, event| {
        let primary = event
            .downcast_ref::<gdk::ButtonEvent>()
            .is_some_and(|button| {
                event.event_type() == gdk::EventType::ButtonPress
                    && button.button() == gdk::BUTTON_PRIMARY
            });
        if (primary || event.event_type() == gdk::EventType::TouchBegin)
            && let (Some(overlay), Some(canvas)) = (controller.widget(), weak_canvas.upgrade())
            && animations_enabled(&overlay)
            && let Some((target, origin)) = press_target(&overlay, event)
        {
            let running = ripple.borrow().is_some();
            let test = crate::config::test();
            if test {
                TEST_PRESS.with(|press| {
                    let mut press = press.borrow_mut();
                    let serial = press["serial"].as_u64().unwrap_or(0) + 1;
                    *press = serde_json::json!({
                        "serial": serial, "target": target.widget_name().as_str(),
                        "x": origin.x(), "y": origin.y(), "width": target.width(),
                        "painted": false,
                    });
                });
            }
            ripple.replace(Some(Ripple {
                target: target.downgrade(),
                origin,
                started: glib::monotonic_time(),
                test,
            }));
            canvas.queue_draw();
            if !running {
                let ripple = ripple.clone();
                canvas.add_tick_callback(move |canvas, _| {
                    let keep = ripple.borrow().as_ref().is_some_and(|press| {
                        glib::monotonic_time() - press.started < FINISH_US
                            && press
                                .target
                                .upgrade()
                                .is_some_and(|target| target.is_mapped())
                    }) && animations_enabled(canvas);
                    canvas.queue_draw();
                    if keep {
                        glib::ControlFlow::Continue
                    } else {
                        ripple.take();
                        glib::ControlFlow::Break
                    }
                });
            }
        }
        glib::Propagation::Proceed
    });
    overlay.add_controller(clicks);
    overlay
}

fn animations_enabled(widget: &impl IsA<gtk::Widget>) -> bool {
    widget.settings().is_gtk_enable_animations()
}

fn press_target(
    overlay: &gtk::Widget,
    event: &gdk::Event,
) -> Option<(gtk::Widget, graphene::Point)> {
    let native = overlay.native()?;
    if event.surface()? != native.surface()? {
        return None;
    }
    let (x, y) = event.position()?;
    let (offset_x, offset_y) = native.surface_transform();
    let native_widget = native.dynamic_cast::<gtk::Widget>().ok()?;
    let point = graphene::Point::new((x + offset_x) as f32, (y + offset_y) as f32);
    let point = native_widget.compute_point(overlay, &point)?;
    let mut target = overlay.pick(point.x().into(), point.y().into(), gtk::PickFlags::DEFAULT)?;
    while target != *overlay {
        if target.is::<gtk::Range>() || target.has_css_class("audio-control") {
            return None;
        }
        if target.is::<gtk::Button>()
            || target.is::<gtk::Switch>()
            || target.is::<gtk::CheckButton>()
            || target.css_name() == "row"
            || target.has_css_class("artist-track")
            || target.has_css_class("media-link")
            || target.has_css_class("artwork-link")
            || target.has_css_class("seek-lyric")
        {
            return Some((target.clone(), overlay.compute_point(&target, &point)?));
        }
        target = target.parent()?;
    }
    None
}

fn draw(canvas: &gtk::DrawingArea, cr: &cairo::Context, press: &Ripple) {
    let elapsed = glib::monotonic_time() - press.started;
    if elapsed >= FINISH_US || !animations_enabled(canvas) {
        return;
    }
    let Some(target) = press
        .target
        .upgrade()
        .filter(|target| target.is_mapped() && target.is_sensitive())
    else {
        return;
    };
    let (Some(bounds), Some(point)) = (
        target.compute_bounds(canvas),
        target.compute_point(canvas, &press.origin),
    ) else {
        return;
    };
    let (x, y, width, height) = rect(&bounds);
    let (cx, cy) = (f64::from(point.x()), f64::from(point.y()));
    let progress = (elapsed as f64 / EXPAND_US as f64).clamp(0.0, 1.0);
    let far_x = (cx - x).abs().max((x + width - cx).abs());
    let far_y = (cy - y).abs().max((y + height - cy).abs());
    let far = far_x.hypot(far_y);
    let radius = 5.0 + far * (1.0 - (1.0 - progress).powi(3));
    let fade =
        ((elapsed - FADE_AFTER_US) as f64 / (FINISH_US - FADE_AFTER_US) as f64).clamp(0.0, 1.0);
    if press.test {
        TEST_PRESS.with(|state| state.borrow_mut()["painted"] = serde_json::Value::Bool(true));
    }
    let color = target.color();
    let _ = cr.save();
    let mut parent = target.parent();
    while let Some(ancestor) = parent {
        if (ancestor.overflow() == gtk::Overflow::Hidden || ancestor.is::<gtk::ScrolledWindow>())
            && let Some(bounds) = ancestor.compute_bounds(canvas)
        {
            let (x, y, width, height) = rect(&bounds);
            cr.rectangle(x, y, width, height);
            cr.clip();
        }
        parent = ancestor.parent();
    }
    rounded_clip(cr, x, y, width, height, corner_radius(&target));
    cr.set_source_rgba(
        color.red().into(),
        color.green().into(),
        color.blue().into(),
        0.10 * (1.0 - fade),
    );
    cr.arc(cx, cy, radius, 0.0, TAU);
    let _ = cr.fill();
    let _ = cr.restore();
}

fn rect(bounds: &graphene::Rect) -> (f64, f64, f64, f64) {
    (
        bounds.x().into(),
        bounds.y().into(),
        bounds.width().into(),
        bounds.height().into(),
    )
}

fn corner_radius(target: &gtk::Widget) -> f64 {
    if target.has_css_class("menu-item") {
        0.0
    } else if target.has_css_class("large-play")
        && target
            .root()
            .and_downcast::<gtk::Window>()
            .is_some_and(|root| root.has_css_class("player-play-rect"))
    {
        12.0
    } else if target.is::<gtk::Switch>()
        || ["recommendation", "quick-card", "album-tile", "now-card"]
            .iter()
            .any(|class| target.has_css_class(class))
    {
        16.0
    } else if target.css_name() == "row"
        || [
            "queue-track",
            "artist-track",
            "library-item",
            "now-action",
            "media-link",
        ]
        .iter()
        .any(|class| target.has_css_class(class))
        || target
            .parent()
            .is_some_and(|parent| parent.is::<gtk::DropDown>())
    {
        12.0
    } else {
        20.0
    }
}

fn rounded_clip(cr: &cairo::Context, x: f64, y: f64, width: f64, height: f64, corners: f64) {
    let r = corners.min(width.min(height) / 2.0);
    if r <= 0.0 {
        cr.rectangle(x, y, width, height);
    } else {
        cr.new_sub_path();
        cr.arc(x + width - r, y + r, r, -FRAC_PI_2, 0.0);
        cr.arc(x + width - r, y + height - r, r, 0.0, FRAC_PI_2);
        cr.arc(x + r, y + height - r, r, FRAC_PI_2, PI);
        cr.arc(x + r, y + r, r, PI, PI + FRAC_PI_2);
        cr.close_path();
    }
    cr.clip();
}
