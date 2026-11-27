//! Bounded Material motion. GTK's animation setting is the reduced-motion gate.
use gtk::{glib, graphene, prelude::*, subclass::prelude::*};
use std::cell::{Cell, RefCell};

thread_local! {
    // Keep the desktop preference when temporarily disabling GTK motion in-app.
    static GATE: RefCell<Option<(bool, bool)>> = const { RefCell::new(None) };
}

pub(super) fn enabled(mode: u32) -> bool {
    mode != 0 && gtk::Settings::default().is_some_and(|s| s.is_gtk_enable_animations())
}

pub(super) fn apply(window: &gtk::Window, mode: u32) {
    let mode = mode.min(2);
    if let Some(settings) = gtk::Settings::default() {
        GATE.with(|gate| {
            let current = settings.is_gtk_enable_animations();
            let mut gate = gate.borrow_mut();
            let (mut desktop, last) = gate.unwrap_or((current, current));
            if current != last {
                desktop = current;
            }
            let next = desktop && mode != 0;
            if current != next {
                settings.set_gtk_enable_animations(next);
            }
            *gate = Some((desktop, next));
        });
    }
    let active = if !enabled(mode) {
        "motion-off"
    } else if mode == 2 {
        "motion-expressive"
    } else {
        "motion-standard"
    };
    for class in ["motion-off", "motion-standard", "motion-expressive"] {
        if class == active {
            window.add_css_class(class);
        } else {
            window.remove_css_class(class);
        }
    }
}

mod spatial {
    use super::*;

    #[derive(Default)]
    pub struct Surface {
        pub offset: Cell<f32>,
        pub generation: Cell<u64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Surface {
        const NAME: &'static str = "HomeMusicMotionSurface";
        type Type = super::Surface;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for Surface {}
    impl BoxImpl for Surface {}
    impl WidgetImpl for Surface {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            snapshot.save();
            snapshot.translate(&graphene::Point::new(0., self.offset.get()));
            self.parent_snapshot(snapshot);
            snapshot.restore();
        }
    }
}

glib::wrapper! {
    pub struct Surface(ObjectSubclass<spatial::Surface>)
        @extends gtk::Widget, gtk::Box,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

pub(super) fn surface(child: &impl IsA<gtk::Widget>) -> Surface {
    let surface: Surface = glib::Object::builder()
        .property("orientation", gtk::Orientation::Vertical)
        .build();
    surface.set_overflow(gtk::Overflow::Hidden);
    surface.set_vexpand(child.as_ref().vexpands());
    surface.append(child);
    child
        .as_ref()
        .bind_property("visible", &surface, "visible")
        .sync_create()
        .build();
    surface
}

pub(super) fn page(widget: &impl IsA<gtk::Widget>, mode: u32) {
    let widget = widget.as_ref();
    let surface = widget.parent().and_downcast::<Surface>();
    let generation = surface.as_ref().map(|surface| {
        let generation = surface.imp().generation.get().wrapping_add(1);
        surface.imp().generation.set(generation);
        surface.imp().offset.set(0.);
        generation
    });
    widget.set_opacity(1.);
    if !enabled(mode) || !widget.is_mapped() {
        if let Some(surface) = surface {
            surface.queue_draw();
        }
        return;
    }
    let (distance, frequency, damping, duration) = if mode == 2 {
        (28., 17., 0.64, 0.49)
    } else {
        (12., 25., 1., 0.30)
    };
    let move_surface = |surface: &Surface, offset| {
        surface.imp().offset.set(offset);
        surface.queue_draw();
    };
    if let Some(surface) = &surface {
        move_surface(surface, distance);
    }
    widget.set_opacity(0.60);
    let start = glib::monotonic_time();
    let weak_surface = surface.as_ref().map(ObjectExt::downgrade);
    widget.add_tick_callback(move |widget, clock| {
        let surface = weak_surface.as_ref().and_then(gtk::glib::WeakRef::upgrade);
        if surface
            .as_ref()
            .is_some_and(|s| Some(s.imp().generation.get()) != generation)
        {
            return glib::ControlFlow::Break;
        }
        let t = ((clock.frame_time() - start) as f64 / 1_000_000.).max(0.);
        if t >= duration || !enabled(mode) || !widget.is_mapped() {
            widget.set_opacity(1.);
            if let Some(surface) = surface {
                move_surface(&surface, 0.);
            }
            return glib::ControlFlow::Break;
        }
        let effects_time = 24. * t;
        widget.set_opacity(1. - 0.40 * (1. + effects_time) * (-effects_time).exp());
        if let Some(surface) = surface {
            let k = frequency * t;
            let factor = if damping < 1. {
                let w = (1.0_f64 - damping * damping).sqrt();
                (-damping * k).exp() * ((w * k).cos() + damping / w * (w * k).sin())
            } else {
                (1. + k) * (-k).exp()
            };
            move_surface(&surface, distance * factor as f32);
        }
        glib::ControlFlow::Continue
    });
}

pub(super) fn state(widget: &gtk::Widget) -> (f64, f32) {
    let offset = widget
        .parent()
        .and_downcast::<Surface>()
        .map_or(0., |surface| surface.imp().offset.get());
    (widget.opacity(), offset)
}

pub(super) fn reveal(revealer: &gtk::Revealer, visible: bool, mode: u32) {
    // Keep the horizontal measurement axis with motion disabled too. `None`
    // retains the child's width even when hidden; zero duration removes motion.
    revealer.set_transition_type(if revealer.has_css_class("left-panel") {
        gtk::RevealerTransitionType::SlideRight
    } else {
        gtk::RevealerTransitionType::SlideLeft
    });
    revealer.set_transition_duration(if !enabled(mode) {
        0
    } else if mode == 2 {
        240
    } else {
        180
    });
    revealer.set_reveal_child(visible);
}
