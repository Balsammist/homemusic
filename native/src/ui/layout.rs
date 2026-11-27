use super::*;

pub(super) fn resizable(
    child: &gtk::Box,
    left: bool,
) -> (gtk::Overlay, gtk::Box, gtk::GestureDrag) {
    let surface = gtk::Overlay::new();
    // Reserve the eight-pixel gutter outside the panel surface.
    let row = hbox(0);
    surface.set_child(Some(&row));
    let handle = hbox(0);
    handle.add_css_class("resize-handle");
    handle.set_width_request(16);
    handle.set_halign(if left {
        gtk::Align::End
    } else {
        gtk::Align::Start
    });
    handle.set_vexpand(true);
    handle.set_focusable(true);
    handle.set_cursor_from_name(Some("col-resize"));
    handle.set_tooltip_text(Some(if left {
        "Ширина медиатеки · перетащите или используйте ← →"
    } else {
        "Ширина правой панели · перетащите или используйте ← →"
    }));
    let line = vbox(0);
    line.add_css_class("resize-line");
    line.set_width_request(2);
    line.set_halign(gtk::Align::Center);
    line.set_valign(gtk::Align::Center);
    line.set_height_request(80);
    line.set_vexpand(false);
    handle.set_homogeneous(true);
    handle.append(&line);
    if left {
        row.append(child);
        row.append(&handle);
    } else {
        row.append(&handle);
        row.append(child);
    }
    let drag = gtk::GestureDrag::new();
    drag.set_button(1);
    drag.set_propagation_phase(gtk::PropagationPhase::Capture);
    (surface, handle, drag)
}

pub(super) fn install(ui: &Rc<Ui>, handle: &gtk::Box, drag: &gtk::GestureDrag, left: bool) {
    let origin = Rc::new(Cell::new(0));
    let active = Rc::new(Cell::new(false));
    let weak = Rc::downgrade(ui);
    let start = origin.clone();
    let edge = handle.downgrade();
    let begun = active.clone();
    drag.connect_drag_begin(move |gesture, x, y| {
        begun.set(false);
        if let Some(u) = weak.upgrade() {
            let synthetic = u.test && gesture.current_event().is_none();
            let inside = edge
                .upgrade()
                .and_then(|w| w.compute_bounds(&u.window))
                .is_some_and(|b| {
                    x >= b.x() as f64
                        && x <= (b.x() + b.width()) as f64
                        && y >= b.y() as f64
                        && y <= (b.y() + b.height()) as f64
                });
            if !synthetic && !inside {
                gesture.set_state(gtk::EventSequenceState::Denied);
                return;
            }
            gesture.set_state(gtk::EventSequenceState::Claimed);
            begun.set(true);
            start.set(if left {
                u.sidebar_widget.width_request()
            } else {
                u.right_widget.width_request()
            });
        }
    });
    let weak = Rc::downgrade(ui);
    let moving = active.clone();
    drag.connect_drag_update(move |_, dx, _| {
        if !moving.get() {
            return;
        }
        if let Some(u) = weak.upgrade() {
            u.resize_side(
                left,
                origin.get() + (if left { dx } else { -dx }).round() as i32,
                false,
            );
        }
    });
    let weak = Rc::downgrade(ui);
    drag.connect_drag_end(move |_, _, _| {
        if !active.replace(false) {
            return;
        }
        if let Some(u) = weak.upgrade() {
            u.save_side_width(left);
        }
    });
    ui.window.add_controller(drag.clone());
    let keys = gtk::EventControllerKey::new();
    let weak = Rc::downgrade(ui);
    keys.connect_key_pressed(move |_, key, _, _| {
        let delta = match key {
            gdk::Key::Left => -16,
            gdk::Key::Right => 16,
            _ => return glib::Propagation::Proceed,
        };
        if let Some(u) = weak.upgrade() {
            let width = if left {
                u.sidebar_widget.width_request()
            } else {
                u.right_widget.width_request()
            };
            u.resize_side(left, width + if left { delta } else { -delta }, true);
        }
        glib::Propagation::Stop
    });
    handle.add_controller(keys);
}

impl Ui {
    pub(super) fn compact_layout(&self) -> bool {
        let required = (self
            .sidebar_widget
            .measure(gtk::Orientation::Horizontal, -1)
            .0
            + self
                .right_widget
                .measure(gtk::Orientation::Horizontal, -1)
                .0
            + 512)
            .max(1120);
        self.window.width() < required
    }
    pub(super) fn apply_panel_widths(&self) {
        let available = (self.window.width().max(800) - 504).max(240);
        for (widget, preferred, minimum) in [
            (&self.sidebar_widget, self.sidebar_width.get(), 200),
            (&self.right_widget, self.right_width.get(), 240),
        ] {
            let width = preferred.clamp(minimum, available.min(480));
            if widget.width_request() != width {
                widget.set_width_request(width);
            }
        }
    }
    pub(super) fn resize_side(&self, left: bool, width: i32, persist: bool) {
        let other = if left && self.panel_revealer.reveals_child() {
            self.right_widget
                .measure(gtk::Orientation::Horizontal, -1)
                .0
        } else if !left && self.sidebar_revealer.reveals_child() {
            self.sidebar_widget
                .measure(gtk::Orientation::Horizontal, -1)
                .0
        } else {
            0
        };
        let minimum = if left { 200 } else { 240 };
        let maximum = (self.window.width() - other - 504).clamp(minimum, 480);
        let target = if left {
            &self.sidebar_width
        } else {
            &self.right_width
        };
        target.set(width.clamp(minimum, maximum));
        self.update_panels();
        if persist {
            self.save_side_width(left);
        }
    }
    fn save_side_width(&self, left: bool) {
        self.save_preferences(&[(
            if left {
                "sidebar_width"
            } else {
                "right_panel_width"
            },
            json!(if left {
                self.sidebar_width.get()
            } else {
                self.right_width.get()
            }),
        )], &[]);
    }
    pub(super) fn update_panel_button(&self) {
        let tab = self
            .panel_stack
            .visible_child_name()
            .unwrap_or_else(|| "now".into());
        let (icon, name) = match tab.as_str() {
            "queue" => ("view-list-symbolic", "Очередь"),
            "history" => ("document-open-recent-symbolic", "История"),
            _ => ("view-reveal-symbolic", "Сейчас играет"),
        };
        let open = self.panel_revealer.reveals_child();
        let tip = format!(
            "{}: {name}",
            if open {
                "Скрыть"
            } else {
                "Открыть"
            }
        );
        if self.panel_button.tooltip_text().as_deref() != Some(tip.as_str()) {
            set_md_icon(&self.panel_button, icon);
            self.panel_button.set_tooltip_text(Some(&tip));
        }
        if open {
            self.panel_button.add_css_class("selected");
        } else {
            self.panel_button.remove_css_class("selected");
        }
        let duration = if self.motion_mode.get() == 0
            || gtk::Settings::default().is_some_and(|s| !s.is_gtk_enable_animations())
        {
            0
        } else if self.motion_mode.get() == 2 {
            240
        } else {
            180
        };
        if self.panel_stack.transition_duration() != duration {
            self.panel_stack.set_transition_duration(duration);
        }
    }
}
