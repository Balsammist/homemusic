//! StatusNotifierItem for the desktop tray, using GTK's session bus connection.
use gtk::{gdk_pixbuf::Pixbuf, gio, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const PATH: &str = "/StatusNotifierItem";
const XML: &str = r#"<node><interface name="org.kde.StatusNotifierItem">
 <method name="Activate"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
 <property name="Category" type="s" access="read"/>
 <property name="Id" type="s" access="read"/>
 <property name="Title" type="s" access="read"/>
 <property name="Status" type="s" access="read"/>
 <property name="WindowId" type="i" access="read"/>
 <property name="IconName" type="s" access="read"/>
 <property name="IconPixmap" type="a(iiay)" access="read"/>
 <property name="ItemIsMenu" type="b" access="read"/>
</interface></node>"#;

pub(super) fn install(
    app: &gtk::Application,
    window: &gtk::ApplicationWindow,
    button: &gtk::Button,
) {
    let registered = Rc::new(Cell::new(false));
    let weak = window.downgrade();
    let available = registered.clone();
    button.connect_clicked(move |_| {
        if let Some(window) = weak.upgrade() {
            if available.get() {
                window.set_visible(false);
            } else {
                window.minimize();
            }
        }
    });

    let Some(connection) = app.dbus_connection() else {
        return;
    };
    let interface = gio::DBusNodeInfo::for_xml(XML)
        .expect("valid StatusNotifierItem introspection")
        .lookup_interface("org.kde.StatusNotifierItem")
        .expect("StatusNotifierItem interface");
    let weak = window.downgrade();
    let pixmaps = icon_pixmaps();
    let icon_name = if pixmaps.is_empty() {
        "audio-x-generic"
    } else {
        ""
    };
    let pixmaps = pixmaps.to_variant();
    let object = match connection
        .register_object(PATH, &interface)
        .method_call(move |_, _, _, _, _, _, invocation| {
            if let Some(window) = weak.upgrade() {
                window.present();
            }
            invocation.return_value(None);
        })
        .property(move |_, _, _, _, property| match property {
            "Category" => "ApplicationStatus".to_variant(),
            "Id" => "homemusic-native".to_variant(),
            "Title" => "HomeMusic".to_variant(),
            "Status" => "Active".to_variant(),
            "WindowId" => 0i32.to_variant(),
            "IconName" => icon_name.to_variant(),
            "IconPixmap" => pixmaps.clone(),
            "ItemIsMenu" => false.to_variant(),
            _ => unreachable!("undeclared StatusNotifierItem property"),
        })
        .build()
    {
        Ok(object) => object,
        Err(error) => {
            eprintln!("Could not export HomeMusic tray item: {error}");
            return;
        }
    };

    let generation = Rc::new(Cell::new(0u64));
    let current_generation = generation.clone();
    let available = registered.clone();
    let weak = button.downgrade();
    let reset_button = weak.clone();
    let watcher = gio::bus_watch_name_on_connection(
        &connection,
        WATCHER,
        gio::BusNameWatcherFlags::NONE,
        move |connection, _, owner| {
            let id = generation.get();
            let generation = generation.clone();
            let registered = registered.clone();
            let weak = weak.clone();
            connection.call(
                Some(owner),
                "/StatusNotifierWatcher",
                WATCHER,
                "RegisterStatusNotifierItem",
                Some(&(PATH,).to_variant()),
                None,
                gio::DBusCallFlags::NO_AUTO_START,
                3000,
                gio::Cancellable::NONE,
                move |result| {
                    if generation.get() == id && result.is_ok() {
                        registered.set(true);
                        if let Some(button) = weak.upgrade() {
                            button.set_tooltip_text(Some("Свернуть в трей"));
                        }
                    }
                },
            );
        },
        move |_, _| {
            current_generation.set(current_generation.get() + 1);
            available.set(false);
            if let Some(button) = reset_button.upgrade() {
                button.set_tooltip_text(Some("Свернуть"));
            }
        },
    );

    let registrations = RefCell::new(Some((watcher, object)));
    app.connect_shutdown(move |_| {
        if let Some((watcher, object)) = registrations.take() {
            gio::bus_unwatch_name(watcher);
            let _ = connection.unregister_object(object);
        }
    });
}

fn icon_pixmaps() -> Vec<(i32, i32, Vec<u8>)> {
    let asset = glib::Bytes::from_static(include_bytes!("../../assets/homemusic.svg"));
    let stream = gio::MemoryInputStream::from_bytes(&asset);
    let Ok(icon) = Pixbuf::from_stream_at_scale(&stream, 32, 32, true, gio::Cancellable::NONE)
    else {
        return Vec::new();
    };
    let (width, height, stride, channels) = (
        icon.width(),
        icon.height(),
        icon.rowstride(),
        icon.n_channels(),
    );
    let rgba = icon.read_pixel_bytes();
    let mut argb = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        let start = (row * stride) as usize;
        let end = start + (width * channels) as usize;
        for pixel in rgba[start..end].chunks_exact(channels as usize) {
            // StatusNotifierItem uses network-order ARGB32, while GdkPixbuf uses RGBA.
            argb.extend_from_slice(&[
                pixel.get(3).copied().unwrap_or(255),
                pixel[0],
                pixel[1],
                pixel[2],
            ]);
        }
    }
    vec![(width, height, argb)]
}
