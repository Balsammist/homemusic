//! Fixed-size Material cookie artwork. Source Pictures are retained offscreen;
//! decoding happens when their paintables change, never inside the draw callback.
use gtk::{cairo, gdk, gdk_pixbuf::Pixbuf, prelude::*};
use std::{cell::RefCell, io::Cursor, rc::Rc};

/// Nine lobes identify a wave; four identify a mix. Pass one or four Pictures
/// populated by Ui::load_cover. Their natural image sizes never affect measurement.
#[allow(deprecated)]
pub(super) fn cover(pictures: &[gtk::Picture], lobes: u32, size: i32) -> gtk::Widget {
    let size = size.max(1);
    let lobes = if lobes == 9 { 9 } else { 4 };
    let sources: Vec<gtk::Picture> = pictures.iter().take(4).cloned().collect();
    let images = Rc::new(RefCell::new(sources.iter().map(decode).collect::<Vec<_>>()));
    let canvas = gtk::DrawingArea::new();
    canvas.add_css_class("cookie-cover");
    canvas.add_css_class("theme-painted");
    canvas.set_content_width(size);
    canvas.set_content_height(size);
    canvas.set_size_request(size, size);
    canvas.set_hexpand(false);
    canvas.set_vexpand(false);
    canvas.set_halign(gtk::Align::Center);
    canvas.set_valign(gtk::Align::Center);
    canvas.set_can_target(false);
    for (index, picture) in sources.iter().enumerate() {
        let cache = images.clone();
        let weak_canvas = canvas.downgrade();
        picture.connect_paintable_notify(move |picture| {
            cache.borrow_mut()[index] = decode(picture);
            if let Some(canvas) = weak_canvas.upgrade() {
                canvas.queue_draw();
            }
        });
    }
    canvas.set_draw_func(move |widget, cr, width, height| {
        // Keep source Pictures alive while asynchronous cover requests target them.
        let _keep_sources_alive = &sources;
        let side = width.min(height) as f64;
        let x = (width as f64 - side) / 2.;
        let y = (height as f64 - side) / 2.;
        let color = |name: &str, fallback| {
            widget
                .style_context()
                .lookup_color(name)
                .unwrap_or(fallback)
        };
        let primary = color("hm_primary", gdk::RGBA::new(0.7, 0.65, 0.95, 1.));
        let container = color(
            "hm_secondary_container",
            gdk::RGBA::new(0.2, 0.18, 0.28, 1.),
        );
        if cr.save().is_err() {
            return;
        }
        match widget.root().and_downcast::<gtk::Window>() {
            Some(root) if root.has_css_class("square-artwork") => {
                cr.rectangle(x, y, side, side);
            }
            Some(root) if root.has_css_class("rounded-artwork") => {
                let radius = side * 0.18;
                let far = side - radius;
                let quarter = std::f64::consts::FRAC_PI_2;
                cr.new_sub_path();
                cr.arc(x + far, y + radius, radius, -quarter, 0.);
                cr.arc(x + far, y + far, radius, 0., quarter);
                cr.arc(x + radius, y + far, radius, quarter, 2. * quarter);
                cr.arc(x + radius, y + radius, radius, 2. * quarter, 3. * quarter);
                cr.close_path();
            }
            _ => cookie_path(cr, x, y, side, lobes),
        }
        cr.clip();
        source_color(cr, container);
        let _ = cr.paint();
        let images = images.borrow();
        let available: Vec<&Pixbuf> = images.iter().filter_map(Option::as_ref).collect();
        if available.len() == 1 {
            paint_image(cr, available[0], x, y, side, side);
        } else if !available.is_empty() {
            let half = side / 2.;
            for index in 0..4 {
                paint_image(
                    cr,
                    available[index % available.len()],
                    x + (index % 2) as f64 * half,
                    y + (index / 2) as f64 * half,
                    half,
                    half,
                );
            }
        } else {
            // A restrained vector motif remains useful while covers load or fail.
            source_color(cr, primary);
            cr.set_line_width((side * 0.035).max(1.));
            cr.set_line_cap(cairo::LineCap::Round);
            for i in 0..5 {
                let at = x + side * (0.3 + i as f64 * 0.1);
                let length = side * if i % 2 == 0 { 0.12 } else { 0.24 };
                cr.move_to(at, y + side / 2. - length / 2.);
                cr.line_to(at, y + side / 2. + length / 2.);
            }
            let _ = cr.stroke();
        }
        let _ = cr.restore();
    });
    let root = gtk::Overlay::new();
    root.set_child(Some(&canvas));
    root.set_size_request(size, size);
    root.set_hexpand(false);
    root.set_vexpand(false);
    root.set_halign(gtk::Align::Center);
    root.set_valign(gtk::Align::Center);
    let badge = gtk::Label::new(Some(if lobes == 9 { "\u{e1b8}" } else { "\u{e043}" }));
    badge.add_css_class("cookie-type-badge");
    badge.set_halign(gtk::Align::End);
    badge.set_valign(gtk::Align::End);
    badge.set_margin_end((size / 32).max(1));
    badge.set_margin_bottom((size / 32).max(1));
    badge.set_can_target(false);
    badge.set_tooltip_text(Some(if lobes == 9 {
        "Моя волна"
    } else {
        "Микс"
    }));
    let css = gtk::CssProvider::new();
    let badge_size = (size / 5).clamp(14, 30);
    css.load_from_string(&format!(
        ".cookie-type-badge {{ font-family: 'Material Symbols Rounded'; font-size: {}px; min-width: {}px; min-height: {}px; border-radius: 999px; background: @hm_primary; color: @hm_on_primary; padding: 2px; }}",
        (badge_size * 2 / 3).max(11), badge_size, badge_size));
    badge
        .style_context()
        .add_provider(&css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    root.add_overlay(&badge);
    root.set_measure_overlay(&badge, false);
    root.upcast()
}

fn decode(picture: &gtk::Picture) -> Option<Pixbuf> {
    let texture = picture.paintable()?.downcast::<gdk::Texture>().ok()?;
    let bytes = texture.save_to_png_bytes();
    Pixbuf::from_read(Cursor::new(bytes.as_ref().to_vec())).ok()
}

fn source_color(cr: &cairo::Context, color: gdk::RGBA) {
    cr.set_source_rgba(
        color.red() as f64,
        color.green() as f64,
        color.blue() as f64,
        color.alpha() as f64,
    );
}

fn cookie_point(angle: f64, side: f64, lobes: u32) -> (f64, f64) {
    let radius = side
        * (0.44
            + 0.055
                * (lobes as f64 * angle + if lobes == 4 { std::f64::consts::PI } else { 0. })
                    .cos());
    (
        side / 2. + radius * angle.cos(),
        side / 2. + radius * angle.sin(),
    )
}

fn cookie_path(cr: &cairo::Context, x: f64, y: f64, side: f64, lobes: u32) {
    cr.new_path();
    for step in 0..=360 {
        let angle = std::f64::consts::TAU * step as f64 / 360. - std::f64::consts::FRAC_PI_2;
        let (px, py) = cookie_point(angle, side, lobes);
        if step == 0 {
            cr.move_to(x + px, y + py);
        } else {
            cr.line_to(x + px, y + py);
        }
    }
    cr.close_path();
}

fn paint_image(cr: &cairo::Context, image: &Pixbuf, x: f64, y: f64, width: f64, height: f64) {
    let iw = image.width() as f64;
    let ih = image.height() as f64;
    if iw <= 0. || ih <= 0. || cr.save().is_err() {
        return;
    }
    cr.rectangle(x, y, width, height);
    cr.clip();
    let scale = (width / iw).max(height / ih);
    cr.translate(
        x + (width - iw * scale) / 2.,
        y + (height - ih * scale) / 2.,
    );
    cr.scale(scale, scale);
    cr.set_source_pixbuf(image, 0., 0.);
    cr.source().set_filter(cairo::Filter::Bilinear);
    let _ = cr.paint();
    let _ = cr.restore();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_bounds_stay_within_requested_size() {
        for size in [32., 128., 384.] {
            for lobes in [4, 9] {
                for index in 0..360 {
                    let (x, y) =
                        cookie_point(index as f64 * std::f64::consts::TAU / 360., size, lobes);
                    assert!((0. ..=size).contains(&x) && (0. ..=size).contains(&y));
                }
            }
        }
    }

    #[test]
    fn cookie_path_clips_corners_but_keeps_center() {
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 128, 128).unwrap();
        let context = cairo::Context::new(&surface).unwrap();
        for lobes in [4, 9] {
            cookie_path(&context, 0., 0., 128., lobes);
            assert!(context.in_fill(64., 64.).unwrap());
            assert!(!context.in_fill(0., 0.).unwrap());
            assert!(!context.in_fill(127., 127.).unwrap());
        }
    }
}
