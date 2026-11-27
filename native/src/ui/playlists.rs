use super::*;
impl Ui {
    pub(super) fn playlist_menu(self: &Rc<Self>, path: String, name: String, anchor: &gtk::Widget) {
        let pop = gtk::Popover::new();
        pop.set_parent(anchor);
        pop.add_css_class("material-menu");
        pop.set_has_arrow(false);
        let actions = vbox(2);
        for (title, delete) in [("Переименовать", false), ("Удалить плейлист", true)]
        {
            let b = button(title, None, "menu-item");
            let u = self.clone();
            let path = path.clone();
            let name = name.clone();
            let menu = pop.clone();
            b.connect_clicked(move |_| {
                menu.popdown();
                u.edit_playlist(&path, &name, delete);
            });
            actions.append(&b);
        }
        pop.set_child(Some(&ripple::wrap(&actions)));
        pop.connect_closed(|p| p.unparent());
        pop.popup();
    }
    fn edit_playlist(self: &Rc<Self>, path: &str, name: &str, delete: bool) {
        let dialog = gtk::Window::builder()
            .transient_for(&self.window)
            .modal(true)
            .title(if delete {
                "Удалить плейлист"
            } else {
                "Переименовать плейлист"
            })
            .default_width(400)
            .build();
        let content = vbox(16);
        content.set_margin_top(20);
        content.set_margin_bottom(20);
        content.set_margin_start(20);
        content.set_margin_end(20);
        let entry = gtk::Entry::new();
        entry.set_text(name);
        if delete {
            let body = label(
                &format!("Удалить «{name}»? Музыкальные файлы останутся в библиотеке."),
                "",
            );
            body.set_wrap(true);
            content.append(&body);
        } else {
            content.append(&entry);
        }
        let actions = hbox(8);
        let cancel = button("Отмена", None, "flat");
        let d = dialog.clone();
        cancel.connect_clicked(move |_| d.close());
        actions.append(&cancel);
        let apply = button(
            if delete {
                "Удалить"
            } else {
                "Сохранить"
            },
            None,
            "primary",
        );
        actions.append(&apply);
        content.append(&actions);
        mount_dialog(&dialog, &content);
        let u = self.clone();
        let path = path.to_string();
        let d = dialog.clone();
        apply.connect_clicked(move |_| {
            let api = u.api.clone();
            let tx = u.tx.clone();
            let path = path.clone();
            let body = json!({"name":entry.text().trim()});
            u.net.run(move || {
                let result = api.mutate(if delete { "DELETE" } else { "PATCH" }, &path, body);
                let _ = tx.send_blocking(Event::PlaylistChanged(path, result, delete));
            });
            d.close();
        });
        dialog.present();
    }
    pub(super) fn reorder_playlist(self: &Rc<Self>, from: usize, to: usize) -> bool {
        let path = self.page.borrow().clone();
        let count = self.items.borrow().len();
        if !path.starts_with("/api/playlists/")
            || self.sort.selected() != 0 || !self.list_filter.text().trim().is_empty()
            || from >= count
            || to >= count
        {
            return false;
        }
        let mut order: Vec<_> = (0..count).collect();
        let index = order.remove(from);
        order.insert(to, index);
        let api = self.api.clone();
        let tx = self.tx.clone();
        self.net.run(move || {
            let result = api.post(&format!("{path}/reorder"), json!({"indexes":order}));
            let _ = tx.send_blocking(Event::PlaylistChanged(path, result, false));
        });
        true
    }
    pub(super) fn move_to_playlist(self: &Rc<Self>, track: &Track, index: usize) {
        let source = self.page.borrow().clone();
        if !source.starts_with("/api/playlists/") || self.sort.selected() != 0 || !self.list_filter.text().trim().is_empty() {
            return;
        }
        self.playlist_picker(track, Some((source, index)));
    }
    pub(super) fn playlist_picker(self: &Rc<Self>, track: &Track, remove: Option<(String, usize)>) {
        let Some(song) = track.nd_song_id.clone() else {
            self.notice("Сначала скачайте трек в библиотеку", true);
            return;
        };
        let window = gtk::Window::builder()
            .transient_for(&self.window)
            .title(if remove.is_some() {
                "Переместить в плейлист"
            } else {
                "Добавить в плейлист"
            })
            .default_width(400)
            .default_height(360)
            .build();
        let list = vbox(6);
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .child(&list)
            .build();
        mount_dialog(&window, &scroll);
        let (tx, rx) = async_channel::bounded(1);
        let api = self.api.clone();
        self.net.run(move || {
            let _ = tx.send_blocking(api.get("/api/playlists"));
        });
        let u = self.clone();
        let weak = window.downgrade();
        glib::spawn_future_local(async move {
            if let Ok(Ok(v)) = rx.recv().await {
                for p in v.as_array().unwrap_or(&vec![]) {
                    let path = format!("/api/playlists/{}", text(p, "id"));
                    if remove.as_ref().is_some_and(|(s, _)| s == &path) {
                        continue;
                    }
                    let b = button(&text(p, "name"), None, "flat");
                    let api = u.api.clone();
                    let tx = u.tx.clone();
                    let song = song.clone();
                    let remove = remove.clone();
                    let u = u.clone();
                    let weak = weak.clone();
                    b.connect_clicked(move |_| {
                        let api = api.clone();
                        let tx = tx.clone();
                        let path = path.clone();
                        let song = song.clone();
                        let remove = remove.clone();
                        u.net.run(move || {
                            let result = api
                                .post(&format!("{path}/add"), json!({"song_ids":[song]}))
                                .and_then(|v| {
                                    if let Some((source, index)) = &remove {
                                        api.post(
                                            &format!("{source}/remove"),
                                            json!({"indexes":[index]}),
                                        )
                                    } else {
                                        Ok(v)
                                    }
                                });
                            if let Some((source, _)) = remove {
                                let _ =
                                    tx.send_blocking(Event::PlaylistChanged(source, result, false));
                            } else {
                                let _ = tx.send_blocking(Event::Notice(
                                    result,
                                    "Добавлено в плейлист".into(),
                                ));
                            }
                        });
                        if let Some(w) = weak.upgrade() {
                            w.close();
                        }
                    });
                    list.append(&b);
                }
            }
        });
        window.present();
    }
}
