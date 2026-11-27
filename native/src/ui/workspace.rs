use super::*;
impl Ui {
    pub(super) fn history_step(self: &Rc<Self>, direction: isize) {
        let target = self.history_index.get() as isize + direction;
        if target < 0 {
            return;
        }
        let key = self.history.borrow().get(target as usize).cloned();
        if let Some(key) = key {
            self.history_index.set(target as usize);
            self.history_move.set(true);
            self.navigate(&key, false);
            self.history_move.set(false);
        }
    }
    pub(super) fn load_sidebar(self: &Rc<Self>) {
        let api = self.api.clone();
        let tx = self.tx.clone();
        self.net.run(move || {
            let _ = tx.send_blocking(Event::Sidebar(api.get("/api/playlists")));
            let _ = tx.send_blocking(Event::PersonalMixes(api.get("/api/mixes/personal")));
        });
    }
    pub(super) fn fill_sidebar(self: &Rc<Self>, value: &Value) {
        while let Some(child) = self.sidebar_items.first_child() {
            self.sidebar_items.remove(&child);
        }
        self.sidebar_items
            .append(&label("Мои миксы дня", "small muted"));
        for (title, path) in [("Моя волна", "wave"), ("Микс дня", "mix")] {
            let b = button(
                title,
                Some("media-playlist-shuffle-symbolic"),
                "library-item",
            );
            let row = hbox(8);
            row.append(&artwork::cover(
                if path == "mix" {
                    &self.mix_pictures
                } else {
                    &self.mix_pictures[..1]
                },
                if path == "mix" { 4 } else { 9 },
                32,
            ));
            row.append(&label(title, "mix-label"));
            b.set_child(Some(&row));
            let u = self.clone();
            b.connect_clicked(move |_| {
                if path == "wave" {
                    u.start_wave()
                } else {
                    u.navigate("mix", false)
                }
            });
            self.sidebar_items.append(&b);
        }
        if self.personal_sidebar.parent().is_some() {
            self.personal_sidebar.unparent();
        }
        self.sidebar_items.append(&self.personal_sidebar);
        self.sidebar_items
            .append(&label("Плейлисты", "small muted"));
        let likes = button(
            "Любимые треки",
            Some("emblem-favorite-symbolic"),
            "library-item",
        );
        let u = self.clone();
        likes.connect_clicked(move |_| u.navigate("likes", false));
        self.sidebar_items.append(&likes);
        for p in value.as_array().unwrap_or(&vec![]) {
            let b = button(
                &text(p, "name"),
                Some("audio-x-generic-symbolic"),
                "library-item",
            );
            b.set_tooltip_text(Some(&text(p, "name")));
            let path = format!("/api/playlists/{}", text(p, "id"));
            let u = self.clone();
            let page_path = path.clone();
            b.connect_clicked(move |_| u.navigate(&page_path, false));
            let gesture = gtk::GestureClick::new();
            gesture.set_button(3);
            let u = self.clone();
            let name = text(p, "name");
            gesture.connect_pressed(move |g, _, _, _| {
                if let Some(w) = g.widget() {
                    u.playlist_menu(path.clone(), name.clone(), &w);
                }
            });
            b.add_controller(gesture);
            self.sidebar_items.append(&b);
        }
    }
    pub(super) fn fill_personal_sidebar(self: &Rc<Self>) {
        while let Some(child) = self.personal_sidebar.first_child() {
            self.personal_sidebar.remove(&child);
        }
        if let Some(groups) = self.personal_mixes.borrow()["groups"].as_array() {
            for group in groups {
                let row = hbox(8);
                let mut pictures = vec![];
                for t in api::tracks(&group["tracks"]).iter().take(4) {
                    let picture = gtk::Picture::new();
                    picture.set_size_request(32, 32);
                    self.load_cover(&t.cover(), &picture);
                    pictures.push(picture);
                }
                row.append(&artwork::cover(&pictures, 4, 32));
                let title = label(group["title"].as_str().unwrap_or("Микс"), "mix-label");
                title.set_ellipsize(gtk::pango::EllipsizeMode::End);
                title.set_max_width_chars(18);
                row.append(&title);
                let b = button("", None, "library-item");
                b.set_child(Some(&row));
                let path = format!("personal:{}", group["id"].as_str().unwrap_or(""));
                let u = self.clone();
                b.connect_clicked(move |_| u.navigate(&path, false));
                self.personal_sidebar.append(&b);
            }
        }
    }
    pub(super) fn fill_home(self: &Rc<Self>, value: &Value) {
        while let Some(child) = self.shelves.first_child() {
            self.shelves.remove(&child);
        }
        let quick = gtk::FlowBox::new();
        quick.set_selection_mode(gtk::SelectionMode::None);
        quick.set_min_children_per_line(1);
        quick.set_max_children_per_line(4);
        quick.set_homogeneous(true);
        quick.set_column_spacing(8);
        quick.set_row_spacing(8);
        for (title, path) in [
            ("Любимые треки", "likes"),
            ("Микс дня", "mix"),
            ("Моя волна", "wave"),
            ("Плейлисты", "playlists"),
        ] {
            let launch = path != "playlists";
            let b = button(
                title,
                if launch {
                    Some("media-playback-start-symbolic")
                } else {
                    Some("view-list-symbolic")
                },
                "quick-card",
            );
            b.set_hexpand(true);
            if path == "mix" || path == "wave" {
                let seeds = api::tracks(if path == "mix" {
                    &value["mix_art"]
                } else {
                    &value["daily"]
                });
                let mut pictures = vec![];
                for t in seeds.iter().take(if path == "mix" { 4 } else { 1 }) {
                    let p = gtk::Picture::new();
                    p.set_size_request(48, 48);
                    self.load_cover(&t.cover(), &p);
                    pictures.push(p);
                }
                let content = hbox(8);
                content.append(&artwork::cover(
                    &pictures,
                    if path == "mix" { 4 } else { 9 },
                    52,
                ));
                let name = label(title, "track-title");
                name.set_hexpand(true);
                name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                name.set_max_width_chars(20);
                content.append(&name);
                content.append(&symbol("media-playback-start-symbolic"));
                b.set_child(Some(&content));
            }
            if path != "mix" && path != "wave" {
                let content = hbox(8);
                let icon = symbol(if path == "likes" {
                    "emblem-favorite-symbolic"
                } else {
                    "view-list-symbolic"
                });
                icon.set_size_request(52, 52);
                content.append(&icon);
                let name = label(title, "track-title");
                name.set_hexpand(true);
                name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                name.set_max_width_chars(20);
                content.append(&name);
                content.append(&symbol("media-playback-start-symbolic"));
                b.set_child(Some(&content));
            }
            let u = self.clone();
            b.connect_clicked(move |_| match path {
                "wave" => u.start_wave(),
                "mix" => u.request("mix_play".into(), "/api/mixes/today".into(), vec![], None),
                "likes" => u.request("likes_play".into(), "/api/nd/liked".into(), vec![], None),
                _ => u.navigate(path, false),
            });
            quick.insert(&b, -1);
        }
        self.shelves.append(&quick);
        if settings::pref_bool("home_show_mixes", true)
            && let Some(groups) = self.personal_mixes.borrow()["groups"].as_array()
        {
            if !groups.is_empty() {
                self.shelves
                    .append(&label("Мои миксы дня", "section-title"));
            }
            let grid = gtk::FlowBox::new();
            grid.set_selection_mode(gtk::SelectionMode::None);
            grid.set_homogeneous(true);
            grid.set_widget_name("home-mix-grid");
            grid.set_max_children_per_line(
                read_pref("home_grid_columns").as_u64().unwrap_or(6).clamp(2, 6) as u32,
            );
            grid.set_column_spacing(12);
            grid.set_row_spacing(12);
            for group in groups {
                let tile = button("", None, "recommendation");
                let column = vbox(8);
                let mut pictures = vec![];
                for t in api::tracks(&group["tracks"]).iter().take(4) {
                    let p = gtk::Picture::new();
                    p.set_size_request(128, 128);
                    self.load_cover(&t.cover(), &p);
                    pictures.push(p);
                }
                column.append(&artwork::cover(&pictures, 4, 128));
                let name = label(group["title"].as_str().unwrap_or("Микс"), "mix-label");
                name.set_max_width_chars(20);
                name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                column.append(&name);
                tile.set_child(Some(&column));
                let path = format!("personal:{}", group["id"].as_str().unwrap_or(""));
                let u = self.clone();
                tile.connect_clicked(move |_| u.navigate(&path, false));
                grid.insert(&tile, -1);
            }
            self.shelves.append(&grid);
        }
        if settings::pref_bool("home_show_waves", true)
            && let Some(tracks) = value["daily"].as_array() {
            self.shelves
                .append(&label("Волны по вашим трекам", "section-title"));
            let grid = gtk::FlowBox::new();
            grid.set_selection_mode(gtk::SelectionMode::None);
            grid.set_widget_name("home-wave-grid");
            grid.set_max_children_per_line(4);
            grid.set_min_children_per_line(2);
            grid.set_column_spacing(8);
            grid.set_row_spacing(8);
            grid.set_homogeneous(true);
            let tracks = api::tracks(&json!(tracks));
            for track in tracks.iter().take(8) {
                let b = gtk::Button::new();
                b.add_css_class("recommendation");
                let content = hbox(8);
                let picture = gtk::Picture::new();
                picture.set_can_shrink(true);
                picture.set_size_request(48, 48);
                picture.set_content_fit(gtk::ContentFit::Cover);
                self.load_cover(&track.cover(), &picture);
                content.append(&artwork::cover(&[picture.clone()], 9, 64));
                let texts = vbox(2);
                let title = label(&track.title, "track-title");
                let link_track = track.clone();
                self.link_track_title(&title, move || Some(link_track.clone()));
                title.set_ellipsize(gtk::pango::EllipsizeMode::End);
                title.set_max_width_chars(18);
                texts.append(&title);
                let artist = label(&track.artist, "small muted");
                artist.set_ellipsize(gtk::pango::EllipsizeMode::End);
                artist.set_max_width_chars(32);
                let metadata = track.clone();
                self.link_track_field(&artist, move || Some(metadata.clone()), true);
                texts.append(&artist);
                content.append(&texts);
                b.set_child(Some(&content));
                let u = self.clone();
                let seed = track.clone();
                b.connect_clicked(move |_| {
                    u.preview_wave_from(&seed);
                });
                grid.insert(&b, -1);
            }
            self.shelves.append(&grid);
        }
        for (field, heading) in [
            ("recent", "Недавно слушали"),
            ("frequent", "Часто слушаете"),
            ("starred", "В медиатеке"),
        ] {
            let values = value[field].as_array();
            if values.is_none_or(|v| v.is_empty()) {
                continue;
            }
            self.shelves.append(&label(heading, "section-title"));
            let row = hbox(12);
            for album in values.unwrap().iter().take(8) {
                let path = format!("/api/nd/albums/{}", text(album, "id"));
                row.append(&self.media_card(
                    &text(album, "title"),
                    &text(album, "artist"),
                    &path,
                    &text(album, "cover_url"),
                ));
            }
            let scroll = gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Automatic)
                .vscrollbar_policy(gtk::PolicyType::Never)
                .child(&row)
                .build();
            self.shelves.append(&scroll);
        }
    }
    pub(super) fn shuffle_queue(self: &Rc<Self>) {
        let start = (self.index.get() + 1).max(self.user_queue_end.get() + 1);
        let mut queue = self.queue.borrow_mut();
        let len = queue.len();
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos() as usize;
        for i in (start..len).rev() {
            let j = start + seed.wrapping_mul(i + 13) % (i - start + 1);
            queue.swap(i, j);
        }
        drop(queue);
        if self.page.borrow().as_str() == "queue" {
            self.show_queue();
        }
    }
    pub(super) fn create_playlist(self: &Rc<Self>) {
        let dialog = gtk::Window::builder()
            .transient_for(&self.window)
            .modal(true)
            .title("Новый плейлист")
            .default_width(360)
            .build();
        let content = vbox(16);
        content.set_margin_top(20);
        content.set_margin_bottom(20);
        content.set_margin_start(20);
        content.set_margin_end(20);
        let entry = gtk::Entry::builder()
            .placeholder_text("Название плейлиста")
            .build();
        content.append(&entry);
        let create = button("Создать", None, "primary");
        content.append(&create);
        mount_dialog(&dialog, &content);
        let u = self.clone();
        let d = dialog.clone();
        create.connect_clicked(move |_| {
            if !entry.text().trim().is_empty() {
                let api = u.api.clone();
                let tx = u.tx.clone();
                let name = entry.text().to_string();
                u.net.run(move || {
                    let result = api.post("/api/playlists", json!({"name":name}));
                    let _ = tx.send_blocking(Event::Notice(result, "Плейлист создан".into()));
                    let _ = tx.send_blocking(Event::Sidebar(api.get("/api/playlists")));
                });
                d.close();
            }
        });
        dialog.present();
    }
    pub(super) fn context_menu(
        self: &Rc<Self>,
        item: &gtk::ListItem,
        anchor: &gtk::Widget,
        x: f64,
        y: f64,
    ) {
        let Some(obj) = item.item().and_downcast::<glib::BoxedAnyObject>() else {
            return;
        };
        let data = obj.borrow::<Item>().clone();
        self.context_item_origin(data, item.position() as usize, anchor, x, y, true);
    }
    pub(super) fn context_item(
        self: &Rc<Self>,
        data: Item,
        position: usize,
        anchor: &gtk::Widget,
        x: f64,
        y: f64,
    ) {
        self.context_item_origin(data, position, anchor, x, y, false);
    }
    fn context_item_origin(
        self: &Rc<Self>,
        data: Item,
        position: usize,
        anchor: &gtk::Widget,
        x: f64,
        y: f64,
        allow_playlist: bool,
    ) {
        let pop = gtk::Popover::new();
        pop.add_css_class("material-menu");
        pop.set_has_arrow(false);
        anchor.add_css_class("context-open");
        let weak_anchor = anchor.downgrade();
        pop.connect_closed(move |_| {
            if let Some(anchor) = weak_anchor.upgrade() {
                anchor.remove_css_class("context-open");
            }
        });
        pop.set_parent(anchor);
        pop.set_position(gtk::PositionType::Right);
        pop.set_halign(gtk::Align::Start);
        pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32 + 6, y as i32 + 8, 1, 1)));
        let actions = vbox(0);
        actions.set_width_request(260);
        let menu_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .propagate_natural_height(true)
            .max_content_height((self.window.height() - 64).clamp(200, 600))
            .child(&actions)
            .build();
        pop.set_child(Some(&ripple::wrap(&menu_scroll)));
        if let Item::Track(t) = data {
            let header = label(&format!("{} · {}", t.title, t.artist), "menu-heading");
            header.set_ellipsize(gtk::pango::EllipsizeMode::End);
            header.set_max_width_chars(30);
            actions.append(&header);
            for (title, action) in [
                ("Волна по треку", 12),
                ("Следующей", 0),
                ("В конец очереди", 1),
                ("Добавить в плейлист", 2),
                ("Открыть альбом", 3),
                ("Открыть исполнителя", 4),
                (
                    if t.liked {
                        "Убрать из любимого"
                    } else {
                        "Добавить в любимое"
                    },
                    5,
                ),
                ("Скачать", 6),
                ("Текст песни", 7),
                ("Убрать из плейлиста", 11),
                ("Переместить в плейлист", 14),
            ] {
                if (8..=10).contains(&action) && self.page.borrow().as_str() != "queue" {
                    continue;
                }
                if (action == 11 || action == 14) && !allow_playlist {
                    continue;
                }
                if (action == 11 || action == 14)
                    && (!self.page.borrow().starts_with("/api/playlists/")
                        || self.sort.selected() != 0 || !self.list_filter.text().trim().is_empty())
                {
                    continue;
                }
                if action == 3 {
                    actions.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
                }
                let b = gtk::Button::new();
                b.add_css_class("menu-item");
                let row = hbox(12);
                let icon = match action {
                    0 | 1 => "view-list-symbolic",
                    2 => "list-add-symbolic",
                    3 => "media-optical-symbolic",
                    4 => "avatar-default-symbolic",
                    5 => "emblem-favorite-symbolic",
                    6 => "folder-download-symbolic",
                    7 => "text-x-generic-symbolic",
                    12 => "media-playlist-shuffle-symbolic",
                    13 => "audio-volume-high-symbolic",
                    _ => "view-list-symbolic",
                };
                row.append(&symbol(icon));
                row.append(&label(title, ""));
                b.set_child(Some(&row));
                let u = self.clone();
                let track = t.clone();
                let menu = pop.clone();

                b.connect_clicked(move |_| {
                    menu.popdown();
                    match action {
                        0 | 1 => {
                            u.enqueue(track.clone(), action == 0);
                        }
                        2 => u.add_to_playlist(&track),
                        3 => u.open_album(&track),
                        4 => u.open_artist(&track),
                        5 => u.favorite(&track, !track.liked),
                        6 => u.download(&track),
                        7 => u.open_details(track.clone()),
                        8 => {
                            if position < u.queue.borrow().len() {
                                u.queue.borrow_mut().remove(position);
                                if position < u.index.get() {
                                    u.index.set(u.index.get() - 1);
                                }
                                u.show_queue();
                            }
                        }
                        12 => u.wave_from(Some(&track), false),
                        13 => u.source_dialog(&track),
                        14 => u.move_to_playlist(&track, position),
                        9 | 10 => {
                            let target = if action == 9 {
                                position.saturating_sub(1)
                            } else {
                                position + 1
                            };
                            if target < u.queue.borrow().len() {
                                u.queue.borrow_mut().swap(position, target);
                                if u.index.get() == position {
                                    u.index.set(target)
                                } else if u.index.get() == target {
                                    u.index.set(position)
                                }
                                u.show_queue();
                            }
                        }
                        11 => {
                            let path = format!("{}/remove", u.page.borrow());
                            let api = u.api.clone();
                            let tx = u.tx.clone();
                            u.net.run(move || {
                                let _ = tx.send_blocking(Event::Notice(
                                    api.post(&path, json!({"indexes":[position]})),
                                    "Удалено из плейлиста · обновите список".into(),
                                ));
                            });
                        }
                        _ => (),
                    }
                });
                if action == 1 {
                    let cursor_gap = vbox(0);
                    cursor_gap.set_height_request(16);
                    cursor_gap.set_can_target(false);
                    actions.append(&cursor_gap);
                }
                actions.append(&b);
            }
        } else if let Item::Card {
            path, title, sub, cover
        } = data
        {
            if !cover.is_empty() {
                let b = button("Рассмотреть обложку", Some("zoom-in-symbolic"), "menu-item");
                let u = Rc::downgrade(self);
                let name = title.clone();
                let menu = pop.clone();
                b.connect_clicked(move |_| {
                    menu.popdown();
                    if let Some(u) = u.upgrade() {
                        cover_viewer::show(&u, &name, &cover, None);
                    }
                });
                actions.append(&b);
            }
            if path.contains("/album") {
                let b = button("Об альбоме", Some("text-x-generic-symbolic"), "menu-item");
                let u = self.clone();
                b.connect_clicked(move |_| {
                    details::show_album(&u.window, u.api.clone(), title.clone(), sub.clone())
                });
                actions.append(&b);
            }
            let b = button("Открыть", None, "flat");
            let u = self.clone();
            let menu = pop.clone();
            b.connect_clicked(move |_| {
                menu.popdown();
                if let Some(id) = path.strip_prefix("download:").and_then(|id| id.parse().ok()) {
                    u.open_download(id);
                } else {
                    u.navigate(&path, false);
                }
            });
            actions.append(&b);
        }
        pop.connect_closed(|p| p.unparent());
        pop.popup();
        let (_, height, _, _) = pop.measure(gtk::Orientation::Vertical, -1);
        let header_height = actions
            .first_child()
            .map(|w| w.measure(gtk::Orientation::Vertical, 260).1)
            .unwrap_or(32);
        let action_height = actions
            .first_child()
            .and_then(|w| w.next_sibling())
            .map(|w| w.measure(gtk::Orientation::Vertical, 260).1)
            .unwrap_or(36);
        // Right-side popovers center vertically on their anchor. Place the
        // pointer in the gap following "Play next", before "Add to queue".
        // Eight logical pixels each for the contents inset and popup surface
        // margin. Child natural sizes exclude both; include them in the gap.
        // Center the pointer within the explicit 16px non-action gap.
        pop.set_offset(8, height / 2 - header_height - 2 * action_height - 32);
    }
    pub(super) fn add_to_playlist(self: &Rc<Self>, track: &Track) {
        self.playlist_picker(track, None);
    }
}
