use super::*;
impl Ui {
    pub(super) fn fill_artist(self: &Rc<Self>, v: &Value) {
        self.artist_covers.borrow_mut().clear();
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(160), move || {
            if let Some(u) = weak.upgrade() {
                u.flush_artist_covers();
            }
        });
        while let Some(child) = self.artist_body.first_child() {
            self.artist_body.remove(&child);
        }
        let artist = v["artist"].clone();
        let name = text(&artist, "name");
        let reference = text(&artist, "ref");
        let local_id = text(&artist, "id");
        let actions = gtk::FlowBox::new();
        actions.set_selection_mode(gtk::SelectionMode::None);
        actions.set_min_children_per_line(1);
        actions.set_max_children_per_line(4);
        actions.set_column_spacing(8);
        actions.set_row_spacing(8);
        let wave = button(
            "Волна по исполнителю",
            Some("media-playlist-shuffle-symbolic"),
            "primary",
        );
        let u = self.clone();
        let seed = if reference.is_empty() {
            json!({"nd_artist_id":local_id})
        } else {
            json!({"artist_ref":reference})
        };
        wave.connect_clicked(move |_| {
            u.request(
                "wave".into(),
                "/api/wave/start".into(),
                vec![],
                Some(json!({"seed":seed,"client":"native/0.4","origin":"artist_page"})),
            )
        });
        actions.insert(&wave, -1);
        let info = button("Об исполнителе", Some("text-x-generic-symbolic"), "tonal");
        let u = self.clone();
        let artist_name = name.clone();
        info.connect_clicked(move |_| {
            details::show_artist(&u.window, u.api.clone(), artist_name.clone())
        });
        actions.insert(&info, -1);
        if !reference.is_empty() {
            let download = button("Скачать релизы", Some("folder-download-symbolic"), "flat");
            let u = self.clone();
            let reference = reference.clone();
            download.connect_clicked(move |_| {
                let api = u.api.clone();
                let tx = u.tx.clone();
                let reference = reference.clone();
                u.net.run(move || {
                    let r = api.post(
                        "/api/downloads",
                        json!({"artist_ref":reference,"record_types":["album","single","ep"]}),
                    );
                    let _ =
                        tx.send_blocking(Event::Notice(r, "Релизы добавлены в загрузки".into()));
                });
            });
            actions.insert(&download, -1);
        }
        let like = icon_button("emblem-favorite-symbolic", "Нравится исполнитель", "flat");
        let initial = artist["starred"].as_bool().unwrap_or(false)
            || self.favorites_snapshot.borrow()["subjects"]
                .as_array()
                .is_some_and(|s| {
                    s.iter().any(|s| {
                        s["target_type"] == "artist"
                            && s["liked"] == true
                            && ((!reference.is_empty() && s["ref"] == reference)
                                || (!local_id.is_empty() && s["nd_id"] == local_id))
                    })
                });
        let liked = Rc::new(Cell::new(initial));
        set_like_icon(&like, initial);
        let u = self.clone();
        let target = if reference.is_empty() {
            json!({"target_type":"artist","nd_artist_id":local_id,"target_data":{"name":name}})
        } else {
            json!({"target_type":"artist","ref":reference,"target_data":{"name":name}})
        };
        like.connect_clicked(move |button|{let desired=!liked.get();let (tx,rx)=async_channel::bounded(1);let api=u.api.clone();let target=target.clone();u.net.run(move ||{let r=api.post("/api/star",json!({"targets":[target],"unstar":!desired,"event_id":unique(),"client":"native/0.4"}));let _=tx.send_blocking(r);});let b=button.clone();let state=liked.clone();let u=u.clone();glib::spawn_future_local(async move{match rx.recv().await{Ok(Ok(_))=>{state.set(desired);set_like_icon(&b,desired);let api=u.api.clone();let tx=u.tx.clone();u.net.run(move ||{let _=tx.send_blocking(Event::Stars(api.get("/api/starred")));});},Ok(Err(e))=>u.notice(&e,true),_=>()}});});
        actions.insert(&like, -1);
        self.artist_body.append(&actions);
        let top: Vec<_> = api::tracks(&v["top_tracks"])
            .into_iter()
            .map(|t| self.pinned(t))
            .collect();
        if !top.is_empty() {
            self.artist_body
                .append(&label("Популярные треки", "section-title"));
            let all = top.clone();
            for (index, t) in top.iter().take(5).enumerate() {
                let row = hbox(8);
                let b = gtk::Button::new();
                b.add_css_class("artist-track");
                let content = hbox(8);
                let picture = gtk::Picture::new();
                picture.set_size_request(48, 48);
                picture.set_content_fit(gtk::ContentFit::Cover);
                picture.set_can_shrink(true);
                content.append(&cover_frame(&picture, 48));
                self.load_cover(&t.cover(), &picture);
                let texts = vbox(2);
                texts.set_valign(gtk::Align::Center);
                let title = label(&t.title, "track-title");
                title.set_ellipsize(gtk::pango::EllipsizeMode::End);
                title.set_max_width_chars(45);
                let link_track = t.clone();
                self.link_track_title(&title, move || Some(link_track.clone()));
                texts.append(&title);
                let metadata = t.clone();
                texts.append(&self.metadata_line(move || Some(metadata.clone())));
                content.append(&texts);
                b.set_hexpand(true);
                b.set_child(Some(&content));
                let u = self.clone();
                let tracks = all.clone();
                b.connect_clicked(move |_| {
                    u.invalidate_continuation();
                    u.user_queue_end.set(index);
                    *u.wave.borrow_mut() = None;
                    *u.queue_original.borrow_mut() = tracks.clone();
                    *u.queue.borrow_mut() = tracks.clone();
                    u.play_index(index);
                });
                row.append(&b);
                row.append(&label(
                    &time_text(t.duration_s.unwrap_or(0.)),
                    "small muted",
                ));
                let gesture = gtk::GestureClick::new();
                gesture.set_button(3);
                let u = self.clone();
                let t = t.clone();
                gesture.connect_pressed(move |g, _, x, y| {
                    if let Some(w) = g.widget() {
                        u.context_item(Item::Track(t.clone()), index, &w, x, y);
                    }
                });
                row.add_controller(gesture);
                self.artist_body.append(&row);
            }
        }
        if top.len() > 5 {
            let more = button("Показать все", None, "tonal");
            more.set_widget_name("artist-all-tracks");
            let u = self.clone();
            let artist_name = name.clone();
            more.connect_clicked(move |_| {
                let route = format!("artist_tracks:{artist_name}");
                {
                    let mut cache = u.cache.borrow_mut();
                    cache.retain(|(key, _)| key != &route);
                    cache.push_back((route.clone(), json!({"tracks":top})));
                }
                u.navigate(&route, false);
            });
            self.artist_body.append(&more);
        }
        for (field, title) in [
            ("albums", "Альбомы"),
            ("singles", "Синглы и EP"),
            ("compilations", "Сборники"),
        ] {
            let releases = v[field].as_array().cloned().unwrap_or_default();
            if releases.is_empty() {
                continue;
            }
            self.artist_body.append(&label(
                &format!("{title} · {}", releases.len()),
                "section-title",
            ));
            let flow = gtk::FlowBox::new();
            flow.set_selection_mode(gtk::SelectionMode::None);
            flow.set_min_children_per_line(2);
            flow.set_max_children_per_line(5);
            flow.set_column_spacing(12);
            flow.set_row_spacing(12);
            self.append_releases(&flow, &releases, 0, 24);
            self.artist_body.append(&flow);
            if releases.len() > 24 {
                let more = button("Показать ещё", None, "tonal");
                let u = self.clone();
                let flow = flow.clone();
                let offset = Rc::new(Cell::new(24));
                more.connect_clicked(move |b| {
                    let start = offset.get();
                    u.append_releases(&flow, &releases, start, 24);
                    offset.set(start + 24);
                    b.set_visible(offset.get() < releases.len());
                });
                self.artist_body.append(&more);
            }
        }
    }
    fn append_releases(
        self: &Rc<Self>,
        flow: &gtk::FlowBox,
        releases: &[Value],
        offset: usize,
        count: usize,
    ) {
        for album in releases.iter().skip(offset).take(count) {
            let path = if let Some(reference) = album["ref"].as_str() {
                format!("/api/catalog/album/{reference}")
            } else {
                format!("/api/nd/albums/{}", text(album, "id"))
            };
            let subtitle = [year_text(album), text(album, "artist")]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            let cover = album["cover_url"].as_str().map(str::to_owned)
                .or_else(|| album["cover_art"].as_str().map(|id| format!("/api/nd/cover/{id}?size=96")))
                .unwrap_or_default();
            flow.insert(
                &self.media_card(
                    &text(album, "title"),
                    &subtitle,
                    &path,
                    &cover,
                ),
                -1,
            );
        }
    }
    pub(super) fn open_album(self: &Rc<Self>, t: &Track) {
        if let Some(id) = t.nd_album_id.as_deref().filter(|s| !s.is_empty()) {
            self.navigate(&format!("/api/nd/albums/{id}"), false)
        } else if let Some(reference) = t.album_ref.as_deref().filter(|s| !s.is_empty()) {
            let path = if let Some(id) = reference.strip_prefix("nd:") {
                format!("/api/nd/albums/{id}")
            } else {
                format!("/api/catalog/album/{reference}")
            };
            self.navigate(&path, false)
        } else if !t.reference.is_empty() {
            self.notice("Ищем альбом этой записи…", false);
            let api = self.api.clone();
            let tx = self.tx.clone();
            let reference = t.reference.clone();
            self.request.set(self.request.get() + 1);
            let generation = self.request.get();
            self.net.run(move || {
                let _ = tx.send_blocking(Event::AlbumMetadata(
                    generation,
                    api.get(&format!("/api/catalog/track/{reference}")),
                ));
            });
        } else {
            self.notice("Альбом этого трека пока неизвестен", false);
        }
    }
    pub(super) fn open_artist(self: &Rc<Self>, t: &Track) {
        if let Some(reference) = t.artist_ref.as_deref().filter(|s| !s.is_empty()) {
            let route = if let Some(id) = reference.strip_prefix("nd:") {
                format!("/api/nd/artists/{id}")
            } else {
                format!("/api/catalog/artist/{reference}")
            };
            self.navigate(&route, false);
        } else if let Some(id) = t.nd_artist_id.as_deref().filter(|s| !s.is_empty()) {
            self.navigate(&format!("/api/nd/artists/{id}"), false);
        } else {
            self.request.set(self.request.get() + 1);
            let generation = self.request.get();
            let api = self.api.clone();
            let tx = self.tx.clone();
            let track = t.clone();
            self.notice("Получаем карточку исполнителя…", false);
            self.net.run(move || {
                let found = if !track.reference.is_empty() {
                    api.get(&format!("/api/catalog/track/{}", track.reference))
                        .ok()
                } else {
                    None
                };
                let mut reference = found
                    .as_ref()
                    .and_then(|v| v["artist_ref"].as_str())
                    .filter(|s| !s.is_empty())
                    .map(String::from);
                if reference.is_none()
                    && (track.nd_song_id.is_some() || track.reference.starts_with("nd:"))
                {
                    if let Ok(artists) = api.get("/api/nd/artists") {
                        let matches: Vec<_> = artists
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|a| {
                                text(a, "name").trim().to_lowercase()
                                    == track.artist.trim().to_lowercase()
                            })
                            .collect();
                        if matches.len() == 1 {
                            reference = matches[0]["id"].as_str().map(|id| format!("nd:{id}"));
                        }
                    }
                }
                let result = reference
                    .map(|r| {
                        if let Some(id) = r.strip_prefix("nd:") {
                            format!("/api/nd/artists/{id}")
                        } else {
                            format!("/api/catalog/artist/{r}")
                        }
                    })
                    .ok_or_else(|| {
                        "Точная карточка исполнителя пока недоступна. Повторите нажатие позже."
                            .into()
                    });
                let _ = tx.send_blocking(Event::ArtistMetadata(generation, result));
            });
        }
    }
}
pub(super) fn complete_artist(api: &Api, local: Value) -> Value {
    let name = text(&local["artist"], "name");
    if let Ok(search) = api.query_with_timeout("/api/search", &[("q", name.as_str())], Duration::from_secs(5)) {
        if let Some(artists) = search["artists"].as_array() {
            if let Some(a) = artists
                .iter()
                .find(|a| text(a, "name").to_lowercase() == name.to_lowercase())
            {
                if let Ok(full) = api.get_with_timeout(&format!("/api/catalog/artist/{}", text(a, "ref")), Duration::from_secs(4)) {
                    return full;
                }
            }
        }
    }
    local
}

impl Ui {
    pub(super) fn flush_artist_covers(self: &Rc<Self>) {
        let height = self.artist_view.height() as f32;
        let mut pending = self.artist_covers.borrow_mut();
        let mut ready = vec![];
        pending.retain(|(weak, url)| {
            let Some(p) = weak.upgrade() else {
                return false;
            };
            let visible = p
                .compute_bounds(&self.artist_view)
                .is_some_and(|b| b.y() < height + 200. && b.y() + b.height() > -200.);
            if visible {
                ready.push((p, url.clone()));
                false
            } else {
                true
            }
        });
        drop(pending);
        for (picture, url) in ready {
            self.load_cover(&url, &picture);
        }
    }
}

/// Preserve version suffixes and known years; a shared song is not album identity.
pub(super) fn same_release(local: &Value, candidate: &Value) -> bool {
    let normalize = |value: &Value, key: &str| {
        text(value, key)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let title = normalize(local, "title");
    let artist = normalize(local, "artist");
    let year = |value: &Value| -> Option<String> {
        if let Some(y) = value["year"].as_u64() {
            return Some(y.to_string());
        }
        value["release_date"]
            .as_str()
            .or(value["year"].as_str())
            .and_then(|s| s.get(..4))
            .filter(|s| s.chars().all(|c| c.is_ascii_digit()))
            .map(String::from)
    };
    !title.is_empty()
        && !artist.is_empty()
        && title == normalize(candidate, "title")
        && artist == normalize(candidate, "artist")
        && match (year(local), year(candidate)) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn album_match_rejects_empty_identity_other_artist_and_versions() {
        let local = json!({"title":"Реанимация", "artist":"Гражданская Оборона", "year":2017});
        assert!(same_release(
            &local,
            &json!({"title":"реанимация", "artist":"гражданская  оборона", "release_date":"2017-01-01"})
        ));
        assert!(!same_release(
            &local,
            &json!({"title":"Реанимация", "artist":"Другой исполнитель"})
        ));
        assert!(!same_release(
            &local,
            &json!({"title":"Реанимация (Live)", "artist":"Гражданская Оборона"})
        ));
        assert!(!same_release(
            &local,
            &json!({"title":"Реанимация", "artist":"Гражданская Оборона", "year":2004})
        ));
        assert!(!same_release(&json!({}), &json!({})));
    }
}
