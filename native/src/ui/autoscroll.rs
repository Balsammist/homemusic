//! Browser-style middle-button autoscroll for dynamically created scroll areas.
use super::*;

struct Session {
    scroll: gtk::ScrolledWindow,
    origin: (f64, f64),
    pointer: (f64, f64),
    held: bool,
    dragged: bool,
    frame_time: i64,
}
type Active = Rc<RefCell<Option<Session>>>;

fn stop(active: &Active, root: &gtk::Widget, marker: &gtk::DrawingArea) -> bool {
    let stopped = active.borrow_mut().take().is_some();
    if stopped {
        root.set_cursor_from_name(None);
        marker.set_visible(false);
        marker.queue_draw();
    }
    stopped
}

pub(super) fn install(window: &impl IsA<gtk::Widget>) {
    let active: Active = Rc::new(RefCell::new(None));
    // A non-targetable overlay leaves the existing child and its allocation intact.
    let marker = gtk::DrawingArea::new();
    marker.set_widget_name("autoscroll-marker");
    marker.set_visible(false);
    marker.set_can_target(false);
    marker.set_hexpand(true);
    marker.set_vexpand(true);
    if let Some(window) = window.as_ref().downcast_ref::<gtk::Window>() {
        if let Some(child) = window.child() {
            let overlay = if let Ok(overlay) = child.clone().downcast::<gtk::Overlay>() {
                overlay
            } else {
                window.set_child(None::<&gtk::Widget>);
                let overlay = gtk::Overlay::new();
                overlay.set_child(Some(&child));
                window.set_child(Some(&overlay));
                overlay
            };
            overlay.add_overlay(&marker);
            overlay.set_measure_overlay(&marker, false);
        }
        let state = active.clone();
        let indicator = marker.clone();
        window.connect_is_active_notify(move |window| {
            if !window.is_active() {
                stop(&state, window.upcast_ref(), &indicator);
            }
        });
    }
    let state = active.clone();
    marker.set_draw_func(move |_, cr, _, _| {
        let state = state.borrow();
        let Some(session) = state.as_ref() else {
            return;
        };
        let (x, y) = session.origin;
        cr.set_source_rgba(0.08, 0.08, 0.10, 0.90);
        cr.arc(x, y, 14., 0., std::f64::consts::TAU);
        let _ = cr.fill_preserve();
        cr.set_source_rgba(1., 1., 1., 0.95);
        cr.set_line_width(1.5);
        let _ = cr.stroke();
        for direction in [-1., 1.] {
            cr.move_to(x - 4., y + direction * 5.);
            cr.line_to(x, y + direction * 10.);
            cr.line_to(x + 4., y + direction * 5.);
            let _ = cr.stroke();
        }
        cr.arc(x, y, 1.5, 0., std::f64::consts::TAU);
        let _ = cr.fill();
    });
    let click = gtk::GestureClick::new();
    click.set_button(0);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let state = active.clone();
    let indicator = marker.clone();
    click.connect_pressed(move |gesture, _, x, y| {
        let Some(root) = gesture.widget() else { return };
        root.remove_css_class("keyboard-navigation");
        if stop(&state, &root, &indicator) {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            return;
        }
        if gesture.current_button() != 2 {
            return;
        }
        let mut target = root.pick(x, y, gtk::PickFlags::DEFAULT);
        while let Some(widget) = target {
            if let Ok(scroll) = widget.clone().downcast::<gtk::ScrolledWindow>() {
                let a = scroll.vadjustment();
                if a.upper() - a.lower() > a.page_size() {
                    *state.borrow_mut() = Some(Session {
                        scroll,
                        origin: (x, y),
                        pointer: (x, y),
                        held: true,
                        dragged: false,
                        frame_time: 0,
                    });
                    root.set_cursor_from_name(Some("all-scroll"));
                    indicator.set_visible(true);
                    indicator.queue_draw();
                    animate(&root, state.clone(), indicator.clone());
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    break;
                }
            }
            target = widget.parent();
        }
    });
    let state = active.clone();
    let indicator = marker.clone();
    click.connect_released(move |gesture, _, x, y| {
        let Some(root) = gesture.widget() else { return };
        let should_stop = if let Some(session) = state.borrow_mut().as_mut() {
            session.held = false;
            session.dragged || (x - session.origin.0).hypot(y - session.origin.1) > 6.
        } else {
            false
        };
        if should_stop {
            stop(&state, &root, &indicator);
        }
    });
    window.add_controller(click);
    let motion = gtk::EventControllerMotion::new();
    motion.set_propagation_phase(gtk::PropagationPhase::Capture);
    let state = active.clone();
    motion.connect_motion(move |_, x, y| {
        if let Some(session) = state.borrow_mut().as_mut() {
            session.pointer = (x, y);
            if session.held && (x - session.origin.0).hypot(y - session.origin.1) > 6. {
                session.dragged = true;
            }
        }
    });
    window.add_controller(motion);
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let state = active.clone();
    let indicator = marker.clone();
    keys.connect_key_pressed(move |controller, key, _, _| {
        let Some(root) = controller.widget() else {
            return glib::Propagation::Proceed;
        };
        if matches!(key, gdk::Key::Tab | gdk::Key::ISO_Left_Tab) {
            root.add_css_class("keyboard-navigation");
        }
        if key == gdk::Key::Escape && stop(&state, &root, &indicator) {
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(keys);
    let state = active.clone();
    let indicator = marker.clone();
    window.connect_unmap(move |root| {
        stop(&state, root.as_ref(), &indicator);
    });
}

fn animate(root: &gtk::Widget, active: Active, marker: gtk::DrawingArea) {
    root.add_tick_callback(move |root, clock| {
        let mut state = active.borrow_mut();
        let Some(session) = state.as_mut() else {
            return glib::ControlFlow::Break;
        };
        if !session.scroll.is_mapped() || !root.is_mapped() || !session.scroll.is_ancestor(root) {
            drop(state);
            stop(&active, root, &marker);
            return glib::ControlFlow::Break;
        }
        let now = clock.frame_time();
        let seconds = if session.frame_time == 0 {
            0.
        } else {
            ((now - session.frame_time) as f64 / 1_000_000.).clamp(0., 0.05)
        };
        session.frame_time = now;
        let delta = session.pointer.1 - session.origin.1;
        // Pixels per second; retain the dead zone and bounded acceleration.
        let speed = delta.signum() * ((delta.abs() - 12.).max(0.).powf(1.25) * 1.5625).min(2400.);
        let adjustment = session.scroll.vadjustment();
        adjustment.set_value((adjustment.value() + speed * seconds).clamp(
            adjustment.lower(),
            (adjustment.upper() - adjustment.page_size()).max(adjustment.lower()),
        ));
        glib::ControlFlow::Continue
    });
}
