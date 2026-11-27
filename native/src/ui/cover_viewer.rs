use super::*;
use gtk::gdk_pixbuf::Pixbuf;
thread_local! {
    static TEST_VIEW: RefCell<std::rc::Weak<View>> = RefCell::new(std::rc::Weak::new());
}
pub(super) fn test_state() -> Value {
    TEST_VIEW.with(|view| {
        view.borrow().upgrade().map_or(Value::Null, |view| {
            json!({
                "angle": view.rotation.get(), "zoom": view.zoom.get(),
                "image": view.image.borrow().as_ref().map(|image| (image.width(), image.height())),
            })
        })
    })
}

#[derive(Default)]
struct View {
    image: RefCell<Option<Pixbuf>>,
    zoom: Cell<f64>,
    offset: Cell<(f64, f64)>,
    rotation: Cell<f64>,
    target: Cell<f64>,
    rotating: Cell<bool>,
}

pub(super) fn install(ui: &Rc<Ui>) {
    for (picture, current) in [
        (&ui.page_cover, false),
        (&ui.mini_picture, true),
        (&ui.now_picture, true),
    ] {
        picture.set_widget_name(if current {
            "current-cover-preview"
        } else {
            "page-cover-preview"
        });
        picture.add_css_class("artwork-link");
        picture.set_focusable(true);
        picture.set_cursor_from_name(Some("zoom-in"));
        picture.set_tooltip_text(Some("Рассмотреть обложку"));
        picture.update_property(&[gtk::accessible::Property::Label("Рассмотреть обложку")]);
        let open = {
            let ui = Rc::downgrade(ui);
            let picture = picture.downgrade();
            Rc::new(move || {
                let (Some(ui), Some(picture)) = (ui.upgrade(), picture.upgrade()) else {
                    return;
                };
                let (title, source) = if current {
                    let Some(track) = ui.current.borrow().clone() else {
                        return;
                    };
                    (track.title.clone(), track.cover())
                } else {
                    (ui.title.text().to_string(), ui.page_art.borrow().clone())
                };
                if !source.is_empty() || picture.paintable().is_some() {
                    show(
                        &ui,
                        &title,
                        &source,
                        picture.paintable().and_downcast::<gdk::Texture>(),
                    );
                }
            })
        };
        let click = gtk::GestureClick::new();
        click.set_button(1);
        let action = open.clone();
        click.connect_released(move |_, count, _, _| {
            if count == 1 {
                action();
            }
        });
        picture.add_controller(click);
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(move |_, key, _, _| {
            if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space) {
                open();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        picture.add_controller(keys);
    }
}

pub(super) fn show(ui: &Rc<Ui>, title: &str, source: &str, thumbnail: Option<gdk::Texture>) {
    let window = gtk::Window::builder()
        .transient_for(&ui.window)
        .modal(true)
        .title(format!("Обложка · {title}"))
        .default_width(720)
        .default_height(700)
        .build();
    window.add_css_class("cover-viewer");
    let view = Rc::new(View {
        zoom: Cell::new(1.),
        ..Default::default()
    });
    if crate::config::test() {
        TEST_VIEW.with(|saved| *saved.borrow_mut() = Rc::downgrade(&view));
    }
    *view.image.borrow_mut() = thumbnail.map(|texture| {
        let mut downloader = gdk::TextureDownloader::new(&texture);
        downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
        let (bytes, stride) = downloader.download_bytes();
        Pixbuf::from_bytes(
            &bytes,
            gtk::gdk_pixbuf::Colorspace::Rgb,
            true,
            8,
            texture.width(),
            texture.height(),
            stride as i32,
        )
    });
    let body = vbox(8);
    let art = gtk::DrawingArea::new();
    art.set_widget_name("cover-viewer-canvas");
    art.set_hexpand(true);
    art.set_vexpand(true);
    art.set_overflow(gtk::Overflow::Hidden);
    let state = view.clone();
    art.set_draw_func(move |_, cr, w, h| {
        let pixbuf = state.image.borrow();
        let Some(image) = pixbuf.as_ref() else {
            return;
        };
        let angle = state.rotation.get();
        let (sin, cos) = angle.sin_cos();
        let (iw, ih) = (f64::from(image.width()), f64::from(image.height()));
        let (w, h) = (f64::from(w), f64::from(h));
        let fit = ((w - 24.).max(1.) / (iw * cos.abs() + ih * sin.abs()))
            .min((h - 24.).max(1.) / (iw * sin.abs() + ih * cos.abs()));
        let zoom = fit * state.zoom.get();
        let (x, y) = state.offset.get();
        cr.translate(w / 2. + x, h / 2. + y);
        cr.rotate(angle);
        cr.scale(zoom, zoom);
        cr.set_source_pixbuf(image, -iw / 2., -ih / 2.);
        let _ = cr.paint();
    });
    body.append(&art);
    let tools = hbox(6);
    tools.add_css_class("cover-viewer-toolbar");
    tools.set_halign(gtk::Align::Center);
    body.append(&tools);
    let status = label("", "small muted");
    status.set_halign(gtk::Align::Center);
    body.append(&status);

    for (icon, label, key, action) in [
        (
            "object-rotate-left-symbolic",
            "Повернуть влево",
            "cover-rotate-left",
            0,
        ),
        (
            "object-rotate-right-symbolic",
            "Повернуть вправо",
            "cover-rotate-right",
            1,
        ),
        ("zoom-out-symbolic", "Уменьшить", "cover-zoom-out", 2),
        ("zoom-in-symbolic", "Увеличить", "cover-zoom-in", 3),
        ("zoom-fit-best-symbolic", "Сбросить вид", "cover-reset", 4),
        (
            "view-fullscreen-symbolic",
            "Во весь экран",
            "cover-fullscreen",
            5,
        ),
    ] {
        let button = icon_button(icon, label, "tonal");
        button.set_widget_name(key);
        let painter = art.clone();
        let view = view.clone();
        let weak = window.downgrade();
        button.connect_clicked(move |_| match action {
            0 => rotate(&painter, &view, -std::f64::consts::FRAC_PI_2),
            1 => rotate(&painter, &view, std::f64::consts::FRAC_PI_2),
            2 => zoom(&painter, &view, 1. / 1.25),
            3 => zoom(&painter, &view, 1.25),
            4 => {
                view.target.set(0.);
                view.rotation.set(0.);
                view.zoom.set(1.);
                view.offset.set((0., 0.));
                painter.queue_draw();
            }
            _ => {
                if let Some(window) = weak.upgrade() {
                    if window.is_fullscreen() {
                        window.unfullscreen();
                    } else {
                        window.fullscreen();
                    }
                }
            }
        });
        tools.append(&button);
    }
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    let painter = art.downgrade();
    let state = view.clone();
    scroll.connect_scroll(move |_, _, dy| {
        if let Some(painter) = painter.upgrade() {
            zoom(&painter, &state, 1.16_f64.powf(-dy));
        }
        glib::Propagation::Stop
    });
    art.add_controller(scroll);
    let drag = gtk::GestureDrag::new();
    drag.set_button(1);
    let origin = Rc::new(Cell::new((0., 0.)));
    let state = view.clone();
    let start = origin.clone();
    drag.connect_drag_begin(move |_, _, _| start.set(state.offset.get()));
    let state = view.clone();
    let painter = art.downgrade();
    drag.connect_drag_update(move |_, dx, dy| {
        if let Some(painter) = painter.upgrade()
            && state.zoom.get() > 1.
        {
            let (x, y) = origin.get();
            let scale = state.zoom.get() / 2.;
            let clamp = |value: f64, width| {
                value.clamp(-f64::from(width) * scale, f64::from(width) * scale)
            };
            state.offset.set((
                clamp(x + dx, painter.width()),
                clamp(y + dy, painter.height()),
            ));
            painter.queue_draw();
        }
    });
    art.add_controller(drag);
    let keys = gtk::EventControllerKey::new();
    let weak = window.downgrade();
    let painter = art.clone();
    let state = view.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        match key {
            gdk::Key::Escape => {
                if let Some(window) = weak.upgrade() {
                    if window.is_fullscreen() {
                        window.unfullscreen();
                    } else {
                        window.close();
                    }
                }
            }
            gdk::Key::Left => rotate(&painter, &state, -std::f64::consts::FRAC_PI_2),
            gdk::Key::Right => rotate(&painter, &state, std::f64::consts::FRAC_PI_2),
            gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add => zoom(&painter, &state, 1.25),
            gdk::Key::minus | gdk::Key::KP_Subtract => zoom(&painter, &state, 1. / 1.25),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    window.add_controller(keys);
    mount_dialog(&window, &body);
    window.present();

    if !source.is_empty() {
        let source = original_size(source);
        let api = ui.api.clone();
        let (tx, rx) = async_channel::bounded(1);
        if ui.images.run(move || {
            let _ = tx.send_blocking(api.image(&source));
        }) {
            if view.image.borrow().is_none() {
                status.set_text("Загружаем обложку…");
            }
            let painter = art.downgrade();
            let status = status.downgrade();
            glib::spawn_future_local(async move {
                let result = rx.recv().await.ok().and_then(Result::ok);
                let (Some(painter), Some(status)) = (painter.upgrade(), status.upgrade()) else {
                    return;
                };
                if let Some(bytes) = result {
                    let stream =
                        gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(bytes));
                    if let Ok(image) = Pixbuf::from_stream_at_scale(
                        &stream,
                        1024,
                        1024,
                        true,
                        gio::Cancellable::NONE,
                    ) {
                        *view.image.borrow_mut() = Some(image);
                        painter.queue_draw();
                    }
                }
                status.set_text(if view.image.borrow().is_some() {
                    ""
                } else {
                    "Обложка недоступна"
                });
            });
        } else if view.image.borrow().is_none() {
            status.set_text("Обложка недоступна. Попробуйте ещё раз.");
        }
    }
}

fn original_size(source: &str) -> String {
    if source.starts_with("/api/nd/cover/") {
        return format!("{}?size=1024", source.split('?').next().unwrap_or(source));
    }
    source.replace("250x250", "1000x1000")
}

fn zoom(painter: &gtk::DrawingArea, view: &View, factor: f64) {
    view.zoom.set((view.zoom.get() * factor).clamp(1., 4.));
    if view.zoom.get() == 1. {
        view.offset.set((0., 0.));
    }
    painter.queue_draw();
}

fn rotate(painter: &gtk::DrawingArea, view: &Rc<View>, delta: f64) {
    view.target.set(view.target.get() + delta);
    if !motion::enabled(1) {
        view.rotation.set(view.target.get());
        painter.queue_draw();
        return;
    }
    if view.rotating.replace(true) {
        return;
    }
    let view = view.clone();
    let previous = Cell::new(glib::monotonic_time());
    painter.add_tick_callback(move |painter, frame| {
        let elapsed = frame
            .frame_time()
            .saturating_sub(previous.replace(frame.frame_time()));
        let remaining = view.target.get() - view.rotation.get();
        if remaining.abs() < 0.004 || !motion::enabled(1) {
            view.rotation.set(view.target.get());
            view.rotating.set(false);
            painter.queue_draw();
            return glib::ControlFlow::Break;
        }
        view.rotation
            .set(view.rotation.get() + remaining * (1. - (-elapsed as f64 / 52_000.).exp()));
        painter.queue_draw();
        glib::ControlFlow::Continue
    });
}
