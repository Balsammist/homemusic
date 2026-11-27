use super::*;

pub(super) fn tracks(value: &Value) -> Vec<Track> {
    let mut seen = HashSet::new();
    value.as_array().into_iter().flatten()
        .filter(|item| item["status"] == "done")
        .filter_map(|item| {
            let id = item["nd_song_id"].as_str().filter(|id| !id.is_empty())?;
            if !seen.insert(id) {
                return None;
            }
            let local = format!("nd:{id}");
            Some(Track {
                reference: item["track_ref"].as_str().filter(|reference| !reference.is_empty())
                    .map(str::to_owned).unwrap_or_else(|| local.clone()),
                playback_ref: Some(local),
                title: text(item, "title"),
                artist: text(item, "artist"),
                album_title: item["album"].as_str().map(str::to_owned),
                nd_song_id: Some(id.into()),
                owned: true,
                ..Track::default()
            })
        }).collect()
}

pub(super) fn rows(value: &Value) -> Vec<Item> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item["star"] != true)
        .filter_map(|item| {
            let id = item["id"].as_u64()?;
            Some(Item::Card {
                title: format!("{} — {}", text(item, "artist"), text(item, "title")),
                sub: description(item),
                path: format!("download:{id}"),
                cover: String::new(),
            })
        })
        .collect()
}

fn description(item: &Value) -> String {
    let status = match item["status"].as_str() {
        Some("queued") => "В очереди",
        Some("searching") => "Поиск источника",
        Some("downloading") => "Скачивание",
        Some("tagging") => "Добавление метаданных",
        Some("importing") => "Добавление в библиотеку",
        Some("done") => "Готово",
        Some("failed") => "Ошибка",
        Some("needs_review") => "Нужен другой источник",
        Some("cancelled") => "Отменено",
        _ => "Статус неизвестен",
    };
    if let Some(error) = item["error"].as_str().filter(|error| !error.is_empty()) {
        format!("{status} · {error}")
    } else if item["status"] == "downloading" {
        let progress = item["progress"].as_f64().unwrap_or(0.).clamp(0., 1.);
        format!("{status} · {:.0} %", progress * 100.)
    } else {
        status.into()
    }
}

fn failed(item: &Value) -> bool {
    matches!(item["status"].as_str(), Some("failed" | "needs_review"))
}

impl Ui {
    pub(super) fn open_download(self: &Rc<Self>, id: u64) {
        let window = gtk::Window::builder()
            .transient_for(&self.window)
            .modal(true)
            .title("Загрузка")
            .default_width(440)
            .build();
        let content = vbox(14);
        content.set_margin_top(24);
        content.set_margin_bottom(24);
        content.set_margin_start(24);
        content.set_margin_end(24);
        let title = label("", "section-title");
        title.set_wrap(true);
        let state = label("", "");
        state.set_widget_name("download-status");
        state.set_wrap(true);
        let actions = hbox(8);
        let refresh = button("Обновить", Some("view-refresh-symbolic"), "tonal");
        let retry = button("Повторить", None, "primary");
        retry.set_visible(false);
        retry.set_widget_name("download-retry");
        actions.append(&refresh);
        actions.append(&retry);
        content.append(&title);
        content.append(&state);
        content.append(&actions);
        mount_dialog(&window, &content);
        let keys = gtk::EventControllerKey::new();
        let weak = window.downgrade();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape
                && let Some(window) = weak.upgrade()
            {
                window.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(keys);

        let run = {
            let u = Rc::downgrade(self);
            let window = window.downgrade();
            let title = title.downgrade();
            let state = state.downgrade();
            let refresh = refresh.downgrade();
            let retry = retry.downgrade();
            move |submit: bool| {
                let (Some(u), Some(state), Some(refresh), Some(retry)) = (
                    u.upgrade(),
                    state.upgrade(),
                    refresh.upgrade(),
                    retry.upgrade(),
                ) else {
                    return;
                };
                let (tx, rx) = async_channel::bounded(1);
                let api = u.api.clone();
                if !u.net.run(move || {
                    let result = api.get(&format!("/api/downloads/{id}")).and_then(|item| {
                        if submit && failed(&item) {
                            api.post(&format!("/api/downloads/items/{id}/retry"), json!({}))
                        } else {
                            Ok(item)
                        }
                    });
                    let _ = tx.send_blocking(result);
                }) {
                    state.set_text("Не удалось отправить запрос");
                    return;
                }
                state.set_text("Получаем состояние загрузки…");
                refresh.set_sensitive(false);
                retry.set_sensitive(false);
                let window = window.clone();
                let title = title.clone();
                let u = Rc::downgrade(&u);
                glib::spawn_future_local(async move {
                    let Ok(result) = rx.recv().await else { return };
                    if window.upgrade().is_none() {
                        return;
                    }
                    refresh.set_sensitive(true);
                    retry.set_sensitive(true);
                    match result {
                        Ok(item) => {
                            if let Some(title) = title.upgrade() {
                                title.set_text(&format!(
                                    "{} — {}",
                                    text(&item, "artist"),
                                    text(&item, "title")
                                ));
                            }
                            state.set_text(&description(&item));
                            retry.set_visible(failed(&item));
                            if let Some(u) = u.upgrade()
                                && u.page.borrow().as_str() == "download_jobs"
                            {
                                u.request(
                                    "download_jobs".into(),
                                    "/api/downloads".into(),
                                    vec![],
                                    None,
                                );
                            }
                        }
                        Err(error) => state.set_text(&error),
                    }
                });
            }
        };
        run(false);
        let load = run.clone();
        refresh.connect_clicked(move |_| load(false));
        retry.connect_clicked(move |_| run(true));
        window.present();
    }
}
