use super::*;

#[derive(Default, Debug, PartialEq)]
struct Counts {
    rows: usize,
    matched: usize,
    waiting: usize,
    done: usize,
    not_found: usize,
    queued: usize,
    failed: usize,
    pending: usize,
}

impl Counts {
    fn add(&mut self, item: &Value) {
        if item["status"] == "skipped" {
            return;
        }
        self.rows += 1;
        if matches!(item["match"].as_str(), Some("owned" | "exact" | "probable")) {
            self.matched += 1;
        }
        match item["status"].as_str().unwrap_or("") {
            "waiting" => self.waiting += 1,
            "done" => self.done += 1,
            "not_found" => self.not_found += 1,
            "queued" => self.queued += 1,
            "failed" => self.failed += 1,
            "pending" => self.pending += 1,
            _ => {}
        }
    }
}

struct Report {
    import: Value,
    counts: Counts,
}

fn load_reports(api: &Api) -> Result<Vec<Report>, String> {
    let imports = api.get("/api/imports")?;
    let imports = imports
        .as_array()
        .ok_or("Сервер вернул некорректный список импортов")?;
    let mut reports = Vec::new();
    for summary in imports {
        let id = summary["id"]
            .as_u64()
            .ok_or("У импорта нет идентификатора")?;
        let import = api.get(&format!("/api/imports/{id}"))?;
        let selected: HashSet<u64> = import["collections"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| {
                c["selected"] == true && matches!(c["kind"].as_str(), Some("likes" | "playlist"))
            })
            .filter_map(|c| c["id"].as_u64())
            .collect();
        let mut counts = Counts::default();
        // API stats.done counts download jobs, not import rows. Read row states so
        // an already-owned song is counted correctly, including every page.
        let mut offset = 0;
        loop {
            let page = api.query(
                &format!("/api/imports/{id}/items"),
                &[("offset", &offset.to_string()), ("limit", "1000")],
            )?;
            let items = page["items"]
                .as_array()
                .ok_or("Не удалось прочитать строки импорта")?;
            for item in items {
                if item["collection_id"]
                    .as_u64()
                    .is_some_and(|id| selected.contains(&id))
                {
                    counts.add(item);
                }
            }
            offset += items.len();
            if offset >= page["total"].as_u64().unwrap_or(0) as usize {
                break;
            }
            if items.is_empty() {
                return Err("Список импорта изменился. Откройте его ещё раз.".into());
            }
        }
        reports.push(Report { import, counts });
    }
    Ok(reports)
}

impl Ui {
    pub(super) fn import_status(self: &Rc<Self>) {
        let window = gtk::Window::builder()
            .transient_for(&self.window)
            .title("Импорт музыки")
            .default_width(640)
            .default_height(580)
            .build();
        let content = import_box();
        let state = label("Загружаем состояние импортов…", "muted");
        content.append(&state);
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&content)
            .build();
        mount_dialog(&window, &scroll);
        let (tx, rx) = async_channel::bounded(1);
        let api = self.api.clone();
        self.net.run(move || {
            let _ = tx.send_blocking(load_reports(&api));
        });
        let weak_window = window.downgrade();
        let weak_ui = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Ok(result) = rx.recv().await else {
                return;
            };
            if weak_window.upgrade().is_none() {
                return;
            }
            match result {
                Err(error) => state.set_text(&error),
                Ok(reports) => {
                    content.remove(&state);
                    if reports.is_empty() {
                        content.append(&label("Импортов пока нет", "muted"));
                    }
                    for Report { import, counts } in reports {
                        let id = import["id"].as_u64().unwrap_or(0);
                        let source = match import["source"].as_str() {
                            Some("spotify") => "Spotify",
                            Some("yandex") => "Яндекс Музыка",
                            _ => "Музыка",
                        };
                        let status = match import["status"].as_str().unwrap_or("") {
                            "done" => "Обработан",
                            "running" => "Обрабатывается",
                            "preview" => "Ожидает запуска",
                            "failed" => "Ошибка",
                            "cancelled" => "Остановлен",
                            "fetching" => "Читаем коллекции",
                            "auth" => "Ожидает входа",
                            _ => "Неизвестное состояние",
                        };
                        content.append(&label(
                            &format!("{source} · импорт #{id} · {status}"),
                            "section-title",
                        ));
                        for collection in import["collections"].as_array().into_iter().flatten() {
                            if collection["selected"] != true
                                || !matches!(
                                    collection["kind"].as_str(),
                                    Some("likes" | "playlist")
                                )
                            {
                                continue;
                            }
                            let kind = if collection["kind"] == "likes" {
                                "Любимые"
                            } else {
                                "Плейлист"
                            };
                            let row = label(
                                &format!(
                                    "{kind}: {} · {} треков",
                                    text(collection, "title"),
                                    collection["count"].as_u64().unwrap_or(0)
                                ),
                                "muted",
                            );
                            row.set_wrap(true);
                            content.append(&row);
                        }
                        let numbers = label(
                            &format!(
                                "Треков: {} · сопоставлено: {}\nВ библиотеке: {} · ждут прослушивания: {}\nНенайдено: {} · в очереди: {} · ошибки: {} · проверяются: {}",
                                counts.rows,
                                counts.matched,
                                counts.done,
                                counts.waiting,
                                counts.not_found,
                                counts.queued,
                                counts.failed,
                                counts.pending
                            ),
                            "",
                        );
                        numbers.set_wrap(true);
                        content.append(&numbers);
                        if import["mode"] == "economy" {
                            let economy = label(
                                "Экономный режим: найденные записи скачиваются после прослушивания. Любимые можно хранить без скачивания.",
                                "small muted",
                            );
                            economy.set_wrap(true);
                            content.append(&economy);
                        }
                        if let Some(error) = import["error"].as_str().filter(|s| !s.is_empty()) {
                            let error = label(error, "error");
                            error.set_wrap(true);
                            content.append(&error);
                        }
                        if counts.not_found > 0 {
                            let unmatched =
                                button("Ненайденные", Some("system-search-symbolic"), "tonal");
                            unmatched.set_halign(gtk::Align::Start);
                            let weak_ui = weak_ui.clone();
                            let status_window = weak_window.clone();
                            unmatched.connect_clicked(move |_| {
                                if let Some(ui) = weak_ui.upgrade() {
                                    ui.import_unmatched(id);
                                    if let Some(window) = status_window.upgrade() {
                                        window.close();
                                    }
                                }
                            });
                            content.append(&unmatched);
                        }
                        content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
                    }
                }
            }
        });
        window.present();
    }

    fn import_unmatched(self: &Rc<Self>, id: u64) {
        let window = gtk::Window::builder()
            .transient_for(&self.window)
            .title(format!("Ненайденные · импорт #{id}"))
            .default_width(620)
            .default_height(560)
            .build();
        let content = import_box();
        let state = label("Загружаем ненайденные треки…", "muted");
        content.append(&state);
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&content)
            .build();
        mount_dialog(&window, &scroll);
        let api = self.api.clone();
        let (tx, rx) = async_channel::bounded(1);
        self.net.run(move || {
            let result = (|| -> Result<Vec<Value>, String> {
                let mut rows = Vec::new();
                loop {
                    let page = api.query(
                        &format!("/api/imports/{id}/items"),
                        &[
                            ("status", "not_found"),
                            ("offset", &rows.len().to_string()),
                            ("limit", "1000"),
                        ],
                    )?;
                    let items = page["items"]
                        .as_array()
                        .ok_or("Некорректный список ненайденных треков")?;
                    rows.extend(items.iter().cloned());
                    if rows.len() >= page["total"].as_u64().unwrap_or(0) as usize {
                        return Ok(rows);
                    }
                    if items.is_empty() {
                        return Err("Список изменился. Откройте его ещё раз.".into());
                    }
                }
            })();
            let _ = tx.send_blocking(result);
        });
        let weak_window = window.downgrade();
        let weak_ui = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Ok(result) = rx.recv().await else {
                return;
            };
            if weak_window.upgrade().is_none() {
                return;
            }
            match result {
                Err(error) => state.set_text(&error),
                Ok(items) => {
                    state.set_text(&format!(
                        "Ненайдено: {}. Нажмите на запись, чтобы искать вручную.",
                        items.len()
                    ));
                    for item in items {
                        let query = format!("{} {}", text(&item, "artist"), text(&item, "title"));
                        let b = button("", None, "menu-item");
                        let name = label(
                            &format!("{} — {}", text(&item, "artist"), text(&item, "title")),
                            "",
                        );
                        name.set_wrap(true);
                        name.set_xalign(0.);
                        b.set_child(Some(&name));
                        let weak_ui = weak_ui.clone();
                        let weak_window = weak_window.clone();
                        b.connect_clicked(move |_| {
                            if let Some(ui) = weak_ui.upgrade() {
                                ui.search.set_text(query.trim());
                                ui.search.emit_activate();
                            }
                            if let Some(window) = weak_window.upgrade() {
                                window.close();
                            }
                        });
                        content.append(&b);
                    }
                }
            }
        });
        window.present();
    }
}

fn import_box() -> gtk::Box {
    let content = vbox(14);
    content.set_margin_top(24);
    content.set_margin_bottom(24);
    content.set_margin_start(24);
    content.set_margin_end(24);
    content
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counters_use_import_rows_including_owned_tracks() {
        let mut counts = Counts::default();
        for value in [
            json!({"status":"done", "match":"owned"}),
            json!({"status":"done", "match":"exact"}),
            json!({"status":"waiting", "match":"probable"}),
            json!({"status":"not_found", "match":"not_found"}),
            json!({"status":"skipped", "match":"exact"}),
        ] {
            counts.add(&value);
        }
        assert_eq!(
            counts,
            Counts {
                rows: 4,
                matched: 3,
                done: 2,
                waiting: 1,
                not_found: 1,
                ..Counts::default()
            }
        );
    }
}
