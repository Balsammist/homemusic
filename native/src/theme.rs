use gtk::{gio, prelude::*};
use material_colors::{color::Rgb, dynamic_color::Variant, scheme::Scheme, theme::ThemeBuilder};
use serde_json::Value;
use std::{cell::{Cell, RefCell}, fs, path::PathBuf};

thread_local! {
    static APPLIED_CSS: RefCell<Option<(gtk::glib::WeakRef<gtk::CssProvider>, String)>> = const { RefCell::new(None) };
    static COVER_SEED: Cell<Option<Rgb>> = const { Cell::new(None) };
}

pub fn update_cover(provider: &gtk::CssProvider, picture: &gtk::Picture) {
    use material_colors::{
        quantize::{Quantizer, QuantizerCelebi},
        score::Score,
    };
    let seed = picture
        .paintable()
        .and_downcast::<gtk::gdk::Texture>()
        .and_then(|texture| {
            let mut downloader = gtk::gdk::TextureDownloader::new(&texture);
            downloader.set_format(gtk::gdk::MemoryFormat::R8g8b8a8);
            let (bytes, stride) = downloader.download_bytes();
            let width = texture.width().min(4096) as usize;
            let height = texture.height().min(4096) as usize;
            let mut pixels = Vec::with_capacity(32 * 32);
            for y in 0..32 {
                for x in 0..32 {
                    let offset = y * height / 32 * stride + x * width / 32 * 4;
                    let pixel = bytes.as_ref().get(offset..offset + 4)?;
                    if pixel[3] >= 128 {
                        pixels.push(Rgb::new(pixel[0], pixel[1], pixel[2]));
                    }
                }
            }
            if pixels.is_empty() {
                return None;
            }
            let colors = QuantizerCelebi::quantize(&pixels, 16);
            Score::score(&colors.color_to_count, Some(1), None, Some(true))
                .first()
                .copied()
        });
    if COVER_SEED.with(|saved| saved.replace(seed)) != seed {
        reload(provider);
    }
}

pub fn path() -> PathBuf {
    if let Some(p) = std::env::var_os("HOMEMUSIC_THEME") {
        return p.into();
    }
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
        });
    state.join("quickshell/user/generated/colors.json")
}
pub fn install() -> (gtk::CssProvider, Option<gio::FileMonitor>) {
    let provider = gtk::CssProvider::new();
    reload(&provider);
    // App-owned widgets share one provider so GTK user CSS cannot split the
    // panel and player into unrelated palettes.
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().unwrap(),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_USER + 1,
    );
    let parent = path().parent().unwrap().to_path_buf();
    let monitor = gio::File::for_path(parent)
        .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
        .ok();
    if let Some(m) = &monitor {
        let p = provider.clone();
        m.connect_changed(move |_, f, other, _| {
            if f.path().as_deref() == Some(path().as_path())
                || other.and_then(|f| f.path()).as_deref() == Some(path().as_path())
            {
                reload(&p);
            }
        });
    }
    if let Some(settings) = gtk::Settings::default() {
        let property = "gtk-interface-color-scheme";
        if settings.find_property(property).is_some() {
            let provider = provider.clone();
            settings.connect_notify_local(Some(property), move |_, _| reload(&provider));
        }
    }
    (provider, monitor)
}
pub fn reload(provider: &gtk::CssProvider) {
    let value: Value = fs::read_to_string(path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    let prefs = Value::Object(crate::config::read_settings().unwrap_or_default());
    let palette_is_light = value["surface"]
        .as_str()
        .and_then(|color| gtk::gdk::RGBA::parse(color).ok())
        .is_some_and(|color| {
            0.2126 * color.red() + 0.7152 * color.green() + 0.0722 * color.blue() > 0.5
        });
    let settings = gtk::Settings::default();
    let light = match prefs["theme_mode"].as_str() {
        Some("light") => true,
        Some("dark") => false,
        _ => settings
            .as_ref()
            .and_then(|settings| {
                let property = "gtk-interface-color-scheme";
                settings.find_property(property)?;
                let value = settings.property_value(property);
                let (_, variant) = gtk::glib::EnumValue::from_value(&value)?;
                match variant.nick() {
                    "light" => Some(true),
                    "dark" => Some(false),
                    _ => None,
                }
            })
            .unwrap_or(palette_is_light),
    };
    let color = |k: &str, fallback: &str| {
        let alternate = match prefs["accent"].as_u64().unwrap_or(0) {
            1 => "secondary",
            2 => "tertiary",
            _ => "primary",
        };
        let mapped = if k.contains("primary") {
            k.replace("primary", alternate)
        } else {
            k.into()
        };
        let k = mapped.as_str();
        value
            .get(k)
            .and_then(Value::as_str)
            .filter(|s| {
                s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
            })
            .unwrap_or(fallback)
            .to_string()
    };
    let pairs = [
        ("surface", "#141313"),
        ("surface_container_lowest", "#0f0e0e"),
        ("surface_container_low", "#1c1b1c"),
        ("surface_container", "#201f20"),
        ("surface_container_high", "#2b2a2a"),
        ("surface_container_highest", "#363435"),
        ("primary", "#cbc4cb"),
        ("on_primary", "#322f34"),
        ("primary_container", "#2d2a2f"),
        ("on_primary_container", "#bcb6bc"),
        ("on_surface", "#e6e1e1"),
        ("on_surface_variant", "#cbc5ca"),
        ("outline", "#a49ca3"),
        ("outline_variant", "#49464a"),
        ("secondary_container", "#4d4b4d"),
        ("on_secondary_container", "#ece6e9"),
        ("tertiary_container", "#31292b"),
        ("on_tertiary_container", "#c1b4b7"),
        ("error", "#ffb4ab"),
    ];
    let light_pairs = [
        ("surface", "#fff8fa"),
        ("surface_container_lowest", "#ffffff"),
        ("surface_container_low", "#fcf1f5"),
        ("surface_container", "#f6ebef"),
        ("surface_container_high", "#f0e5e9"),
        ("surface_container_highest", "#ebe0e4"),
        ("primary", "#5d5463"),
        ("on_primary", "#ffffff"),
        ("primary_container", "#e9deee"),
        ("on_primary_container", "#302835"),
        ("on_surface", "#211b20"),
        ("on_surface_variant", "#4c444b"),
        ("outline", "#7f747c"),
        ("outline_variant", "#d0c4cc"),
        ("secondary_container", "#eadee6"),
        ("on_secondary_container", "#302631"),
        ("tertiary_container", "#f4dce2"),
        ("on_tertiary_container", "#39292f"),
        ("error", "#ba1a1a"),
    ];
    let mut css = scaled_stylesheet(prefs["text_size"].as_u64().unwrap_or(1));
    let source = if light { light_pairs } else { pairs };
    let variant = prefs["palette_variant"].as_u64().unwrap_or(0);
    let custom = match prefs["palette_source"].as_str() {
        Some("custom") => {
            let seed = seed_color(&prefs["theme_seed"]);
            let rgb =
                Rgb::from_u32(u32::from_str_radix(&seed[1..], 16).expect("validated color seed"));
            Some(seeded_roles(rgb, variant, light))
        }
        Some("cover") => COVER_SEED
            .with(|seed| seed.get())
            .map(|seed| seeded_roles(seed, variant, light)),
        _ => None,
    };
    let mut roles: std::collections::BTreeMap<_, _> = source
        .into_iter()
        .map(|(key, fallback)| {
            let role = if let Some(scheme) = &custom {
                scheme[key].clone()
            } else if light == palette_is_light {
                color(key, fallback)
            } else if matches!(
                key,
                "primary" | "primary_container" | "secondary_container" | "tertiary_container"
            ) {
                mix(fallback, &color(key, fallback), 0.18)
            } else {
                fallback.to_string()
            };
            (key, role)
        })
        .collect();

    if !light && prefs["deep_dark"].as_bool().unwrap_or(false) {
        for (role, weight) in [
            ("surface", 1.),
            ("surface_container_lowest", 0.9),
            ("surface_container_low", 0.65),
            ("surface_container", 0.50),
            ("surface_container_high", 0.40),
            ("surface_container_highest", 0.32),
        ] {
            roles.insert(role, mix(&roles[role], "#000000", weight));
        }
    }

    for (foreground, background, minimum) in [
        ("on_surface", "surface", 4.5),
        ("on_surface_variant", "surface_container", 4.5),
        ("on_primary", "primary", 4.5),
        ("on_primary_container", "primary_container", 4.5),
        ("on_secondary_container", "secondary_container", 4.5),
        ("on_tertiary_container", "tertiary_container", 4.5),
        ("error", "surface", 4.5),
        ("outline", "surface_container_highest", 3.0),
    ] {
        let bg = luminance(&roles[background]);
        let fg = luminance(&roles[foreground]);
        if contrast(fg, bg) < minimum {
            let fallback = if contrast(1., bg) >= contrast(0., bg) {
                "#ffffff"
            } else {
                "#000000"
            };
            roles.insert(foreground, fallback.into());
        }
    }
    for (key, role) in roles {
        css = format!("@define-color hm_{key} {role};\n{css}");
    }
    let changed = APPLIED_CSS.with(|applied| {
        let mut applied = applied.borrow_mut();
        if applied.as_ref().is_some_and(|(old_provider, old_css)| {
            old_provider.upgrade().as_ref() == Some(provider) && old_css == &css
        }) {
            return false;
        }
        provider.load_from_string(&css);
        *applied = Some((provider.downgrade(), css));
        true
    });
    if changed {
        repaint_controls();
    }
}

pub fn repaint_controls() {
    for window in gtk::Window::list_toplevels() {
        invalidate_painted_colors(&window);
    }
}

fn invalidate_painted_colors(widget: &gtk::Widget) {
    if widget.has_css_class("theme-painted") {
        widget.queue_draw();
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        invalidate_painted_colors(&widget);
    }
}

pub fn seed_color(value: &Value) -> &str {
    value
        .as_str()
        .filter(|value| {
            value.len() == 7
                && value.starts_with('#')
                && value[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .unwrap_or("#6750a4")
}

fn seeded_roles(
    seed: Rgb,
    variant: u64,
    light: bool,
) -> std::collections::BTreeMap<&'static str, String> {
    let variant = match variant {
        1 => Variant::Vibrant,
        2 => Variant::Expressive,
        3 => Variant::Monochrome,
        4 => Variant::Neutral,
        5 => Variant::Fidelity,
        6 => Variant::Content,
        7 => Variant::Rainbow,
        8 => Variant::FruitSalad,
        _ => Variant::TonalSpot,
    };
    let theme = ThemeBuilder::with_source(seed).variant(variant).build();
    let Scheme {
        surface,
        surface_container_lowest,
        surface_container_low,
        surface_container,
        surface_container_high,
        surface_container_highest,
        primary,
        on_primary,
        primary_container,
        on_primary_container,
        on_surface,
        on_surface_variant,
        outline,
        outline_variant,
        secondary_container,
        on_secondary_container,
        tertiary_container,
        on_tertiary_container,
        error,
        ..
    } = if light {
        theme.schemes.light
    } else {
        theme.schemes.dark
    };
    [
        ("surface", surface),
        ("surface_container_lowest", surface_container_lowest),
        ("surface_container_low", surface_container_low),
        ("surface_container", surface_container),
        ("surface_container_high", surface_container_high),
        ("surface_container_highest", surface_container_highest),
        ("primary", primary),
        ("on_primary", on_primary),
        ("primary_container", primary_container),
        ("on_primary_container", on_primary_container),
        ("on_surface", on_surface),
        ("on_surface_variant", on_surface_variant),
        ("outline", outline),
        ("outline_variant", outline_variant),
        ("secondary_container", secondary_container),
        ("on_secondary_container", on_secondary_container),
        ("tertiary_container", tertiary_container),
        ("on_tertiary_container", on_tertiary_container),
        ("error", error),
    ]
    .into_iter()
    .map(|(role, rgb)| {
        (
            role,
            format!("#{:02x}{:02x}{:02x}", rgb.red, rgb.green, rgb.blue),
        )
    })
    .collect()
}

fn channels(color: &str) -> [f64; 3] {
    [1, 3, 5].map(|i| u8::from_str_radix(&color[i..i + 2], 16).unwrap() as f64 / 255.)
}

fn mix(base: &str, accent: &str, weight: f64) -> String {
    let base = channels(base);
    let accent = channels(accent);
    let [r, g, b] =
        [0, 1, 2].map(|i| ((base[i] + (accent[i] - base[i]) * weight) * 255.).round() as u8);
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn luminance(color: &str) -> f64 {
    let [r, g, b] = channels(color).map(|value| {
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    });
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn contrast(first: f64, second: f64) -> f64 {
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

fn scaled_stylesheet(size: u64) -> String {
    let scale = match size {
        0 => 0.9,
        2 => 1.15,
        _ => return include_str!("style.css").into(),
    };
    let mut css = String::new();
    for line in include_str!("style.css").lines() {
        if let Some((prefix, rest)) = line.split_once("font-size: ")
            && !prefix.contains(".md-icon")
            && !prefix.contains(".art-symbol")
            && let Some((number, suffix)) = rest.split_once("px")
            && let Ok(number) = number.parse::<f64>()
        {
            css.push_str(&format!(
                "{prefix}font-size: {:.1}px{suffix}\n",
                number * scale
            ));
            continue;
        }
        css.push_str(line);
        css.push('\n');
    }
    css
}
