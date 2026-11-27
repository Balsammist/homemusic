//! Shared media cards and sectioned catalog search.
use super::*;
impl Ui {
    pub(super) fn link_track_title(
        self: &Rc<Self>,
        label: &gtk::Label,
        track: impl Fn() -> Option<Track> + 'static,
    ) {
        self.link_track_field(label, track, false);
    }
    pub(super) fn link_track_field(
        self: &Rc<Self>,
        label: &gtk::Label,
        track: impl Fn() -> Option<Track> + 'static,
        artist: bool,
    ) {
        label.add_css_class("media-link");
        label.set_cursor_from_name(Some("pointer"));
        label.set_focusable(true);
        label.set_can_target(true);
        let action: Rc<dyn Fn()> = {
            let u = self.clone();
            Rc::new(move || {
                if let Some(t) = track() {
                    if artist {
                        u.open_artist(&t)
                    } else {
                        u.open_album(&t)
                    }
                }
            })
        };
        let click = gtk::GestureClick::new();
        click.set_button(1);
        click.connect_pressed(|g,_,_,_|{g.set_state(gtk::EventSequenceState::Claimed);});
        let run = action.clone();
        click.connect_released(move |g, n, _, _| {
            if n == 1 {
                g.set_state(gtk::EventSequenceState::Claimed);
                run();
            }
        });
        label.add_controller(click);
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(move |_, key, _, _| {
            if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space) {
                action();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        label.add_controller(keys);
    }
    pub(super) fn metadata_line(
        self: &Rc<Self>,
        track: impl Fn() -> Option<Track> + 'static,
    ) -> gtk::Box {
        let track: Rc<dyn Fn() -> Option<Track>> = Rc::new(track);
        let row = hbox(4);
        row.set_hexpand(true);
        row.set_widget_name("track-metadata");
        for (i, text) in ["", "·", ""].iter().enumerate() {
            let label = label(text, "small muted");
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.set_max_width_chars(if i == 0 { 24 } else { 40 });
            if i != 1 {
                let getter = track.clone();
                self.link_track_field(&label, move || getter(), i == 0);
            }
            row.append(&label);
        }
        if let Some(t) = track() {
            set_metadata_line(&row, &t.artist, t.album_title.as_deref().unwrap_or(""));
        }
        row
    }

    pub(super) fn media_card(
        self: &Rc<Self>,
        title: &str,
        subtitle: &str,
        path: &str,
        cover: &str,
    ) -> gtk::Widget {
        let artist = path.contains("/artist/") || path.contains("/artists/");
        let card = button("", None, "album-tile");
        card.set_width_request(160);
        let column = vbox(8);
        let picture = gtk::Picture::new();
        picture.set_size_request(144, 144);
        picture.set_content_fit(gtk::ContentFit::Cover);
        picture.set_can_shrink(true);
        let frame = cover_frame(&picture, 144);
        if artist {
            frame.add_css_class("artist-portrait");
            frame.set_overflow(gtk::Overflow::Hidden);
        }
        // The fixed overlay measures only its144px fallback, not the remote
        // texture or desktop GtkFrame padding. It remains square in every theme.
        let square=frame;
        square.set_widget_name("media-card-cover");
        square.set_size_request(144,144);square.set_hexpand(false);square.set_vexpand(false);
        square.set_halign(gtk::Align::Start);square.set_valign(gtk::Align::Start);
        column.append(&square);
        self.defer_cover(cover, &picture);
        let name = label(title, "track-title");
        name.set_max_width_chars(16);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        name.set_lines(2);
        name.set_wrap(true);
        column.append(&name);
        let description = label(subtitle, "small muted");
        description.set_max_width_chars(16);
        description.set_ellipsize(gtk::pango::EllipsizeMode::End);
        if !subtitle.is_empty() {
            column.append(&description);
        }
        let footer = hbox(6);
        footer.append(&label(
            if artist {
                "Исполнитель"
            } else {
                "Альбом / подборка"
            },
            "small muted",
        ));

        if subtitle.is_empty() {
            column.append(&footer);
        }
        card.set_child(Some(&column));
        card.set_tooltip_text(Some(title));
        let u = self.clone();
        let route = path.to_string();
        card.connect_clicked(move |_| u.navigate(&route, false));
        let gesture = gtk::GestureClick::new();
        gesture.set_button(3);
        let u = self.clone();
        let data = Item::Card {
            title: title.into(),
            sub: subtitle.into(),
            path: path.into(),
            cover: cover.into(),
        };
        gesture.connect_pressed(move |g, _, x, y| {
            if let Some(anchor) = g.widget() {
                u.context_item(data.clone(), 0, &anchor, x, y);
            }
        });
        card.add_controller(gesture);
        let root = gtk::Overlay::new();
        root.add_css_class("media-card");
        root.set_halign(gtk::Align::Start);
        root.set_valign(gtk::Align::Start);
        root.set_child(Some(&card));
        if !artist {
            let play = icon_button(
                "media-playback-start-symbolic",
                "Слушать подборку",
                "primary card-play",
            );
            play.set_halign(gtk::Align::Start);
            play.set_width_request(44);
            play.set_height_request(44);
            play.set_margin_start(98);
            play.set_valign(gtk::Align::Start);
            play.set_margin_top(98);
            play.set_margin_end(0);
            let u = self.clone();
            let route = if path == "likes" {
                "/api/nd/liked".into()
            } else {
                path.to_string()
            };
            play.connect_clicked(move |_| {
                let api = u.api.clone();
                let tx = u.tx.clone();
                let path = route.clone();
                let generation = u.resolve.get() + 1;
                u.resolve.set(generation);
                u.resolving_pause.set(None);
                u.restore_position.set(None);
                u.update_play_control();
                u.notice("Загружаем треки подборки…", false);
                if !u.net.run(move || {
                    let _ = tx.send_blocking(Event::Collection(generation, api.get(&path)));
                }) {
                    u.notice("Не удалось открыть подборку. Повторите попытку", true);
                }
            });
            let hover = gtk::EventControllerMotion::new();
            let b = play.clone();
            hover.connect_enter(move |_, _, _| b.set_can_target(true));
            let b = play.clone();
            hover.connect_leave(move |_| b.set_can_target(false));
            root.add_controller(hover);
            play.set_can_target(false);
            root.add_overlay(&play);
        }
        root.upcast()
    }
    fn card_flow(self: &Rc<Self>, entries: &[Item]) -> gtk::FlowBox {
        let flow = gtk::FlowBox::new();
        flow.set_selection_mode(gtk::SelectionMode::None);
        flow.set_min_children_per_line(1);
        flow.set_max_children_per_line(6);
        flow.set_column_spacing(12);
        flow.set_row_spacing(12);
        flow.set_homogeneous(true);
        for entry in entries {
            if let Item::Card {
                title,
                sub,
                path,
                cover,
            } = entry
            {
                flow.insert(&self.media_card(title, sub, path, cover), -1);
            }
        }
        flow
    }
    pub(super) fn fill_card_page(self: &Rc<Self>) {
        if self.collection_view.selected() == 1 {
            self.artist_view.set_visible(false);
            self.content_list.set_visible(true);
            return;
        }
        while let Some(child) = self.artist_body.first_child() {
            self.artist_body.remove(&child);
        }
        if self.items.borrow().is_empty() {
            self.artist_body.append(&label("Ничего не найдено", "empty-title"));
        } else {
            self.artist_body.append(&self.card_flow(&self.items.borrow()));
        }
        self.artist_view.set_visible(true);
        self.content_list.set_visible(false);
    }
    pub(super) fn fill_search(self: &Rc<Self>) {
        while let Some(child) = self.artist_body.first_child() {
            self.artist_body.remove(&child);
        }
        let entries = self.items.borrow().clone();
        if entries.is_empty() {
            self.artist_body
                .append(&label("Ничего не найдено", "empty-title"));
        }
        let tracks: Vec<_> = entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                if let Item::Track(t) = e {
                    Some((i, t.clone()))
                } else {
                    None
                }
            })
            .collect();
        let overview = hbox(24);
        overview.set_homogeneous(true);
        let best = vbox(12);
        best.append(&label("Лучший результат", "section-title"));
        if let Some(Item::Card {
            title,
            sub,
            path,
            cover,
        }) = entries
            .iter()
            .find(|e| matches!(e,Item::Card{path,..} if path.contains("artist")))
        {
            let card = self.media_card(title, sub, path, cover);
            card.add_css_class("best-result");
            best.append(&card);
        } else if let Some((index, track)) = tracks.first() {
            best.append(&self.search_track(*index, track));
        }
        if !entries.is_empty() {
            overview.append(&best);
        }
        let top = vbox(4);
        top.append(&label("Треки", "section-title"));
        for (index, track) in tracks.iter().take(5) {
            top.append(&self.search_track(*index, track));
        }
        if !tracks.is_empty() {
            overview.append(&top);
        }
        self.artist_body.append(&overview);
        for (name, artist) in [("Исполнители", true), ("Альбомы и подборки", false)]
        {
            let cards:Vec<_>=entries.iter().filter(|e|matches!(e,Item::Card{path,..} if (path.contains("/artist/")||path.contains("/artists/"))==artist)).cloned().collect();
            if !cards.is_empty() {
                self.artist_body.append(&label(name, "section-title"));
                self.artist_body.append(&self.card_flow(&cards));
            }
        }
        if tracks.len() > 5 {
            self.artist_body
                .append(&label("Все треки", "section-title"));
            for (index, track) in tracks.iter().skip(5) {
                self.artist_body.append(&self.search_track(*index, track));
            }
        }
        self.artist_view.set_visible(true);
        self.content_list.set_visible(false);
    }
    fn search_track(self: &Rc<Self>, index: usize, track: &Track) -> gtk::Widget {
        let row = hbox(8);
        row.add_css_class("artist-track");
        let picture = gtk::Picture::new();
        picture.set_size_request(40, 40);
        picture.set_content_fit(gtk::ContentFit::Cover);
        row.append(&cover_frame(&picture, 40));
        self.defer_cover(&track.cover(), &picture);
        let texts = vbox(2);
        texts.set_hexpand(true);
        let title = button(&track.title, None, "media-link");
        title.set_halign(gtk::Align::Start);
        let u = self.clone();
        let t = track.clone();
        title.connect_clicked(move |_| u.open_album(&t));
        texts.append(&title);
        let metadata = track.clone();
        texts.append(&self.metadata_line(move || Some(metadata.clone())));
        row.append(&texts);
        row.append(&label(
            &track
                .duration_s
                .map(time_text)
                .unwrap_or_else(|| "—".into()),
            "small muted",
        ));
        let play = icon_button("media-playback-start-symbolic", "Слушать", "flat");
        let u = self.clone();
        play.connect_clicked(move |_| u.activate(index));
        row.append(&play);
        let more = icon_button("view-more-symbolic", "Действия с треком", "flat");
        let u = self.clone();
        let data = Item::Track(track.clone());
        more.connect_clicked(move |b| u.context_item(data.clone(), index, b.upcast_ref(), 0., 0.));
        row.append(&more);
        let click = gtk::GestureClick::new();
        click.set_button(3);
        let u = self.clone();
        let data = Item::Track(track.clone());
        click.connect_pressed(move |g, _, x, y| {
            if let Some(anchor) = g.widget() {
                u.context_item(data.clone(), index, &anchor, x, y);
            }
        });
        row.add_controller(click);
        row.upcast()
    }
}

pub(super) fn set_metadata_line(row: &gtk::Box, artist: &str, album: &str) {
    child(row.upcast_ref(), 0)
        .downcast::<gtk::Label>()
        .unwrap()
        .set_text(artist);
    child(row.upcast_ref(), 1).set_visible(!artist.is_empty() && !album.is_empty());
    child(row.upcast_ref(), 2)
        .downcast::<gtk::Label>()
        .unwrap()
        .set_text(album);
    child(row.upcast_ref(), 2).set_visible(!album.is_empty());
}
