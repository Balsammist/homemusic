use super::*;
impl Ui {
    pub(super) fn open_details(self: &Rc<Self>, track: Track) {
        self.open_details_tab(track, false);
    }
    pub(super) fn open_track_info(self: &Rc<Self>, track: Track) {
        self.open_details_tab(track, true);
    }
    fn open_details_tab(self: &Rc<Self>, track: Track, info_first: bool) {
        let weak = Rc::downgrade(self);
        let seek_weak = weak.clone();
        let lyrics_track = track.reference.clone();
        let seek_track = lyrics_track.clone();
        let source_weak = weak.clone();
        let source_track = track.clone();
        let source = Some((
            self.source_label.text().to_string(),
            Rc::new(move || {
                if let Some(u) = source_weak.upgrade() {
                    u.source_dialog(&source_track);
                }
            }) as Rc<dyn Fn()>,
        ));
        let artist_track = track.clone();
        let album_track = track.clone();
        let artist_ui = Rc::downgrade(self);
        let album_ui = Rc::downgrade(self);
        let lyrics_prefetch = self.now_lyrics.prefetch(&track);
        details::show(
            &self.window,
            self.api.clone(),
            track,
            move || {
                weak.upgrade()
                    .filter(|u| u.current.borrow().as_ref().is_some_and(|t| t.reference == lyrics_track))
                    .and_then(|u| u.player.position())
                    .unwrap_or(f64::NAN)
            },
            move |seconds| {
                if let Some(u) = seek_weak.upgrade()
                    && u.current.borrow().as_ref().is_some_and(|t| t.reference == seek_track)
                {
                    u.seek_to(seconds);
                }
            },
            source,
            info_first,
            Some((
                Rc::new(move || {
                    if let Some(u) = artist_ui.upgrade() {
                        u.open_artist(&artist_track);
                    }
                }),
                Rc::new(move || {
                    if let Some(u) = album_ui.upgrade() {
                        u.open_album(&album_track);
                    }
                }),
            )),
            lyrics_prefetch,
        );
    }
    pub(super) fn wave_from(self: &Rc<Self>, track: Option<&Track>, continuation: bool) {
        if continuation {
            self.wave_waiting.set(true);
            self.continue_from_track(track);
            return;
        }
        self.invalidate_continuation();
        *self.wave.borrow_mut() = None;
        self.show_wave_page("wave");
        self.subtitle.set_text("Подбираем музыку");
        let mut body = json!({"client":"native/0.2","origin":"track_menu"});
        if let Some(t) = track {
            body["seed"] = json!({"ref":t.reference});
            self.title.set_text(&format!("Волна · {}", t.title));
        } else {
            self.title.set_text("Моя волна");
        }
        self.request(
            "wave".into(),
            "/api/wave/start".into(),
            vec![],
            Some(body),
        );
    }
    pub(super) fn source_dialog(self: &Rc<Self>, track: &Track) {
        let window = gtk::Window::builder()
            .transient_for(&self.window)
            .title("Источники аудио")
            .default_width(640)
            .default_height(520)
            .build();
        let root = vbox(12);
        root.set_margin_top(24);
        root.set_margin_bottom(24);
        root.set_margin_start(24);
        root.set_margin_end(24);
        mount_dialog(&window, &root);
        let title=button(&track.title,None,"media-link section-title");title.set_halign(gtk::Align::Start);
        let u=self.clone();let t=track.clone();let dialog=window.clone();title.connect_clicked(move |_|{dialog.close();u.open_album(&t);});root.append(&title);
        let metadata=hbox(6);
        for (value,artist) in [(track.artist.as_str(),true),(track.album_title.as_deref().unwrap_or(""),false)] {
            if value.is_empty(){continue;}
            let link=button(value,None,"media-link muted");
            let u=self.clone();let t=track.clone();let dialog=window.clone();link.connect_clicked(move |_|{dialog.close();if artist{u.open_artist(&t)}else{u.open_album(&t)}});metadata.append(&link);
        }
        root.append(&metadata);
        let list = vbox(4);
        let state = label("Ищем доступные записи…", "muted");
        list.append(&state);
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&list)
            .build();
        root.append(&scroll);
        let (tx, rx) = async_channel::bounded(1);
        let api = self.api.clone();
        let query = format!("{} - {}", track.artist, track.title);
        self.net.run(move || {
            let _ = tx.send_blocking(api.query(
                "/api/search/providers",
                &[("q", &query), ("group", "sources")],
            ));
        });
        let u = self.clone();
        let original = track.clone();
        let weak = window.downgrade();
        glib::spawn_future_local(async move {
            let Ok(result) = rx.recv().await else {
                return;
            };
            if weak.upgrade().is_none() {
                return;
            }
            match result {
                Err(e) => state.set_text(&e),
                Ok(v) => {
                    list.remove(&state);
                    let candidates = v["candidates"].as_array().cloned().unwrap_or_default();
                    if candidates.is_empty() {
                        list.append(&label("Доступные записи не найдены", "muted"));
                    }
                    for c in candidates {
                        let b = gtk::Button::new();
                        b.add_css_class("source-row");
                        let texts = vbox(4);
                        texts.append(&label(
                            &format!("{} — {}", text(&c, "artist"), text(&c, "title")),
                            "track-title",
                        ));
                        texts.append(&label(
                            &format!(
                                "{} · {} кбит/с · {}",
                                text(&c, "provider"),
                                c["bitrate_kbps"]
                                    .as_u64()
                                    .map(|n| n.to_string())
                                    .unwrap_or_else(|| "—".into()),
                                time_text(c["duration_s"].as_f64().unwrap_or(0.))
                            ),
                            "small muted",
                        ));
                        b.set_child(Some(&texts));
                        let pin = pins::Pin {
                            reference: format!(
                                "file:{}:{}",
                                text(&c, "provider"),
                                text(&c, "provider_id")
                            ),
                            provider: text(&c, "provider"),
                            duration_s: c["duration_s"].as_f64(),
                        };
                        let u = u.clone();
                        let weak = weak.clone();
                        let original = original.clone();
                        b.connect_clicked(move |_| {
                            u.pin_source(&original, pin.clone());
                            if let Some(w) = weak.upgrade() {
                                w.close();
                            }
                        });
                        list.append(&b);
                    }
                }
            }
        });
        window.present();
    }
}
impl Ui {
    pub(super) fn preview_wave_from(self: &Rc<Self>, track: &Track) {
        self.show_wave_page("wave_preview");
        self.title.set_text(&format!("Волна · {}", track.title));
        self.subtitle.set_text(&track.artist);
        self.request(
            format!("wave_preview:{}", track.reference),
            "/api/wave/start".into(),
            vec![],
            Some(
                json!({"client":"native/0.3","origin":"home_card","seed":{"ref":track.reference}}),
            ),
        );
    }
    pub(super) fn set_shuffle(self: &Rc<Self>, enabled: bool) {
        if self.shuffled.replace(enabled) == enabled {
            return;
        }
        if enabled {
            *self.queue_original.borrow_mut() = self.queue.borrow().clone();
            self.shuffle_queue();
        } else {
            // Restore only the remaining source, never relocate a duplicate current
            // position by canonical ref or shuffle the user's explicit additions.
            let start = (self.index.get() + 1).max(self.user_queue_end.get() + 1);
            let original = self.queue_original.borrow().clone();
            let mut queue = self.queue.borrow_mut();
            if start < queue.len() && start < original.len() {
                let mut remaining = queue[start..].to_vec();
                let mut unconsumed = original.clone();
                for consumed in &queue[..start] {
                    if let Some(i) = unconsumed.iter().position(|t| {
                        t.reference == consumed.reference && t.playback_ref == consumed.playback_ref
                    }) {
                        unconsumed.remove(i);
                    }
                }
                let mut ordered = vec![];
                for t in &unconsumed {
                    if let Some(i) = remaining.iter().position(|x| {
                        x.reference == t.reference && x.playback_ref == t.playback_ref
                    }) {
                        ordered.push(remaining.remove(i));
                    }
                }
                ordered.extend(remaining);
                queue.truncate(start);
                queue.extend(ordered);
            }
        }
        if self.queue_panel.is_visible() {
            self.fill_queue_panel();
        }
    }
}

impl Ui {
    pub(super) fn invalidate_continuation(&self) {
        self.continuation_generation
            .set(self.continuation_generation.get().wrapping_add(1));
        self.autoplay_pending.set(false);
        self.continuation_restarted.set(false);
        self.wave_waiting.set(false);
        self.refill_after.set(Instant::now());
    }
    fn continue_from_track(self: &Rc<Self>, track: Option<&Track>) {
        self.autoplay_pending.set(true);
        let generation = self.continuation_generation.get();
        let api = self.api.clone();
        let tx = self.tx.clone();
        let body = json!({"seed":track.map(|t|json!({"ref":t.reference})),"client":"native/0.3","origin":"queue_end"});
        if !self.net.run(move || {
            let _ = tx.send_blocking(Event::Continuation(
                generation,
                String::new(),
                api.post("/api/wave/start", body),
            ));
        }) {
            self.autoplay_pending.set(false);
            self.refill_after
                .set(Instant::now() + Duration::from_secs(2));
            self.notice("Подбор продолжения занят; повторим попытку", false);
        }
    }
    pub(super) fn continue_session(self: &Rc<Self>, id: &str) {
        if self.autoplay_pending.replace(true) {
            return;
        }
        let api = self.api.clone();
        let tx = self.tx.clone();
        let path = format!("/api/wave/{id}/next");
        let session = id.to_string();
        let generation = self.continuation_generation.get();
        let played: Vec<Value> = vec![]; // Playback events are reported through /api/plays, never forge plays for buffering.
        let queued: Vec<_> = self
            .queue
            .borrow()
            .iter()
            .rev()
            .take(50)
            .map(|t| t.reference.clone())
            .collect();
        if !self.net.run(move || {
            let _ = tx.send_blocking(Event::Continuation(
                generation,
                session,
                api.post(
                    &path,
                    json!({"played":played,"queued":queued,"append":true}),
                ),
            ));
        }) {
            self.autoplay_pending.set(false);
            self.refill_after
                .set(Instant::now() + Duration::from_secs(2));
            self.notice("Подбор продолжения занят; повторим попытку", false);
        }
    }
    pub(super) fn receive_continuation(self: &Rc<Self>, result: Result<Value, String>) {
        self.autoplay_pending.set(false);
        match result {
            Err(e) => {
                self.refill_after
                    .set(Instant::now() + Duration::from_secs(15));
                if e.contains("404") && !self.continuation_restarted.replace(true) {
                    *self.wave.borrow_mut() = None;
                    self.notice("Сессия волны истекла; подбираем новое продолжение", false);
                    let seed = self.current.borrow().clone();
                    self.continue_from_track(seed.as_ref());
                } else {
                    self.notice(&e, true);
                }
            }
            Ok(v) => {
                let current = self.current.borrow().as_ref().map(|t| t.reference.clone());
                let fresh: Vec<_> = api::tracks(&v["tracks"])
                    .into_iter()
                    .filter(|t| {
                        Some(&t.reference) != current.as_ref()
                            && !self
                                .queue
                                .borrow()
                                .iter()
                                .any(|q| q.reference == t.reference)
                    })
                    .collect();
                if let Some(sid) = v["session_id"].as_str() {
                    *self.wave.borrow_mut() = Some(sid.into());
                }
                if fresh.is_empty() {
                    self.refill_after
                        .set(Instant::now() + Duration::from_secs(15));
                    self.notice("Волна пока не нашла продолжение", false);
                    return;
                }
                let next = self.queue.borrow().len();
                self.queue_original.borrow_mut().extend(fresh.clone());
                self.queue.borrow_mut().extend(fresh);
                if self.wave_waiting.replace(false) {
                    self.play_index(next);
                }
                if self.queue_panel.is_visible() {
                    self.fill_queue_panel();
                }
            }
        }
    }
}
