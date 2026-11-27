//! Session-bus controls for Quickshell, media keys, and playerctl. No additional runtime process.
use super::{
    Item, Ui,
    api::{Api, Track},
};
use gtk::{gio, glib, prelude::*};
use std::{
    cell::RefCell,
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    io::Read,
    rc::Rc,
    time::Duration,
};

const PATH: &str = "/org/mpris/MediaPlayer2";
const ROOT: &str = "org.mpris.MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const QUEUE: &str = "io.github.balsammist.HomeMusic.Queue1";
const XML: &str = r#"<node>
<interface name="org.mpris.MediaPlayer2">
 <method name="Raise"/><method name="Quit"/>
 <property name="CanQuit" type="b" access="read"/><property name="CanRaise" type="b" access="read"/>
 <property name="HasTrackList" type="b" access="read"/><property name="Identity" type="s" access="read"/>
 <property name="DesktopEntry" type="s" access="read"/>
 <property name="SupportedUriSchemes" type="as" access="read"/><property name="SupportedMimeTypes" type="as" access="read"/>
</interface>
<interface name="org.mpris.MediaPlayer2.Player">
 <method name="Next"/><method name="Previous"/><method name="Pause"/><method name="PlayPause"/><method name="Stop"/><method name="Play"/>
 <method name="Seek"><arg name="Offset" type="x" direction="in"/></method>
 <method name="SetPosition"><arg name="TrackId" type="o" direction="in"/><arg name="Position" type="x" direction="in"/></method>
 <signal name="Seeked"><arg name="Position" type="x"/></signal>
 <property name="PlaybackStatus" type="s" access="read"/><property name="Rate" type="d" access="readwrite"/>
 <property name="Metadata" type="a{sv}" access="read"/><property name="Volume" type="d" access="readwrite"/>
 <property name="Position" type="x" access="read"/><property name="MinimumRate" type="d" access="read"/>
 <property name="MaximumRate" type="d" access="read"/>
 <property name="CanGoNext" type="b" access="read"/><property name="CanGoPrevious" type="b" access="read"/>
 <property name="CanPlay" type="b" access="read"/><property name="CanPause" type="b" access="read"/>
 <property name="CanSeek" type="b" access="read"/><property name="CanControl" type="b" access="read"/>
</interface>
<interface name="io.github.balsammist.HomeMusic.Queue1">
 <property name="Ready" type="b" access="read">
  <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="false"/>
 </property>
 <method name="List">
  <arg name="Revision" type="s" direction="out"/>
  <arg name="Tracks" type="a(ussssb)" direction="out"/>
 </method>
 <method name="Remove">
  <arg name="Position" type="u" direction="in"/><arg name="Revision" type="s" direction="in"/>
 </method>
 <method name="Add">
  <arg name="Ref" type="s" direction="in"/><arg name="Mode" type="s" direction="in"/>
  <arg name="Status" type="s" direction="out"/>
 </method>
</interface></node>"#;

pub(super) struct Mpris {
    connection: gio::DBusConnection,
    _owner: gio::OwnerId,
    _objects: Vec<gio::RegistrationId>,
    last: RefCell<String>,
}
pub(super) fn install(ui: &Rc<Ui>) -> Option<Mpris> {
    let connection = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok()?;
    let node = gio::DBusNodeInfo::for_xml(XML).ok()?;
    let mut objects = vec![];
    for name in [ROOT, PLAYER] {
        let info = node.lookup_interface(name)?;
        let weak = Rc::downgrade(ui);
        let props = Rc::downgrade(ui);
        let set = Rc::downgrade(ui);
        let id = connection
            .register_object(PATH, &info)
            .method_call(move |conn, _, _, _, method, args, invocation| {
                let Some(ui) = weak.upgrade() else {
                    invocation
                        .return_dbus_error("org.mpris.MediaPlayer2.Error.Closed", "Player closed");
                    return;
                };
                match method {
                    "Raise" => ui.window.present(),
                    "Quit" => {
                        invocation.return_value(Some(&().to_variant()));
                        ui.window.close();
                        return;
                    }
                    "PlayPause" => ui.toggle_play(),
                    "Play" => {
                        if ui.resolving_pause.get().unwrap_or_else(|| {
                            ui.current.borrow().is_none()
                                || ui.player.flag("pause")
                                || ui.player.flag("idle-active")
                        }) {
                            ui.toggle_play();
                        }
                    }
                    "Pause" => {
                        if ui.resolving_pause.get().is_some() {
                            ui.resolving_pause.set(Some(true));
                        } else {
                            ui.player.set("pause", "yes");
                        }
                        ui.update_play_control();
                    }
                    "Stop" => {
                        if ui.started.get() {
                            ui.report("interrupted");
                        }
                        ui.resolve.set(ui.resolve.get() + 1);
                        ui.resolving_pause.set(None);
                        ui.restore_position.set(None);
                        ui.player.command(&["stop"]);
                        ui.reset_timeline(ui.timeline_duration(), 0.);
                        ui.started.set(false);
                        ui.playing.set(false);
                        ui.update_play_control();
                    }
                    "Next" => ui.next(false),
                    "Previous" => {
                        if ui.player.number("time-pos") > 3. {
                            ui.seek_to(0.);
                        } else if ui.index.get() > 0 {
                            ui.play_index(ui.index.get() - 1);
                        }
                    }
                    "Seek" => {
                        if let Some((offset,)) = args.get::<(i64,)>() {
                            let position = (ui.player.number("time-pos") + offset as f64 / 1e6)
                                .clamp(0., ui.timeline_duration());
                            if ui.seek_to(position) {
                                let _ = conn.emit_signal(
                                    None,
                                    PATH,
                                    PLAYER,
                                    "Seeked",
                                    Some(&((position * 1e6) as i64,).to_variant()),
                                );
                            }
                        }
                    }
                    "SetPosition" => {
                        if let Some((track, position)) =
                            args.get::<(glib::variant::ObjectPath, i64)>()
                        {
                            let position = position as f64 / 1e6;
                            if track.as_str() == track_id(&ui)
                                && (0. ..=ui.timeline_duration()).contains(&position)
                                && ui.seek_to(position)
                            {
                                let _ = conn.emit_signal(
                                    None,
                                    PATH,
                                    PLAYER,
                                    "Seeked",
                                    Some(&((position * 1e6) as i64,).to_variant()),
                                );
                            }
                        }
                    }
                    _ => {
                        invocation.return_dbus_error(
                            "org.mpris.MediaPlayer2.Error.NotSupported",
                            "Not supported",
                        );
                        return;
                    }
                }
                invocation.return_value(Some(&().to_variant()));
                if let Some(m) = ui.mpris.borrow().as_ref() {
                    m.publish(&ui);
                }
            })
            .property(move |_, _, _, interface, property| {
                let Some(ui) = props.upgrade() else {
                    return false.to_variant();
                };
                if interface == ROOT {
                    match property {
                        "CanQuit" | "CanRaise" => true.to_variant(),
                        "HasTrackList" => false.to_variant(),
                        "Identity" => "HomeMusic".to_variant(),
                        "DesktopEntry" => "homemusic-native".to_variant(),
                        _ => Vec::<String>::new().to_variant(),
                    }
                } else {
                    property_value(&ui, property)
                }
            })
            .set_property(move |_, _, _, _, property, value| {
                let Some(ui) = set.upgrade() else {
                    return false;
                };
                match property {
                    "Volume" => {
                        if let Some(volume) = value.get::<f64>() {
                            if volume.is_finite() {
                                ui.volume_control.set_value(volume.clamp(0., 1.) * 100.);
                                return true;
                            }
                        }
                        false
                    }
                    "Rate" => value.get::<f64>() == Some(1.),
                    _ => false,
                }
            })
            .build()
            .ok()?;
        objects.push(id);
    }
    let weak = Rc::downgrade(ui);
    let ready = weak.clone();
    objects.push(
        connection
            .register_object(PATH, &node.lookup_interface(QUEUE)?)
            .method_call(move |_, _, _, _, method, args, invocation| {
                let Some(ui) = weak.upgrade() else {
                    queue_error(&invocation, "Closed", "HomeMusic закрыт.");
                    return;
                };
                match method {
                    "List" => {
                        let (revision, rows) = queue_snapshot(&ui);
                        invocation.return_value(Some(&(revision, rows).to_variant()));
                    }
                    "Remove" => {
                        let Some((position, revision)) = args.get::<(u32, String)>() else {
                            queue_error(&invocation, "InvalidArguments", "Укажи номер в очереди.");
                            return;
                        };
                        if queue_snapshot(&ui).0 != revision {
                            queue_error(
                                &invocation,
                                "QueueChanged",
                                "Очередь изменилась. Проверь номера ещё раз.",
                            );
                            return;
                        }
                        let Some(index) = (position as usize).checked_sub(1) else {
                            queue_error(
                                &invocation,
                                "InvalidPosition",
                                "Нет такого номера в очереди.",
                            );
                            return;
                        };
                        let start = ui
                            .current
                            .borrow()
                            .as_ref()
                            .map_or(0, |_| ui.index.get() + 1);
                        if ui.remove_queue_entry(start + index) {
                            invocation.return_value(Some(&().to_variant()));
                        } else {
                            queue_error(
                                &invocation,
                                "InvalidPosition",
                                "Нет такого номера в очереди.",
                            );
                        }
                    }
                    "Add" => match args.get::<(String, String)>() {
                        Some((reference, mode))
                            if matches!(mode.as_str(), "now" | "next" | "end")
                                && !reference.trim().is_empty()
                                && reference.len() <= 2048 =>
                        {
                            add_by_ref(&ui, reference, mode, invocation)
                        }
                        _ => queue_error(
                            &invocation,
                            "InvalidArguments",
                            "Укажи ссылку на трек и режим очереди.",
                        ),
                    },
                    _ => queue_error(&invocation, "NotSupported", "Неизвестная команда очереди."),
                }
            })
            .property(move |_, _, _, _, _| {
                ready.upgrade().is_some_and(|ui| ui.backend_ready()).to_variant()
            })
            .build()
            .ok()?,
    );
    let name = if ui.test {
        format!(
            "org.mpris.MediaPlayer2.homemusic.test.p{}",
            std::process::id()
        )
    } else {
        "org.mpris.MediaPlayer2.homemusic".into()
    };
    let owner = gio::bus_own_name_on_connection(
        &connection,
        &name,
        gio::BusNameOwnerFlags::NONE,
        |_, _| {},
        |_, _| {},
    );
    Some(Mpris {
        connection,
        _owner: owner,
        _objects: objects,
        last: RefCell::new(String::new()),
    })
}

type QueueRow = (u32, String, String, String, String, bool);

fn queue_snapshot(ui: &Ui) -> (String, Vec<QueueRow>) {
    let playing = ui.current.borrow().is_some();
    let index = ui.index.get();
    let mut revision = DefaultHasher::new();
    playing.hash(&mut revision);
    index.hash(&mut revision);
    ui.playback.borrow().hash(&mut revision);
    let rows = ui
        .queue
        .borrow()
        .iter()
        .skip(index)
        .enumerate()
        .map(|(offset, t)| {
            t.reference.hash(&mut revision);
            t.playback_ref.hash(&mut revision);
            let position = offset + usize::from(!playing);
            (
                position as u32,
                t.reference.clone(),
                t.title.clone(),
                t.artist.clone(),
                t.album_title.clone().unwrap_or_default(),
                position == 0,
            )
        })
        .collect();
    (format!("{:016x}", revision.finish()), rows)
}

fn queue_error(invocation: &gio::DBusMethodInvocation, name: &str, message: &str) {
    invocation
        .clone()
        .return_dbus_error(&format!("{QUEUE}.Error.{name}"), message);
}

fn add_by_ref(ui: &Rc<Ui>, reference: String, mode: String, invocation: gio::DBusMethodInvocation) {
    let known = ui
        .queue
        .borrow()
        .iter()
        .find(|track| track.reference == reference)
        .cloned()
        .or_else(|| {
            ui.items.borrow().iter().find_map(|item| match item {
                Item::Track(track) if track.reference == reference => Some(track.clone()),
                _ => None,
            })
        });
    // Pin selection is copied on the GTK thread; the worker only talks to the Hub.
    let pinned = ui.pinned(Track {
        reference: reference.clone(),
        ..Track::default()
    });
    let api = ui.api.clone();
    let (tx, rx) = async_channel::bounded(1);
    if !ui.net.run(move || {
        let result = (|| {
            let mut track = match known {
                Some(track) => track,
                None => {
                    let encoded = glib::uri_escape_string(&reference, None::<&str>, false);
                    api.get_with_timeout(
                        &format!("/api/catalog/track/{encoded}"),
                        Duration::from_secs(12),
                    )
                    .ok()
                    .and_then(|value| serde_json::from_value::<Track>(value).ok())
                    .filter(|track| track.reference == reference && !track.title.is_empty())
                    .ok_or("TrackNotFound")?
                }
            };
            let source = pinned
                .playback_ref
                .as_deref()
                .or(track.playback_ref.as_deref())
                .unwrap_or(&track.reference);
            verify_full_audio(&api, source)?;
            track.require_full_audio = true;
            Ok(track)
        })();
        let _ = tx.send_blocking(result);
    }) {
        queue_error(&invocation, "Busy", "HomeMusic занят. Повтори команду.");
        return;
    }
    let weak = Rc::downgrade(ui);
    glib::spawn_future_local(async move {
        let (Some(ui), Ok(result)) = (weak.upgrade(), rx.recv().await) else {
            queue_error(&invocation, "Closed", "HomeMusic закрыт.");
            return;
        };
        let track = match result {
            Ok(track) => track,
            Err(code) => {
                let detail = match code {
                    "PreviewOnly" => "Доступно только превью. Полную запись не удалось открыть.",
                    "TrackNotFound" => "Метаданные трека недоступны в HomeMusic.",
                    _ => "Полная запись сейчас недоступна в HomeMusic.",
                };
                queue_error(&invocation, code, detail);
                return;
            }
        };
        let status = if mode == "now" {
            let position = ui.insert_in_queue(track, true);
            ui.play_index(position);
            "starting"
        } else if ui.enqueue(track, mode == "next") {
            "queued"
        } else {
            "duplicate"
        };
        invocation.return_value(Some(&(status,).to_variant()));
    });
}

fn verify_full_audio(api: &Api, reference: &str) -> Result<(), &'static str> {
    let mut response = api
        .agent
        .get(format!("{}/api/listen/stream", api.base))
        .query("ref", reference)
        .query("full", "1")
        .header("Cookie", api.cookie())
        .header("Range", "bytes=0-4095")
        .header("X-HomeMusic-Prefetch", "1")
        .config()
        .timeout_global(Some(Duration::from_secs(45)))
        .http_status_as_error(false)
        .build()
        .call()
        .map_err(|_| "Unavailable")?;
    let kind = response
        .headers()
        .get("X-HomeMusic-Listen-Kind")
        .and_then(|value| value.to_str().ok());
    let full = matches!(kind, Some("full" | "library"));
    if kind == Some("preview")
        || (response.status() == 503
            && response
                .body_mut()
                .read_json::<serde_json::Value>()
                .is_ok_and(|value| {
                    value["detail"]
                        .as_str()
                        .is_some_and(|detail| detail.starts_with("Полная версия пока недоступна."))
                }))
    {
        return Err("PreviewOnly");
    }
    if full
        && response.status().is_success()
        && response
            .body_mut()
            .as_reader()
            .read_exact(&mut [0; 1])
            .is_ok()
    {
        Ok(())
    } else {
        Err("Unavailable")
    }
}

impl Mpris {
    pub(super) fn publish(&self, ui: &Ui) {
        let key = format!(
            "{}|{}|{}|{}|{}",
            track_id(ui),
            status(ui),
            ui.player.number("volume"),
            ui.player.number("duration"),
            can_go_next(ui)
        );
        if *self.last.borrow() == key {
            return;
        }
        *self.last.borrow_mut() = key;
        let changed: HashMap<String, glib::Variant> = [
            "Metadata",
            "PlaybackStatus",
            "Volume",
            "CanGoNext",
            "CanGoPrevious",
            "CanPlay",
            "CanPause",
            "CanSeek",
        ]
        .into_iter()
        .map(|name| (name.to_string(), property_value(ui, name)))
        .collect();
        let _ = self.connection.emit_signal(
            None,
            PATH,
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            Some(&(PLAYER, changed, Vec::<String>::new()).to_variant()),
        );
    }
}
fn track_id(ui: &Ui) -> String {
    let mut h = DefaultHasher::new();
    if let Some(t) = ui.current.borrow().as_ref() {
        t.reference.hash(&mut h);
    }
    format!("/io/github/balsammist/HomeMusic/track/t{:x}", h.finish())
}
fn status(ui: &Ui) -> &'static str {
    if ui.current.borrow().is_none() || ui.player.flag("idle-active") {
        "Stopped"
    } else if ui.player.flag("pause") {
        "Paused"
    } else {
        "Playing"
    }
}
fn can_go_next(ui: &Ui) -> bool {
    let len = ui.queue.borrow().len();
    len > 0
        && (ui.current.borrow().is_none()
            || ui.index.get() + 1 < len
            || ui.repeat.get() != 0
            || ui.autoplay.get()
            || ui.wave.borrow().is_some())
}
fn metadata(ui: &Ui) -> glib::Variant {
    let dict = glib::VariantDict::new(None);
    if let Some(t) = ui.current.borrow().as_ref() {
        let path = glib::variant::ObjectPath::try_from(track_id(ui)).unwrap();
        dict.insert("mpris:trackid", path);
        dict.insert(
            "mpris:length",
            (ui.player.number("duration").max(t.duration_s.unwrap_or(0.)) * 1e6) as i64,
        );
        dict.insert("xesam:title", &t.title);
        dict.insert("xesam:artist", vec![t.artist.clone()]);
        dict.insert("xesam:album", t.album_title.as_deref().unwrap_or(""));
        dict.insert("mpris:artUrl", t.cover());
    }
    dict.end()
}
fn property_value(ui: &Ui, name: &str) -> glib::Variant {
    match name {
        "PlaybackStatus" => status(ui).to_variant(),
        "Metadata" => metadata(ui),
        "Rate" | "MinimumRate" | "MaximumRate" => 1f64.to_variant(),
        "Volume" => (ui.player.number("volume") / 100.).to_variant(),
        "Position" => ((ui.player.number("time-pos") * 1e6) as i64).to_variant(),
        "CanGoNext" => can_go_next(ui).to_variant(),
        "CanGoPrevious" => (ui.index.get() > 0 || ui.player.number("time-pos") > 0.).to_variant(),
        "CanPlay" => (ui.current.borrow().is_some()
            || !ui.items.borrow().is_empty()
            || !ui.queue.borrow().is_empty())
        .to_variant(),
        "CanPause" => ui.current.borrow().is_some().to_variant(),
        "CanSeek" => (ui.player.number("duration") > 0.).to_variant(),
        _ => true.to_variant(),
    }
}
