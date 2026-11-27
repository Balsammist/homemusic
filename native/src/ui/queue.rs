use super::*;
impl Ui {
    pub(super) fn save_playback(&self) {
        let snapshot = json!({"version":1,"queue":*self.queue.borrow(),
            "original":*self.queue_original.borrow(),"index":self.index.get(),"user_queue_end":self.user_queue_end.get(),
            "wave":*self.wave.borrow(),"position":self.position_to_restore(),
            "repeat":self.repeat.get(),"shuffled":self.shuffled.get()});
        self.save_preferences(&[("playback", snapshot)], &[]);
    }
    pub(super) fn restore_playback(self: &Rc<Self>) {
        let saved = read_pref("playback");
        if saved["version"] != 1 {
            return;
        }
        let Ok(queue) = serde_json::from_value::<Vec<Track>>(saved["queue"].clone()) else {
            return;
        };
        let index = saved["index"].as_u64().unwrap_or(0) as usize;
        if index >= queue.len() {
            return;
        }
        self.invalidate_continuation();
        let queue_len = queue.len();
        *self.queue_original.borrow_mut() = queue.clone();
        *self.queue.borrow_mut() = queue;
        self.index.set(index);
        self.user_queue_end.set(
            (saved["user_queue_end"].as_u64().unwrap_or(index as u64) as usize).min(queue_len - 1),
        );
        *self.wave.borrow_mut() = saved["wave"].as_str().map(String::from);
        self.repeat.set(0);
        for _ in 0..saved["repeat"].as_u64().unwrap_or(0).min(2) {
            self.repeat_button.emit_clicked();
        }
        // The persisted queue is already shuffled. Update the button without
        // invoking a fresh shuffle or moving the restored current occurrence.
        let shuffled = saved["shuffled"].as_bool().unwrap_or(false);
        self.shuffled.set(shuffled);
        self.shuffle_button.set_active(shuffled);
        if let Ok(original) = serde_json::from_value::<Vec<Track>>(saved["original"].clone()) {
            *self.queue_original.borrow_mut() = original;
        }
        let position = settings::pref_bool("restore_track_position", true)
            .then(|| saved["position"].as_f64().unwrap_or(0.).max(0.));
        self.open_queue_entry(index, position, true);
    }

    pub(super) fn queue_is_visible(&self) -> bool {
        self.panel_stack.visible_child_name().as_deref() == Some("queue")
    }
    pub(super) fn select_panel(self: &Rc<Self>, name: &str) {
        let changed = self.panel_stack.visible_child_name().as_deref() != Some(name);
        self.panel_stack.set_visible_child_name(name);
        self.show_details.set(Some(true));
        self.update_panels();
        if !changed && name == "queue" {
            self.fill_queue_panel();
        } else if !changed && name == "history" {
            self.fill_history_panel();
        }
    }
    pub(super) fn enqueue(self: &Rc<Self>, track: Track, next: bool) -> bool {
        if settings::pref_bool("prevent_queue_duplicates", false)
            && self.queue.borrow().iter().skip(self.index.get()).any(|item| item.reference == track.reference)
        {
            self.notice("Этот трек уже есть в очереди", false);
            return false;
        }
        self.insert_in_queue(track, next);
        true
    }

    pub(super) fn insert_in_queue(self: &Rc<Self>, track: Track, next: bool) -> usize {
        let len = self.queue.borrow().len();
        let at = if self.current.borrow().is_none() {
            self.user_queue_end.set(len);
            if next { 0 } else { len }
        } else {
            let end = self.user_queue_end.get().max(self.index.get()).min(len - 1);
            self.user_queue_end.set(end + 1);
            if next { self.index.get() + 1 } else { end + 1 }
        };
        self.queue.borrow_mut().insert(at, track.clone());
        let mut original = self.queue_original.borrow_mut();
        let original_at = at.min(original.len());
        original.insert(original_at, track);
        drop(original);
        if self.queue_is_visible() {
            self.fill_queue_panel();
        }
        at
    }

    pub(super) fn remove_queue_entry(self: &Rc<Self>, index: usize) -> bool {
        if (self.current.borrow().is_some() && index <= self.index.get())
            || index >= self.queue.borrow().len()
        {
            return false;
        }
        let removed = self.queue.borrow_mut().remove(index);
        if index <= self.user_queue_end.get() {
            self.user_queue_end.set(self.user_queue_end.get().saturating_sub(1));
        }
        let mut original = self.queue_original.borrow_mut();
        let same_source = |t: &Track| {
            t.reference == removed.reference && t.playback_ref == removed.playback_ref
                && t.require_full_audio == removed.require_full_audio
        };
        if original.get(index).is_some_and(same_source) {
            original.remove(index);
        } else if let Some(position) = original.iter().enumerate().skip(self.index.get() + 1)
            .find_map(|(position, t)| same_source(t).then_some(position))
        {
            original.remove(position);
        }
        drop(original);
        if self.queue_is_visible() {
            self.fill_queue_panel();
        }
        true
    }
    // `to` is an insertion gap, so len is a valid target. Crossing the section
    // boundary adopts the destination section, preserving positional duplicates.
    pub(super) fn reorder_queue(self: &Rc<Self>, from: usize, to: usize) -> bool {
        let len = self.queue.borrow().len();
        if from <= self.index.get() || to <= self.index.get() || from >= len || to > len {
            return false;
        }
        let original_end = self.user_queue_end.get();
        let end = original_end.max(self.index.get());
        let was_user = from <= end;
        let destination_user = to <= end + 1;
        let mut queue = self.queue.borrow_mut();
        let t = queue.remove(from);
        let target = if from < to { to - 1 } else { to };
        queue.insert(target, t);
        self.user_queue_end.set(match (was_user, destination_user) {
            (true, false) => end.saturating_sub(1),
            (false, true) => end + 1,
            _ => original_end,
        });
        *self.queue_original.borrow_mut() = queue.clone();
        drop(queue);
        self.shuffle_button.set_active(false);
        self.fill_queue_panel();
        true
    }
    pub(super) fn install_queue_drag(self: &Rc<Self>) {
        let drag = gtk::GestureDrag::new();
        drag.set_button(1);
        drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        let moving = Rc::new(Cell::new(None::<(usize, usize)>));
        let origin = Rc::new(Cell::new(0.));
        let state = moving.clone();
        let y_origin = origin.clone();
        let weak = Rc::downgrade(self);
        drag.connect_drag_begin(move |g, x, y| {
            let Some(u) = weak.upgrade() else {
                return;
            };
            state.set(None);
            let bounds = u.queue_panel.compute_bounds(&u.window);
            let x = x + bounds.as_ref().map(|b| b.x() as f64).unwrap_or(0.);
            let y = y + bounds.as_ref().map(|b| b.y() as f64).unwrap_or(0.);
            if u.queue_is_visible() && u.panel_revealer.reveals_child() {
                let rows = queue_positions(&u.queue_panel.clone().upcast(), &u.window);
                if let Some(row) = rows.iter().find(|r| {
                    let rx = r["x"].as_f64().unwrap_or(0.);
                    let ry = r["y"].as_f64().unwrap_or(0.);
                    x >= rx
                        && x < rx + r["width"].as_f64().unwrap_or(0.) - 40.
                        && y >= ry
                        && y < ry + r["height"].as_f64().unwrap_or(0.)
                }) {
                    let from = row["index"].as_u64().unwrap_or(0) as usize;
                    if from > u.index.get() {
                        y_origin.set(y);
                        state.set(Some((from, from)));
                        return;
                    }
                }
            }
            g.set_state(gtk::EventSequenceState::Denied);
        });
        let state = moving.clone();
        let weak = Rc::downgrade(self);
        drag.connect_drag_update(move |gesture, _, dy| {
            let (Some(u), Some((from, _))) = (weak.upgrade(), state.get()) else {
                return;
            };
            if dy.abs() < 5. {
                return;
            }
            gesture.set_state(gtk::EventSequenceState::Claimed);
            let pointer = origin.get() + dy;
            let rows = queue_positions(&u.queue_panel.clone().upcast(), &u.window);
            let gap = rows
                .iter()
                .find_map(|r| {
                    if pointer < r["y"].as_f64()? + r["height"].as_f64()? / 2. {
                        r["index"].as_u64().map(|i| i as usize)
                    } else {
                        None
                    }
                })
                .unwrap_or(u.queue.borrow().len())
                .max(u.index.get() + 1);
            state.set(Some((from, gap)));
            mark_insertion(
                &u.queue_panel.clone().upcast(),
                Some(gap),
                u.queue.borrow().len(),
            );
        });
        let cancel_state=moving.clone();
        let weak = Rc::downgrade(self);
        drag.connect_drag_end(move |_, _, dy| {
            if let Some(u) = weak.upgrade() {
                mark_insertion(&u.queue_panel.clone().upcast(), None, 0);
                if let Some((from, to)) = moving.take() {
                    if dy.abs() >= 5. {
                        u.reorder_queue(from, to);
                    }
                }
            }
        });
        let keys=gtk::EventControllerKey::new();keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak=Rc::downgrade(self);let cancel_drag=drag.clone();
        keys.connect_key_pressed(move |_,key,_,_|{
            if key==gdk::Key::Escape && cancel_state.take().is_some() {
                cancel_drag.reset();
                if let Some(u)=weak.upgrade(){mark_insertion(&u.queue_panel.clone().upcast(),None,0);}
                glib::Propagation::Stop
            }else{glib::Propagation::Proceed}
        });
        self.window.add_controller(keys);
        self.queue_panel.add_controller(drag);
    }
    pub(super) fn fill_queue_panel(self: &Rc<Self>) {
        let rows: Vec<_> = self
            .queue
            .borrow()
            .iter()
            .cloned()
            .enumerate()
            .skip(self.index.get())
            .collect();
        if let Some(model) = self.queue_model.borrow().as_ref() {
            let old: Vec<_> = (0..model.n_items())
                .filter_map(|i| {
                    model
                        .item(i)
                        .and_downcast::<glib::BoxedAnyObject>()
                        .map(|o| o.borrow::<(usize, Track)>().clone())
                })
                .collect();
            let prefix = old.iter().zip(&rows).take_while(|(a, b)| a == b).count();
            let suffix = old[prefix..]
                .iter()
                .rev()
                .zip(rows[prefix..].iter().rev())
                .take_while(|(a, b)| a == b)
                .count();
            let replacements: Vec<_> = rows[prefix..rows.len() - suffix]
                .iter()
                .cloned()
                .map(glib::BoxedAnyObject::new)
                .collect();
            model.splice(
                prefix as u32,
                (old.len() - prefix - suffix) as u32,
                &replacements,
            );
            refresh_queue_boundaries(
                &self.queue_panel.clone().upcast(),
                self.index.get(),
                self.user_queue_end.get(),
            );
            return;
        }
        let model = gio::ListStore::new::<glib::BoxedAnyObject>();
        for row in rows {
            model.append(&glib::BoxedAnyObject::new(row));
        }
        *self.queue_model.borrow_mut() = Some(model.clone());
        let selection = gtk::NoSelection::new(Some(model));
        let factory = gtk::SignalListItemFactory::new();
        let u = self.clone();
        factory.connect_setup(move |_, obj| {
            let item = obj.downcast_ref::<gtk::ListItem>().unwrap();
            let row = hbox(8);
            row.add_css_class("queue-row");
            let picture = gtk::Picture::new();
            picture.set_can_shrink(true);
            picture.set_size_request(44, 44);
            picture.set_content_fit(gtk::ContentFit::Cover);
            row.append(&cover_frame(&picture, 44));
            let texts = vbox(2);
            texts.set_hexpand(true);
            texts.set_valign(gtk::Align::Center);
            let title = label("", "track-title");
            title.set_ellipsize(gtk::pango::EllipsizeMode::End);
            title.set_max_width_chars(20);
            let weak = item.downgrade();
            u.link_track_title(&title, move || {
                let object = weak
                    .upgrade()?
                    .item()?
                    .downcast::<glib::BoxedAnyObject>()
                    .ok()?;
                let track = object.borrow::<(usize, Track)>().1.clone();
                Some(track)
            });
            let weak = item.downgrade();
            let artist = u.metadata_line(move || {
                let object = weak
                    .upgrade()?
                    .item()?
                    .downcast::<glib::BoxedAnyObject>()
                    .ok()?;
                let track = object.borrow::<(usize, Track)>().1.clone();
                Some(track)
            });
            texts.append(&title);
            texts.append(&artist);
            row.append(&texts);
            let remove = icon_button("window-close-symbolic", "Убрать из очереди", "flat");
            row.append(&remove);
            let weak = item.downgrade();
            let ui = u.clone();
            remove.connect_clicked(move |_| {
                if let Some(item) = weak.upgrade() {
                    if let Some(obj) = item.item().and_downcast::<glib::BoxedAnyObject>() {
                        let (i, _) = *obj.borrow::<(usize, Track)>();
                        ui.remove_queue_entry(i);
                    }
                }
            });
            let context = gtk::GestureClick::new();
            context.set_button(3);
            let weak = item.downgrade();
            let ui = u.clone();
            context.connect_pressed(move |g, _, x, y| {
                if let Some(item) = weak.upgrade() {
                    if let Some(obj) = item.item().and_downcast::<glib::BoxedAnyObject>() {
                        let data = obj.borrow::<(usize, Track)>();
                        if let Some(w) = g.widget() {
                            ui.context_item(Item::Track(data.1.clone()), data.0, &w, x, y);
                        }
                    }
                }
            });
            row.add_controller(context);
            let drop = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);
            let weak = item.downgrade();
            let ui = u.clone();
            drop.connect_drop(move |target, value, _, y| {
                let Ok(value) = value.get::<String>() else {
                    return false;
                };
                let Some(from) = value.strip_prefix("queue:").and_then(|s| s.parse().ok()) else {
                    return false;
                };
                let Some(item) = weak.upgrade() else {
                    return false;
                };
                let Some(obj) = item.item().and_downcast::<glib::BoxedAnyObject>() else {
                    return false;
                };
                let to = obj.borrow::<(usize, Track)>().0;
                let after = target.widget().is_some_and(|w| y > w.height() as f64 / 2.);
                ui.reorder_queue(from, to + usize::from(after))
            });
            drop.connect_motion(|target, _, y| {
                if let Some(w) = target.widget() {
                    w.remove_css_class("insert-before");
                    w.remove_css_class("insert-after");
                    w.add_css_class(if y < w.height() as f64 / 2. {
                        "insert-before"
                    } else {
                        "insert-after"
                    });
                }
                gdk::DragAction::MOVE
            });
            drop.connect_leave(|target| {
                if let Some(w) = target.widget() {
                    w.remove_css_class("insert-before");
                    w.remove_css_class("insert-after");
                }
            });
            row.add_controller(drop);
            item.set_child(Some(&row));
        });
        let u = self.clone();
        factory.connect_bind(move |_, obj| {
            let item = obj.downcast_ref::<gtk::ListItem>().unwrap();
            let Some(obj) = item.item().and_downcast::<glib::BoxedAnyObject>() else {
                return;
            };
            let data = obj.borrow::<(usize, Track)>();
            let row = item.child().unwrap();
            row.set_widget_name(&format!("queue-row-{}", data.0));
            row.remove_css_class("source-start");
            if u.user_queue_end.get() > u.index.get() && data.0 == u.user_queue_end.get() + 1 {
                row.add_css_class("source-start");
            }
            let frame = child(&row, 0);
            let picture = frame
                .downcast::<gtk::Overlay>()
                .unwrap()
                .last_child()
                .unwrap()
                .downcast::<gtk::Picture>()
                .unwrap();

            let texts = child(&row, 1);
            child(&texts, 0)
                .downcast::<gtk::Label>()
                .unwrap()
                .set_text(&data.1.title);
            set_metadata_line(
                &child(&texts, 1).downcast::<gtk::Box>().unwrap(),
                &data.1.artist,
                data.1.album_title.as_deref().unwrap_or(""),
            );
            let current = data.0 == u.index.get() && u.current.borrow().is_some();
            if current {
                row.add_css_class("playing")
            } else {
                row.remove_css_class("playing")
            }
            child(&row, 2).set_sensitive(!current);
            u.load_cover(&data.1.cover(), &picture);
        });
        let list = gtk::ListView::new(Some(selection), Some(factory));
        list.set_single_click_activate(false);
        let u = self.clone();
        list.connect_activate(move |l, p| {
            if let Some(obj) = l
                .model()
                .and_then(|m| m.item(p))
                .and_downcast::<glib::BoxedAnyObject>()
            {
                let i = obj.borrow::<(usize, Track)>().0;
                u.play_index(i);
            }
        });
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build();
        self.queue_panel.append(&scroll);
    }
    pub(super) fn fill_history_panel(self: &Rc<Self>) {
        while let Some(child) = self.history_panel.first_child() {
            self.history_panel.remove(&child);
        }
        let list = vbox(4);
        for t in self.played_history.borrow().iter().take(100) {
            let b = gtk::Button::new();
            b.add_css_class("queue-track");
            let row = hbox(8);
            let picture = gtk::Picture::new();
            picture.set_size_request(44, 44);
            picture.set_can_shrink(true);
            picture.set_content_fit(gtk::ContentFit::Cover);
            row.append(&cover_frame(&picture, 44));
            self.defer_cover(&t.cover(), &picture);
            let texts = vbox(2);
            let title = label(&t.title, "track-title");
            let link_track = t.clone();
            self.link_track_title(&title, move || Some(link_track.clone()));
            title.set_max_width_chars(20);
            title.set_ellipsize(gtk::pango::EllipsizeMode::End);
            texts.append(&title);
            let metadata = t.clone();
            texts.append(&self.metadata_line(move || Some(metadata.clone())));
            row.append(&texts);
            let context = gtk::GestureClick::new();
            context.set_button(3);
            let ui = self.clone();
            let track = t.clone();
            context.connect_pressed(move |g, _, x, y| {
                if let Some(w) = g.widget() {
                    ui.context_item(Item::Track(track.clone()), 0, &w, x, y);
                }
            });
            b.add_controller(context);
            b.set_child(Some(&row));
            let u = self.clone();
            let t = t.clone();
            b.connect_clicked(move |_| {
                u.invalidate_continuation();
                u.user_queue_end.set(0);
                *u.wave.borrow_mut() = None;
                *u.queue_original.borrow_mut() = vec![t.clone()];
                *u.queue.borrow_mut() = vec![t.clone()];
                u.play_index(0);
            });
            list.append(&b);
        }
        if self.played_history.borrow().is_empty() {
            list.append(&label("Пока нет прослушиваний", "muted"));
        }
        self.history_panel.append(
            &gtk::ScrolledWindow::builder()
                .vexpand(true)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&list)
                .build(),
        );
    }
    pub(super) fn activate_or_toggle(self: &Rc<Self>, index: usize) {
        let is_current=self.items.borrow().get(index).is_some_and(|i|matches!(i,Item::Track(t) if self.current.borrow().as_ref().is_some_and(|c|c.reference==t.reference)));
        if is_current {
            self.toggle_play()
        } else {
            self.activate(index);
        }
    }
    pub(super) fn refresh_track_rows(&self) {
        let current = self
            .current
            .borrow()
            .as_ref()
            .map(|t| t.reference.clone())
            .unwrap_or_default();
        let key = format!("{}:{}", current, self.playing.get());
        if *self.row_state.borrow() == key {
            return;
        }
        *self.row_state.borrow_mut() = key;
        self.active_rows.borrow_mut().retain(|weak| {
            let Some(item) = weak.upgrade() else {
                return false;
            };
            let Some(obj) = item.item().and_downcast::<glib::BoxedAnyObject>() else {
                return true;
            };
            let Some(row) = item.child() else {
                return true;
            };
            let data = obj.borrow::<Item>();
            let active = matches!(&*data,Item::Track(t) if t.reference==current);
            if active {
                row.add_css_class("playing")
            } else {
                row.remove_css_class("playing")
            }
            if let Ok(button) = child(&row, 0).downcast::<gtk::Button>() {
                set_md_icon(
                    &button,
                    if active && self.playing.get() {
                        "media-playback-pause-symbolic"
                    } else {
                        "media-playback-start-symbolic"
                    },
                );
            }
            true
        });
    }
}

fn mark_insertion(widget: &gtk::Widget, gap: Option<usize>, len: usize) {
    widget.remove_css_class("insert-before");
    widget.remove_css_class("insert-after");
    if let Some(index) = widget
        .widget_name()
        .strip_prefix("queue-row-")
        .and_then(|s| s.parse::<usize>().ok())
    {
        if gap == Some(index) {
            widget.add_css_class("insert-before");
        }
        if gap == Some(len) && index + 1 == len {
            widget.add_css_class("insert-after");
        }
    }
    let mut current = widget.first_child();
    while let Some(child) = current {
        mark_insertion(&child, gap, len);
        current = child.next_sibling();
    }
}

fn refresh_queue_boundaries(widget: &gtk::Widget, current: usize, user_end: usize) {
    if let Some(index) = widget
        .widget_name()
        .strip_prefix("queue-row-")
        .and_then(|s| s.parse::<usize>().ok())
    {
        widget.remove_css_class("source-start");
        if user_end > current && index == user_end + 1 {
            widget.add_css_class("source-start");
        }
    }
    let mut child = widget.first_child();
    while let Some(w) = child {
        refresh_queue_boundaries(&w, current, user_end);
        child = w.next_sibling();
    }
}
