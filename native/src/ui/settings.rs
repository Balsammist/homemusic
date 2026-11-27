use super::*;

const SECTIONS: [(&str, &str); 8] = [
    ("Внешний вид", "preferences-system-symbolic"),
    ("Воспроизведение", "audio-volume-high-symbolic"),
    ("Библиотека", "folder-music-symbolic"),
    ("Загрузки", "folder-download-symbolic"),
    ("Сочетания клавиш", "input-keyboard-symbolic"),
    ("Соединение", "network-wired-symbolic"),
    ("Навигация", "go-home-symbolic"),
    ("Тексты песен", "text-x-generic-symbolic"),
];

const APPEARANCE: &[&str] = &[
    "theme_mode",
    "text_size",
    "compact",
    "motion_mode",
    "shape",
    "slider",
    "accent",
    "sidebar_width",
    "right_panel_width",
    "deep_dark",
    "persistent_card_play",
    "palette_source",
    "theme_seed",
    "palette_variant",
    "progress_style",
    "volume_style",
    "wave_height",
    "wave_length",
    "wave_speed",
    "cover_style",
    "player_tone",
    "player_button_shape",
    "player_album_line",
    "player_cover_fit",
    "system_window_frame",
];
const LYRICS: &[&str] = &[
    "lyrics_size",
    "lyrics_spacing",
    "lyrics_alignment",
    "lyrics_focus",
    "lyrics_follow",
    "lyrics_seek",
    "lyrics_offset",
    "lyrics_active_alignment",
    "lyrics_agents",
    "lyrics_neutral",
    "lyrics_animation",
    "lyrics_in_player",
    "lyrics_live_in_player",
    "lyrics_hide_status",
    "lyrics_real_timing",
    "lyrics_soft_focus",
    "lyrics_prefer_words",
    "lyrics_lrclib",
    "lyrics_lrcred",
    "lyrics_kugou",
    "lyrics_simpmusic",
    "lyrics_provider_first",
    "lyrics_romanize",
    "lyrics_romanize_main",
    "lyrics_romanize_script",
    "lyrics_subtitle_size",
    "lyrics_translation",
    "lyrics_translate_language",
    "lyrics_translate_formality",
    "lyrics_deepl_key",
];
const DOWNLOADS: &[&str] = &["download_artwork", "download_lyrics", "download_info"];
const START_PAGES: &[&str] = &["home", "library", "likes", "playlists", "downloads", "last"];
const START_LABELS: &[&str] = &[
    "Главная",
    "Библиотека",
    "Любимые треки",
    "Плейлисты",
    "Загрузки",
    "Последний открытый раздел",
];
pub(super) const SEARCH_SCOPES: &[&str] = &["Каталог", "Файлы", "Перезаливы"];
pub(super) const SEARCH_KINDS: &[&str] = &[
    "Все результаты",
    "Треки",
    "Альбомы",
    "Исполнители",
    "В библиотеке",
];

pub(super) fn default_search_scope() -> u32 {
    pref_index("default_search_scope", 0, SEARCH_SCOPES.len() as u64)
}

pub(super) fn default_search_kind() -> u32 {
    pref_index("default_search_kind", 0, SEARCH_KINDS.len() as u64)
}

pub(super) fn default_collection_view() -> u32 {
    pref_index("default_collection_view", 0, 2)
}

fn startup_page() -> String {
    let page = read_pref("startup_page");
    if page.as_str() == Some("last") {
        return remembered_page(&read_pref("last_page"))
            .unwrap_or("home")
            .to_string();
    }
    remembered_page(&page).unwrap_or("home").to_string()
}

fn remembered_page(page: &Value) -> Option<&str> {
    page.as_str().filter(|page| START_PAGES[..5].contains(page))
}

pub(super) fn right_panel_startup() -> Option<bool> {
    match read_pref("right_panel_startup").as_str() {
        Some("show") => Some(true),
        Some("hide") => Some(false),
        _ => None,
    }
}

type Binding = (gdk::Key, gdk::ModifierType);

#[derive(Clone, Copy)]
enum Shortcut {
    Search,
    Settings,
    Details,
    Play,
    Next,
    Restart,
    Queue,
    Back,
    Forward,
}

const SHORTCUTS: [(Shortcut, &str, &str, &str); 9] = [
    (Shortcut::Search, "search", "Поиск", "<Control>k"),
    (
        Shortcut::Settings,
        "settings",
        "Настройки",
        "<Control>comma",
    ),
    (
        Shortcut::Details,
        "details",
        "Сведения о треке",
        "<Control>l",
    ),
    (Shortcut::Play, "play", "Воспроизвести / пауза", "space"),
    (Shortcut::Next, "next", "Следующий трек", "<Control>Right"),
    (
        Shortcut::Restart,
        "restart",
        "В начало трека",
        "<Control>Left",
    ),
    (Shortcut::Queue, "queue", "Открыть очередь", "<Control>q"),
    (Shortcut::Back, "back", "Предыдущая страница", "<Alt>Left"),
    (
        Shortcut::Forward,
        "forward",
        "Следующая страница",
        "<Alt>Right",
    ),
];

#[derive(Clone, Copy, Default)]
enum Connection {
    #[default]
    Unchecked,
    Checking,
    Connected,
    Disconnected,
}

impl Connection {
    fn description(self) -> &'static str {
        match self {
            Self::Unchecked => "Соединение ещё не проверено",
            Self::Checking => "Подключаемся к HomeMusic…",
            Self::Connected => "Сервер доступен · сеанс активен",
            Self::Disconnected => "Не удалось подключиться к серверу",
        }
    }
}

pub(super) struct Settings {
    section: Cell<u32>,
    palette_preferences: RefCell<Vec<Value>>,
    shortcuts: RefCell<Vec<Binding>>,
    connection: Cell<Connection>,
    connection_label: glib::WeakRef<gtk::Label>,
    retry_button: glib::WeakRef<gtk::Button>,
    authenticated: Cell<bool>,
    pub(super) record_history: Cell<bool>,
    last_page: RefCell<String>,
}

impl Settings {
    pub(super) fn new() -> Self {
        let saved = read_pref("shortcuts");
        let defaults = || {
            SHORTCUTS
                .iter()
                .map(|(_, _, _, key)| gtk::accelerator_parse(*key).unwrap())
        };
        let shortcuts: Vec<_> = SHORTCUTS
            .iter()
            .zip(defaults())
            .map(|((_, id, _, _), fallback)| {
                saved[*id]
                    .as_str()
                    .and_then(gtk::accelerator_parse)
                    .filter(|binding| allowed(*binding))
                    .unwrap_or(fallback)
            })
            .collect();
        let shortcuts = if bindings_conflict(&shortcuts) {
            defaults().collect()
        } else {
            shortcuts
        };
        Self {
            section: Cell::new(0),
            palette_preferences: RefCell::new(Vec::new()),
            shortcuts: RefCell::new(shortcuts),
            connection: Cell::new(Connection::Unchecked),
            connection_label: glib::WeakRef::new(),
            retry_button: glib::WeakRef::new(),
            authenticated: Cell::new(false),
            record_history: Cell::new(pref_bool("record_history", true)),
            last_page: RefCell::new(
                remembered_page(&read_pref("last_page"))
                    .unwrap_or("home")
                    .to_string(),
            ),
        }
    }

    fn set_connection(&self, connection: Connection) {
        self.connection.set(connection);
        if let Some(label) = self.connection_label.upgrade() {
            label.set_text(connection.description());
        }
        if let Some(button) = self.retry_button.upgrade() {
            button.set_sensitive(!matches!(connection, Connection::Checking));
        }
    }
}

impl Ui {
    pub(super) fn save_preferences(&self, values: &[(&str, Value)], remove: &[&str]) -> bool {
        match crate::config::update_settings(values, remove) {
            Ok(()) => true,
            Err(error) => {
                self.notice(&format!("Не удалось сохранить настройки · {error}"), true);
                false
            }
        }
    }

    pub(super) fn preferences(self: &Rc<Self>) {
        self.navigate("settings", false);
    }

    pub(super) fn show_preferences(self: &Rc<Self>) {
        let content = vbox(20);
        content.add_css_class("settings-page");
        content.set_margin_top(6);
        content.set_margin_bottom(28);
        content.set_margin_start(20);
        content.set_margin_end(20);
        let search = gtk::SearchEntry::new();
        search.set_widget_name("settings-search");
        search.set_placeholder_text(Some("Поиск по настройкам"));
        search.update_property(&[gtk::accessible::Property::Label("Поиск по настройкам")]);
        content.append(&search);

        let navigation = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .min_children_per_line(1)
            .max_children_per_line(SECTIONS.len() as u32)
            .column_spacing(6)
            .row_spacing(6)
            .build();
        navigation.add_css_class("settings-navigation");
        content.append(&navigation);

        let pages = gtk::Stack::new();
        pages.set_widget_name("settings-pages");
        pages.set_hhomogeneous(false);
        pages.set_vhomogeneous(false);
        let results = vbox(18);
        results.set_widget_name("settings-results");
        pages.add_named(&results, Some("results"));
        let surface = motion::surface(&pages);
        content.append(&surface);
        let mut first = None;
        for (index, (title, icon)) in SECTIONS.iter().enumerate() {
            let tab = gtk::ToggleButton::new();
            tab.set_widget_name(&format!("settings-section-{index}"));
            tab.add_css_class("settings-tab");
            tab.update_property(&[gtk::accessible::Property::Label(title)]);
            let contents = hbox(8);
            let icon = gtk::Image::from_icon_name(icon);
            icon.set_pixel_size(16);
            contents.append(&icon);
            contents.append(&label(title, ""));
            tab.set_child(Some(&contents));
            if let Some(first) = &first {
                tab.set_group(Some(first));
            } else {
                first = Some(tab.clone());
            }
            tab.set_active(index as u32 == self.settings.section.get());
            let u = Rc::downgrade(self);
            let body = pages.downgrade();
            let search = search.downgrade();
            tab.connect_clicked(move |tab| {
                let (Some(u), Some(body), Some(search)) =
                    (u.upgrade(), body.upgrade(), search.upgrade())
                else {
                    return;
                };
                tab.set_active(true);
                let previous = u.settings.section.replace(index as u32);
                let searching = body.visible_child_name().as_deref() == Some("results");
                search.set_text("");
                if previous != index as u32 || searching {
                    u.render_settings_section(index, &body);
                    u.settings_view.vadjustment().set_value(0.);
                    motion::page(&body, u.motion_mode.get());
                }
            });
            navigation.insert(&tab, -1);
            if let Some(wrapper) = tab.parent() {
                wrapper.set_focusable(false);
            }
        }
        let u = Rc::downgrade(self);
        let body = pages.downgrade();
        search.connect_search_changed(move |search| {
            if let (Some(u), Some(body)) = (u.upgrade(), body.upgrade()) {
                let query = search.text().to_lowercase();
                let terms: Vec<_> = query.split_whitespace().collect();
                let showing_results = body.visible_child_name().as_deref() == Some("results");
                if terms.is_empty() {
                    if showing_results {
                        u.render_settings_section(u.settings.section.get() as usize, &body);
                        motion::page(&body, u.motion_mode.get());
                    }
                } else if let Some(results) = body.child_by_name("results").and_downcast::<gtk::Box>() {
                    if !showing_results {
                        for index in 0..SECTIONS.len() {
                            if let Some(page) = body.child_by_name(&index.to_string()) {
                                body.remove(&page);
                            }
                        }
                    }
                    u.render_settings_results(&results, &terms);
                    body.set_visible_child(&results);
                }
            }
        });
        self.render_settings_section(self.settings.section.get() as usize, &pages);
        self.settings_view.set_child(Some(&content));
        self.settings_view.vadjustment().set_value(0.);
        if crate::config::read_settings().is_err() {
            self.notice(
                "Не удалось прочитать файл настроек · проверьте формат JSON и права доступа",
                true,
            );
        }
    }

    pub(super) fn settings_section(&self) -> u32 {
        self.settings.section.get()
    }

    fn render_settings_section(self: &Rc<Self>, section: usize, stack: &gtk::Stack) {
        let name = section.to_string();
        if stack.child_by_name(&name).is_none() {
            let page = vbox(18);
            self.build_settings_section(section, &page);
            stack.add_named(&page, Some(&name));
        }
        stack.set_visible_child_name(&name);
        if let Some(results) = stack.child_by_name("results").and_downcast::<gtk::Box>() {
            clear_settings_body(&results);
            results.remove_css_class("settings-results");
        }
    }

    fn build_settings_section(self: &Rc<Self>, section: usize, page: &gtk::Box) {
        page.append(&label(SECTIONS[section].0, "section-title"));
        match section {
            0 => self.appearance_settings(page),
            1 => self.playback_settings(page),
            2 => self.library_settings(page),
            3 => self.download_settings(page),
            4 => self.shortcut_settings(page),
            5 => self.connection_settings(page),
            6 => self.navigation_settings(page),
            7 => self.lyric_settings(page),
            _ => unreachable!(),
        }
    }

    fn render_settings_results(self: &Rc<Self>, page: &gtk::Box, terms: &[&str]) {
        if !page.has_css_class("settings-results") {
            clear_settings_body(page);
            page.add_css_class("settings-results");
            for section in 0..SECTIONS.len() {
                let body = vbox(12);
                self.build_settings_section(section, &body);
                page.append(&body);
            }
            page.append(&label("Настройки не найдены", "muted"));
        }
        let mut child = page.first_child();
        let mut found = false;
        for (title, _) in SECTIONS {
            let body = child.and_downcast::<gtk::Box>().unwrap();
            child = body.next_sibling();
            let matches = filter_settings(&body, &title.to_lowercase(), terms);
            body.set_visible(matches);
            found |= matches;
        }
        if let Some(empty) = child {
            empty.set_visible(!found);
        }
    }

    pub(super) fn apply_appearance(self: &Rc<Self>) {
        let settings = Value::Object(crate::config::read_settings().unwrap_or_default());
        self.apply_preferences_appearance(&settings);
    }

    fn apply_preferences_appearance(&self, settings: &Value) {
        let progress = progress_style(settings);
        let bars_changed = material::Bars::from_settings(settings, progress).install();
        let cover = settings["cover_style"].as_u64().unwrap_or_default();
        for (class, active) in [
            ("compact", settings["compact"].as_bool().unwrap_or(false)),
            ("expressive", settings["shape"].as_u64() == Some(1)),
            ("md-slider", progress == 3),
            ("progress-line", progress == 1),
            ("progress-wave", progress == 2),
            ("rounded-artwork", cover == 1),
            ("square-artwork", cover == 2),
            ("player-tonal", settings["player_tone"].as_u64() == Some(1)),
            (
                "player-elevated",
                settings["player_tone"].as_u64() == Some(2),
            ),
            (
                "player-play-rect",
                settings["player_button_shape"].as_u64() == Some(1),
            ),
            (
                "player-play-tonal",
                settings["player_button_shape"].as_u64() == Some(2),
            ),
            (
                "persistent-card-play",
                settings["persistent_card_play"].as_bool().unwrap_or(false),
            ),
        ] {
            if active {
                self.window.add_css_class(class);
            } else {
                self.window.remove_css_class(class);
            }
        }
        let motion_mode = settings["motion_mode"].as_u64().unwrap_or(1).min(2) as u32;
        self.motion_mode.set(motion_mode);
        motion::apply(&self.window.clone().upcast(), motion_mode);
        let framed = settings["system_window_frame"].as_bool().unwrap_or(false);
        self.window.set_decorated(framed);
        let close = self
            .top_bar
            .end_widget()
            .and_downcast::<gtk::Box>()
            .and_then(|row| row.last_child())
            .filter(|child| child.has_css_class("custom-window-close"));
        if let Some(close) = close {
            close.set_visible(!framed);
        }
        self.lyrics_button
            .set_visible(settings["lyrics_in_player"].as_bool().unwrap_or(true));
        self.now_lyrics.configure(self, settings);
        self.mini_album
            .set_visible(settings["player_album_line"].as_bool().unwrap_or(true));
        self.mini_picture.set_content_fit(
            if settings["player_cover_fit"].as_bool().unwrap_or(true) {
                gtk::ContentFit::Cover
            } else {
                gtk::ContentFit::Contain
            },
        );
        let palette = [
            "palette_source", "palette_variant", "theme_seed", "theme_mode",
            "accent", "deep_dark", "text_size",
        ].into_iter().map(|key| settings[key].clone()).collect::<Vec<_>>();
        if self.settings.palette_preferences.replace(palette.clone()) != palette {
            if settings["palette_source"].as_str() == Some("cover") {
                theme::update_cover(&self._theme, &self.mini_picture);
            }
            theme::reload(&self._theme);
        }
        if bars_changed {
            theme::repaint_controls();
        }
        self.timeline.canvas.queue_draw();
    }

    fn appearance_settings(self: &Rc<Self>, page: &gtk::Box) {
        let changes = |u: &Rc<Self>| u.apply_appearance();
        let colors = group(page, "Цвета");
        self.choice(
            &colors,
            "Тема",
            "Системная тема следует настройкам рабочего стола.",
            "theme_mode",
            &["Системная", "Светлая", "Тёмная"],
            match read_pref("theme_mode").as_str() {
                Some("light") => 1,
                Some("dark") => 2,
                _ => 0,
            },
            |index| json!(["system", "light", "dark"][index as usize]),
            changes,
        );
        self.palette_settings(&colors);
        self.toggle(
            &colors,
            "Почти чёрный фон",
            "Затемняет фон и панели в тёмной теме.",
            "deep_dark",
            false,
            |u, _| u.apply_appearance(),
        );
        let interface = group(page, "Интерфейс");
        self.choice(
            &interface,
            "Размер текста",
            "",
            "text_size",
            &["Мелкий", "Обычный", "Крупный"],
            pref_index("text_size", 1, 3),
            |index| json!(index),
            changes,
        );
        self.choice(
            &interface,
            "Анимации",
            "",
            "motion_mode",
            &["Выключены", "Стандартные", "Выразительные"],
            self.motion_mode.get(),
            |index| json!(index),
            changes,
        );
        let motion_sample = vbox(6);
        motion_sample.set_halign(gtk::Align::Start);
        motion_sample.set_width_request(260);
        let stack = gtk::Stack::new();
        stack.set_widget_name("motion-preview-pages");
        for (name, icon, title) in [
            ("albums", "folder-music-symbolic", "Альбомы"),
            ("artists", "system-users-symbolic", "Исполнители"),
        ] {
            let panel = hbox(10);
            panel.add_css_class("motion-sample-panel");
            panel.append(&symbol(icon));
            panel.append(&label(title, "settings-row-title"));
            stack.add_named(&panel, Some(name));
        }
        motion_sample.append(&motion::surface(&stack));
        let replay = button("Показать переход", None, "tonal");
        replay.set_widget_name("motion-preview-replay");
        replay.set_halign(gtk::Align::Start);
        let u = Rc::downgrade(self);
        replay.connect_clicked(move |_| {
            let page = if stack.visible_child_name().as_deref() == Some("albums") {
                "artists"
            } else {
                "albums"
            };
            stack.set_visible_child_name(page);
            if let Some(u) = u.upgrade() {
                motion::page(&stack, u.motion_mode.get());
            }
        });
        motion_sample.append(&replay);
        append_preview(&interface, "Предпросмотр анимации", &motion_sample);
        self.toggle(
            &interface,
            "Плотные списки",
            "Уменьшает высоту строк в списках треков.",
            "compact",
            false,
            |u, _| u.apply_appearance(),
        );
        self.toggle(
            &interface,
            "Всегда показывать кнопки воспроизведения",
            "Кнопки остаются видны на обложках без наведения курсора.",
            "persistent_card_play",
            false,
            |u, _| u.apply_appearance(),
        );
        self.toggle(
            &interface,
            "Системная рамка окна",
            "Показывает заголовок и рамку рабочего стола.",
            "system_window_frame",
            false,
            |u, _| u.apply_appearance(),
        );
        let player = group(page, "Плеер");
        let initial = progress_style(&Value::Object(
            crate::config::read_settings().unwrap_or_default(),
        ));
        self.choice(
            &player,
            "Форма шкалы перемотки",
            "XS и S — размеры ползунка Material 3 Expressive.",
            "progress_style",
            material::SEEK_STYLES,
            initial,
            |index| json!(index),
            changes,
        );
        self.choice(
            &player,
            "Цвет контейнера",
            "Фон и элементы плеера меняются вместе.",
            "player_tone",
            &["Surface", "Secondary container", "Surface container high"],
            pref_index("player_tone", 0, 3),
            |index| json!(index),
            changes,
        );
        self.choice(
            &player,
            "Кнопка воспроизведения",
            "Размер и действие кнопки сохраняются.",
            "player_button_shape",
            &[
                "Заполненная круглая",
                "Заполненная скруглённая",
                "Тональная круглая",
            ],
            pref_index("player_button_shape", 0, 3),
            |index| json!(index),
            changes,
        );
        self.toggle(
            &player,
            "Название альбома в плеере",
            "Добавляет третью строку под исполнителем внизу окна.",
            "player_album_line",
            true,
            |u, _| u.apply_appearance(),
        );
        let sample = vbox(2);
        sample.add_css_class("player");
        sample.add_css_class("player-preview");
        sample.set_hexpand(true);
        let transport = hbox(6);
        transport.set_halign(gtk::Align::Center);
        for (icon, class) in [
            ("media-skip-backward-symbolic", "flat"),
            ("media-playback-start-symbolic", "primary large-play"),
            ("media-skip-forward-symbolic", "flat"),
        ] {
            let preview = icon_button(icon, "Предпросмотр", class);
            preview.set_focusable(false);
            preview.set_can_target(false);
            transport.append(&preview);
        }
        sample.append(&transport);
        sample.append(&material::progress_preview(material::Bar::Progress));
        append_preview(&player, "Предпросмотр плеера", &sample);

        let sliders = group(page, "Громкость и волна");
        self.choice(
            &sliders, "Шкала громкости", "Форма регулятора в нижней панели.", "volume_style",
            material::SEEK_STYLES, pref_index("volume_style", 1, 4), |index| json!(index), changes,
        );
        append_preview(&sliders, "Предпросмотр громкости", &material::progress_preview(material::Bar::Volume));
        for (title, key, options) in [
            ("Высота волны", "wave_height", ["Низкая", "Средняя", "Высокая"]),
            ("Длина волны", "wave_length", ["Короткая", "Средняя", "Длинная"]),
            ("Движение волны", "wave_speed", ["Неподвижная", "Плавная", "Быстрая"]),
        ] {
            self.choice(&sliders, title, "Действует на обе шкалы со стилем «Волна».", key,
                &options, pref_index(key, 1, 3), |index| json!(index), changes);
        }

        let covers = group(page, "Обложки");
        self.choice(
            &covers,
            "Форма обложек",
            "Выразительный вариант сохраняет фигурные обложки миксов и волн.",
            "cover_style",
            &["Выразительная", "Скруглённый квадрат", "Квадрат"],
            pref_index("cover_style", 0, 3),
            |index| json!(index),
            changes,
        );
        self.toggle(
            &covers,
            "Заполнять мини-обложку",
            "Обрезает края вместо показа исходных пропорций.",
            "player_cover_fit",
            true,
            |u, _| u.apply_appearance(),
        );
        let art = artwork::cover(&[], 9, 56);
        art.set_halign(gtk::Align::Start);
        append_preview(&covers, "Предпросмотр обложки", &art);
        self.reset_section(
            page,
            "Сбросить оформление",
            APPEARANCE,
            |u| {
                u.sidebar_width.set(240);
                u.right_width.set(280);
                u.apply_appearance();
                u.update_panels();
            },
        );
    }

    fn palette_settings(self: &Rc<Self>, colors: &gtk::Box) {
        let apply = |u: &Rc<Self>| u.apply_appearance();
        let source = self.choice(
            colors,
            "Источник палитры",
            "Quickshell следует рабочему столу. Два других источника строят палитру Material 3.",
            "palette_source",
            &["Quickshell", "Свой цвет", "Обложка трека"],
            match read_pref("palette_source").as_str() {
                Some("custom") => 1,
                Some("cover") => 2,
                _ => 0,
            },
            |index| json!(["quickshell", "custom", "cover"][index as usize]),
            apply,
        );

        let dialog = gtk::ColorDialog::builder()
            .title("Исходный цвет палитры")
            .with_alpha(false)
            .modal(true)
            .build();
        let picker = gtk::ColorDialogButton::new(Some(dialog));
        picker.add_css_class("tonal");
        picker.set_widget_name("pref-theme_seed");
        let initial = read_pref("theme_seed");
        let seed = theme::seed_color(&initial);
        picker.set_rgba(&gdk::RGBA::parse(seed).expect("validated color seed"));
        let restored = Rc::new(Cell::new(picker.rgba()));

        let hex = gtk::Entry::new();
        hex.set_width_chars(8);
        hex.set_max_length(18);
        hex.set_text(seed);
        hex.set_widget_name("palette-hex");
        hex.update_property(&[gtk::accessible::Property::Label("Код цвета HEX")]);
        let input = hbox(8);
        input.append(&hex);
        input.append(&picker);
        colors.append(&row(
            "Исходный цвет",
            "Выберите цвет или введите HEX и нажмите Enter.",
            &input,
        ));
        let weak_hex = hex.downgrade();
        let u = Rc::downgrade(self);
        let saved = restored.clone();
        picker.connect_rgba_notify(move |picker| {
            let rgba = picker.rgba();
            if rgba == saved.get() {
                return;
            }
            let channel = |value: f32| (255. * value).round() as u8;
            let seed = format!(
                "#{:02X}{:02X}{:02X}",
                channel(rgba.red()),
                channel(rgba.green()),
                channel(rgba.blue())
            );
            if let Some(u) = u.upgrade()
                && u.save_preferences(&[("theme_seed", json!(seed))], &[])
            {
                saved.set(rgba);
                if let Some(hex) = weak_hex.upgrade() {
                    hex.set_text(&seed);
                }
                u.apply_appearance();
                return;
            }
            picker.set_rgba(&saved.get());
        });
        let u = Rc::downgrade(self);
        let weak_picker = picker.downgrade();
        hex.connect_activate(move |hex| {
            let value = hex.text();
            let value = value.trim();
            let valid = value.len() == 7
                && value.starts_with('#')
                && value[1..].bytes().all(|byte| byte.is_ascii_hexdigit());
            if valid && let Some(picker) = weak_picker.upgrade() {
                let rgba = gdk::RGBA::parse(value).expect("validated color seed");
                picker.set_rgba(&rgba);
            } else if let Some(u) = u.upgrade() {
                u.notice("Введите цвет в формате #RRGGBB", true);
            }
        });

        let variant = self.choice(
            colors,
            "Стиль палитры",
            "Алгоритм Material Color Utilities для выбранного цвета или обложки.",
            "palette_variant",
            &[
                "Tonal Spot",
                "Vibrant",
                "Expressive",
                "Monochrome",
                "Neutral",
                "Fidelity",
                "Content",
                "Rainbow",
                "Fruit Salad",
            ],
            pref_index("palette_variant", 0, 9),
            |index| json!(index),
            apply,
        );
        let enabled = source.selected() == 1;
        input.set_sensitive(enabled);
        variant.set_sensitive(source.selected() != 0);
        let weak_input = input.downgrade();
        let weak_variant = variant.downgrade();
        source.connect_selected_notify(move |control| {
            let enabled = control.selected() == 1;
            if let Some(input) = weak_input.upgrade() {
                input.set_sensitive(enabled);
            }
            if let Some(variant) = weak_variant.upgrade() {
                variant.set_sensitive(control.selected() != 0);
            }
        });

        let preview = hbox(6);
        preview.add_css_class("palette-preview");
        preview.append(&label("Акцент", "palette-sample-primary"));
        preview.append(&label("Тональная", "palette-sample-tonal"));
        preview.append(&label("Фон", "palette-sample-surface"));
        append_preview(&colors, "Предпросмотр палитры", &preview);
    }

    fn lyric_settings(self: &Rc<Self>, page: &gtk::Box) {
        let display = group(page, "Оформление текста");
        let preview = vbox(12);
        preview.add_css_class("lyrics-preview");
        preview.set_hexpand(true);
        for (words, state) in [
            ("Сквозь тишину за окном", "karaoke-past"),
            ("Первый свет на горизонте", "karaoke-active"),
            ("Город проснётся потом", "karaoke-future"),
        ] {
            let line = label(words, "karaoke-line");
            line.set_wrap(true);
            line.add_css_class(state);
            preview.append(&line);
        }
        update_lyrics_preview(&preview);
        let weak = preview.downgrade();
        let update = move |_: &Rc<Self>| {
            if let Some(preview) = weak.upgrade() {
                update_lyrics_preview(&preview);
            }
        };
        for (title, description, key, choices, initial) in [
            (
                "Размер текста",
                "В синхронизированных строках и обычном тексте.",
                "lyrics_size",
                ["Мелкий", "Обычный", "Крупный"],
                pref_index("lyrics_size", 1, 3),
            ),
            (
                "Интервал между строками",
                "Расстояние между строками караоке.",
                "lyrics_spacing",
                ["Плотный", "Обычный", "Широкий"],
                pref_index("lyrics_spacing", 1, 3),
            ),
            (
                "Выравнивание",
                "Расположение строк в окне текста.",
                "lyrics_alignment",
                ["По левому краю", "По центру", "По правому краю"],
                pref_index("lyrics_alignment", 0, 3),
            ),
        ] {
            self.choice(
                &display,
                title,
                description,
                key,
                &choices,
                initial,
                |index| json!(index),
                update.clone(),
            );
        }
        let weak = preview.downgrade();
        self.toggle(
            &display,
            "Приглушать соседние строки",
            "Текущая строка остаётся самой заметной.",
            "lyrics_focus",
            true,
            move |_, _| {
                if let Some(preview) = weak.upgrade() {
                    update_lyrics_preview(&preview);
                }
            },
        );
        self.choice(
            &display,
            "Выравнивание активной строки",
            "Расположение строки, которая звучит сейчас.",
            "lyrics_active_alignment",
            &[
                "Как остальной текст",
                "По левому краю",
                "По центру",
                "По правому краю",
            ],
            pref_index("lyrics_active_alignment", 0, 4),
            |index| json!(index),
            update.clone(),
        );
        self.choice(
            &display,
            "Цвет текста",
            "",
            "lyrics_neutral",
            &["Акцент темы", "Нейтральный"],
            u32::from(pref_bool("lyrics_neutral", false)),
            |index| json!(index == 1),
            update.clone(),
        );
        self.choice(
            &display,
            "Подсветка караоке",
            "Плавная подсветка использует тайминг слов, если он есть.",
            "lyrics_animation",
            &["Строкой", "По словам", "Плавная"],
            pref_index("lyrics_animation", 2, 3),
            |index| json!(index),
            update.clone(),
        );
        self.toggle(
            &display,
            "Разделять голоса",
            "Второй голос выравнивается к противоположному краю, если источник пометил голоса.",
            "lyrics_agents",
            true,
            |_, _| {},
        );
        self.toggle(
            &display,
            "Размывать дальние строки",
            "Степень размытия растёт с расстоянием от текущей строки.",
            "lyrics_soft_focus",
            false,
            |_, _| {},
        );
        let sample_row = preview
            .first_child()
            .and_then(|row| row.next_sibling())
            .and_downcast::<gtk::Label>();
        let origin = Cell::new(None);
        preview.add_tick_callback(move |preview, clock| {
            if let Some(line) = &sample_row {
                if !gtk::Settings::default().is_some_and(|s| s.is_gtk_enable_animations())
                    || !preview.has_css_class("karaoke-sample")
                {
                    if line.attributes().is_some() {
                        line.set_attributes(None);
                    }
                    return glib::ControlFlow::Continue;
                }
                super::details::paint_lyric_sample(
                    line,
                    clock.frame_time(),
                    &origin,
                    preview.has_css_class("karaoke-smooth-sample"),
                );
            }
            glib::ControlFlow::Continue
        });
        append_preview(&display, "Предпросмотр текста", &preview);

        let sync = group(page, "Синхронизация");
        self.toggle(
            &sync,
            "Автопрокрутка",
            "Показывает текущую строку по мере воспроизведения.",
            "lyrics_follow",
            true,
            |_, _| {},
        );
        self.toggle(
            &sync,
            "Перемотка по строке",
            "Нажатие на строку переносит воспроизведение к ней.",
            "lyrics_seek",
            true,
            |_, _| {},
        );
        self.toggle(
            &sync,
            "Учитывать конец строки",
            "В паузах между строками подсветка исчезает, если источник задал окончание.",
            "lyrics_real_timing",
            true,
            |_, _| {},
        );
        self.toggle(
            &sync,
            "Кнопка текста в плеере",
            "Текст также открывается через сведения о треке.",
            "lyrics_in_player",
            true,
            |u, _| u.apply_appearance(),
        );
        self.toggle(
            &sync,
            "Текущая строка в плеере",
            "Показывает синхронизированный текст в панели «Сейчас играет».",
            "lyrics_live_in_player",
            false,
            |u, _| u.apply_appearance(),
        );
        self.toggle(
            &sync,
            "Скрывать статус в полноэкранном режиме",
            "Убирает строку состояния над текстом.",
            "lyrics_hide_status",
            false,
            |_, _| {},
        );
        let offset = gtk::SpinButton::with_range(-30., 30., 0.1);
        offset.set_widget_name("pref-lyrics_offset");
        offset.set_digits(1);
        offset.set_value(LyricsAppearance::current().offset_seconds);
        let saved = Cell::new(offset.value());
        let u = Rc::downgrade(self);
        offset.connect_value_changed(move |control| {
            let seconds = control.value();
            if (seconds - saved.get()).abs() < 0.01 {
                return;
            }
            if let Some(u) = u.upgrade()
                && u.save_preferences(&[("lyrics_offset", json!(seconds))], &[])
            {
                saved.set(seconds);
            } else {
                control.set_value(saved.get());
            }
        });
        sync.append(&row(
            "Смещение, с",
            "Положительное значение показывает строки раньше.",
            &offset,
        ));
        let sources = group(page, "Поиск текста");
        self.toggle(
            &sources,
            "Предпочитать пословную синхронизацию",
            "Сравнивает ответы включённых источников, чтобы найти пословный тайминг.",
            "lyrics_prefer_words",
            true,
            |_, _| {},
        );
        self.choice(
            &sources,
            "Первый источник",
            "При одинаковом качестве текст берётся из приоритетного источника.",
            "lyrics_provider_first",
            &["LRCLIB", "lrc.red", "KuGou", "SimpMusic"],
            pref_index("lyrics_provider_first", 0, 4),
            |index| json!(index),
            |_| {},
        );
        for &(name, _, key) in super::lyric_sources::PROVIDERS {
            let description = if key == "lyrics_simpmusic" {
                "Используется для треков с идентификатором YouTube."
            } else {
                ""
            };
            self.toggle(&sources, name, description, key, true, |_, _| {});
        }
        let translate = group(page, "Перевод");
        self.choice(
            &translate,
            "Перевод через DeepL",
            "При переводе текст отправляется в DeepL. Для работы нужен ключ API.",
            "lyrics_translation",
            &["Выключен", "По кнопке", "При открытии текста"],
            pref_index("lyrics_translation", 0, 3),
            |index| json!(index),
            |_| {},
        );
        self.choice(
            &translate,
            "Язык перевода",
            "",
            "lyrics_translate_language",
            super::lyric_translation::LANGUAGES,
            pref_index(
                "lyrics_translate_language",
                0,
                super::lyric_translation::LANGUAGES.len() as u64,
            ),
            |index| json!(index),
            |_| {},
        );
        self.choice(
            &translate,
            "Формальность перевода",
            "",
            "lyrics_translate_formality",
            &["По умолчанию", "Менее формально", "Более формально"],
            pref_index("lyrics_translate_formality", 0, 3),
            |index| json!(index),
            |_| {},
        );
        let key = gtk::PasswordEntry::new();
        key.set_widget_name("pref-lyrics_deepl_key");
        let configured = read_pref("lyrics_deepl_key")
            .as_str()
            .is_some_and(|key| !key.is_empty());
        key.set_placeholder_text(Some(if configured {
            "Ключ сохранён"
        } else {
            "Введите ключ"
        }));
        key.update_property(&[gtk::accessible::Property::Label("Ключ DeepL API")]);
        let weak = Rc::downgrade(self);
        key.connect_activate(move |key| {
            if let Some(u) = weak.upgrade() {
                let text = key.text();
                let value = text.trim();
                if value.len() > 256
                    || !value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_:".contains(&b))
                {
                    u.notice("Проверьте формат ключа DeepL", true);
                    return;
                }
                let saved = if value.is_empty() {
                    u.save_preferences(&[], &["lyrics_deepl_key"])
                } else {
                    u.save_preferences(&[("lyrics_deepl_key", json!(value))], &[])
                };
                if saved {
                    key.set_placeholder_text(Some(if value.is_empty() {
                        "Введите ключ"
                    } else {
                        "Ключ сохранён"
                    }));
                    key.set_text("");
                }
            }
        });
        translate.append(&row(
            "Ключ DeepL API",
            "Нажмите Enter для сохранения. Пустое поле удаляет ключ.",
            &key,
        ));
        let romanize = group(page, "Романизация");
        self.toggle(
            &romanize,
            "Показывать романизацию",
            "Преобразует поддерживаемые письменности в латиницу с помощью ICU.",
            "lyrics_romanize",
            false,
            |_, _| {},
        );
        self.toggle(
            &romanize,
            "Романизация как основной текст",
            "Показывает оригинал под романизацией, если преобразование доступно.",
            "lyrics_romanize_main",
            false,
            |_, _| {},
        );
        self.choice(
            &romanize,
            "Письменность",
            "",
            "lyrics_romanize_script",
            &[
                "Авто",
                "Кириллица",
                "Китайская",
                "Японская",
                "Корейская",
                "Греческая",
                "Арабская",
                "Иврит",
            ],
            pref_index("lyrics_romanize_script", 0, 8),
            |index| json!(index),
            |_| {},
        );
        self.choice(
            &romanize,
            "Размер дополнительного текста",
            "",
            "lyrics_subtitle_size",
            &["Мелкий", "Обычный", "Крупный"],
            pref_index("lyrics_subtitle_size", 1, 3),
            |index| json!(index),
            |_| {},
        );
        self.reset_section(
            page,
            "Сбросить настройки текста",
            LYRICS,
            Self::apply_appearance,
        );
    }

    fn playback_settings(self: &Rc<Self>, page: &gtk::Box) {
        let queue = group(page, "Очередь");
        self.toggle(
            &queue,
            "Продолжать очередь «Моей волной»",
            "Когда закончится последний трек, HomeMusic подберёт следующие.",
            "autoplay",
            true,
            |u, active| u.autoplay.set(active),
        );
        self.toggle(
            &queue,
            "Восстанавливать очередь с паузой",
            "При запуске открывает трек и очередь прошлого сеанса.",
            "restore_playback",
            true,
            |_, _| {},
        );
        self.toggle(
            &queue,
            "Запоминать позицию трека",
            "При восстановлении очереди продолжает с места остановки.",
            "restore_track_position",
            true,
            |_, _| {},
        );
        self.toggle(
            &queue,
            "Сохранять паузу при переключении",
            "При переходе по очереди плеер остаётся на паузе.",
            "keep_paused_on_skip",
            false,
            |_, _| {},
        );
        self.toggle(
            &queue,
            "Не добавлять дубли в очередь",
            "Проверяет текущий и предстоящие треки при ручном добавлении.",
            "prevent_queue_duplicates",
            false,
            |_, _| {},
        );
        self.choice(
            &queue,
            "Шаг перемотки стрелками",
            "Работает, когда шкала перемотки в фокусе.",
            "seek_step",
            &["1 с", "5 с", "10 с", "15 с", "30 с"],
            pref_index("seek_step", 0, 5),
            |index| json!(index),
            |u| u.apply_seek_step(),
        );
        self.action(
            &queue,
            "Открыть очередь",
            "Посмотреть и переставить следующие треки.",
            "view-list-symbolic",
            Self::show_queue,
        );
        let sound = group(page, "Звук");
        let volume = gtk::Scale::new(
            gtk::Orientation::Horizontal,
            Some(&self.volume_control.adjustment()),
        );
        volume.set_size_request(112, -1);
        volume.set_draw_value(true);
        volume.set_format_value_func(|_, value| format!("{value:.0} %"));
        volume.set_widget_name("pref-volume");
        sound.append(&row("Громкость", "Меняет громкость плеера.", &volume));
        self.choice(
            &sound,
            "Ограничение громкости",
            "Ограничивает громкость плеера, включая системное управление.",
            "volume_limit",
            &["100 %", "85 %", "70 %", "50 %"],
            pref_index("volume_limit", 0, 4),
            |index| json!(index),
            Self::apply_volume_limit,
        );
        let stream = group(page, "Потоковое аудио");
        self.choice(
            &stream,
            "Качество звука",
            "Применяется со следующего трека.",
            "audio_quality",
            &["Исходное", "Экономия трафика"],
            u32::from(read_pref("audio_quality").as_str() == Some("mobile")),
            |index| json!(["original", "mobile"][index as usize]),
            |_| {},
        );
        self.choice(
            &stream,
            "Буферизация",
            "Размер буфера со следующего трека.",
            "audio_buffer",
            &["Стандартная", "Быстрый запуск", "Нестабильная сеть"],
            pref_index("audio_buffer", 0, 3),
            |index| json!(index),
            |_| {},
        );
        self.reset_section(
            page,
            "Сбросить параметры воспроизведения",
            &[
                "autoplay",
                "restore_playback",
                "restore_track_position",
                "volume_curve_v2",
                "volume",
                "volume_limit",
                "audio_quality",
                "audio_buffer",
                "keep_paused_on_skip",
                "prevent_queue_duplicates",
                "seek_step",
            ],
            |u| {
                u.autoplay.set(true);
                u.apply_volume_limit();
                u.apply_seek_step();
                u.volume_control.set_value(20.);
            },
        );
    }

    pub(super) fn apply_seek_step(&self) {
        let seconds = [1., 5., 10., 15., 30.][pref_index("seek_step", 0, 5) as usize];
        self.seek.adjustment().set_step_increment(seconds);
    }

    pub(super) fn apply_volume_limit(self: &Rc<Self>) {
        let limit = [100., 85., 70., 50.][pref_index("volume_limit", 0, 4) as usize];
        let volume = self.volume_control.adjustment();
        volume.configure(
            volume.value(),
            volume.lower(),
            limit,
            volume.step_increment(),
            volume.page_increment(),
            volume.page_size(),
        );
    }

    fn library_settings(self: &Rc<Self>, page: &gtk::Box) {
        let library = group(page, "Представление");
        self.choice(
            &library,
            "Вид новых коллекций",
            "Применяется к альбомам, исполнителям и плейлистам без своего сохранённого вида.",
            "default_collection_view",
            &["Список", "Сетка"],
            default_collection_view(),
            |index| json!(index),
            |_| {},
        );
        self.toggle(
            &library,
            "Разрешить статьи на английском",
            "Если русскоязычная статья недоступна, HomeMusic может открыть английскую.",
            "wiki_english",
            false,
            |_, _| {},
        );
        let history = group(page, "История прослушиваний");
        self.toggle(
            &history,
            "Сохранять прослушивания на сервере",
            "Сервер использует новые прослушивания для истории и персональных рекомендаций.",
            "record_history",
            true,
            |u, active| u.settings.record_history.set(active),
        );
        let imports = group(page, "Перенос музыки");
        self.action(
            &imports,
            "Импорт из Spotify",
            "Перенести библиотеку и плейлисты.",
            "document-import-symbolic",
            Self::import_dialog,
        );
        self.action(
            &imports,
            "Статус импорта Spotify",
            "Проверить выполнение и результат.",
            "view-refresh-symbolic",
            Self::import_status,
        );
        self.reset_section(
            page,
            "Сбросить параметры библиотеки",
            &[
                "wiki_english",
                "list_sorts",
                "list_views",
                "default_collection_view",
                "record_history",
            ],
            |u| u.settings.record_history.set(true),
        );
    }

    fn download_settings(self: &Rc<Self>, page: &gtk::Box) {
        let files = group(page, "Файлы рядом с треком");
        for (title, explanation, key) in [
            (
                "Сохранять обложку",
                "Загружает изображение релиза, если оно найдено.",
                "download_artwork",
            ),
            (
                "Сохранять текст",
                "Скачивает доступный текст песни отдельным файлом.",
                "download_lyrics",
            ),
            (
                "Сохранять сведения",
                "Добавляет файл с информацией о треке.",
                "download_info",
            ),
        ] {
            self.toggle(&files, title, explanation, key, true, |_, _| {});
        }
        self.action(
            &files,
            "Открыть задачи загрузки",
            "Посмотреть состояние скачиваемых треков.",
            "folder-download-symbolic",
            |u| u.navigate("download_jobs", true),
        );
        self.reset_section(page, "Сбросить параметры загрузок", DOWNLOADS, |_| {});
    }

    fn navigation_settings(self: &Rc<Self>, page: &gtk::Box) {
        let home = group(page, "Разделы на Главной");
        self.toggle(
            &home,
            "Мои миксы дня",
            "Показывает сетку персональных миксов.",
            "home_show_mixes",
            true,
            |_, _| {},
        );
        self.toggle(
            &home,
            "Волны по вашим трекам",
            "Показывает рекомендации на основе треков.",
            "home_show_waves",
            true,
            |_, _| {},
        );
        self.choice(
            &home,
            "Столбцы миксов",
            "Максимум карточек в строке на Главной.",
            "home_grid_columns",
            &["2", "3", "4", "5", "6"],
            read_pref("home_grid_columns")
                .as_u64()
                .unwrap_or(6)
                .clamp(2, 6) as u32
                - 2,
            |index| json!(index + 2),
            |_| {},
        );
        let startup = group(page, "При запуске");
        let saved = read_pref("startup_page");
        self.choice(
            &startup,
            "Начальный раздел",
            "Куда перейти после подключения к HomeMusic.",
            "startup_page",
            START_LABELS,
            START_PAGES
                .iter()
                .position(|page| saved.as_str() == Some(page))
                .unwrap_or(0) as u32,
            |index| json!(START_PAGES[index as usize]),
            |_| {},
        );
        let current = match right_panel_startup() {
            Some(true) => 1,
            Some(false) => 2,
            None => 0,
        };
        self.choice(
            &startup,
            "Правая панель при запуске",
            "В автоматическом режиме она открывается в широком окне.",
            "right_panel_startup",
            &["Автоматически", "Показывать", "Скрывать"],
            current,
            |index| json!(["auto", "show", "hide"][index as usize]),
            |u| {
                u.show_details.set(right_panel_startup());
                u.update_panels();
            },
        );
        let search = group(page, "Поиск музыки");
        self.choice(
            &search,
            "Источник по умолчанию",
            "Какой источник открыть для поиска музыки.",
            "default_search_scope",
            SEARCH_SCOPES,
            default_search_scope(),
            |index| json!(index),
            |u| u.search_scope.set_selected(default_search_scope()),
        );
        self.choice(
            &search,
            "Тип результатов в каталоге",
            "",
            "default_search_kind",
            SEARCH_KINDS,
            default_search_kind(),
            |index| json!(index),
            |u| u.search_kind.set_selected(default_search_kind()),
        );
        self.reset_section(
            page,
            "Сбросить параметры навигации",
            &[
                "startup_page",
                "home_show_mixes",
                "home_show_waves",
                "home_grid_columns",
                "right_panel_startup",
                "default_search_scope",
                "default_search_kind",
            ],
            |u| {
                u.show_details.set(None);
                u.update_panels();
                u.search_scope.set_selected(0);
                u.search_kind.set_selected(0);
            },
        );
    }

    fn shortcut_settings(self: &Rc<Self>, page: &gtk::Box) {
        let help = label(
            "Выберите поле и нажмите новое сочетание. Tab перемещает фокус, Enter активирует кнопку.",
            "muted",
        );
        help.set_wrap(true);
        page.append(&help);
        let keyboard = group(page, "Клавиатура");
        let mut entries = Vec::new();
        for ((_, id, title, _), (key, mods)) in SHORTCUTS
            .iter()
            .zip(self.settings.shortcuts.borrow().iter().copied())
        {
            let input = gtk::Entry::new();
            input.set_widget_name(&format!("shortcut-{id}"));
            input.set_editable(false);
            input.set_width_chars(13);
            input.set_text(&gtk::accelerator_get_label(key, mods));
            let draft = Rc::new(Cell::new((key, mods)));
            let keys = gtk::EventControllerKey::new();
            let value = draft.clone();
            let field = input.downgrade();
            let u = Rc::downgrade(self);
            keys.set_propagation_phase(gtk::PropagationPhase::Capture);
            keys.connect_key_pressed(move |_, key, _, mods| {
                let binding = normalize(key, mods);
                if matches!(
                    key,
                    gdk::Key::Tab
                        | gdk::Key::ISO_Left_Tab
                        | gdk::Key::Return
                        | gdk::Key::KP_Enter
                        | gdk::Key::Escape
                ) {
                    return glib::Propagation::Proceed;
                }
                if allowed(binding) {
                    value.set(binding);
                    if let Some(field) = field.upgrade() {
                        field.set_text(&gtk::accelerator_get_label(binding.0, binding.1));
                    }
                } else if let Some(u) = u.upgrade() {
                    u.notice(
                        "Нажмите сочетание с Ctrl, Alt или Shift либо функциональную клавишу",
                        true,
                    );
                }
                glib::Propagation::Stop
            });
            input.add_controller(keys);
            keyboard.append(&row(title, "", &input));
            entries.push(draft);
        }
        let apply = button("Применить сочетания", None, "primary");
        apply.set_widget_name("shortcut-apply");
        apply.set_halign(gtk::Align::Start);
        let u = Rc::downgrade(self);
        apply.connect_clicked(move |_| {
            let Some(u) = u.upgrade() else { return };
            let bindings: Vec<_> = entries.iter().map(|draft| draft.get()).collect();
            if bindings_conflict(&bindings) {
                u.notice("Сочетание уже назначено другому действию", true);
                return;
            }
            let values: serde_json::Map<_, _> = SHORTCUTS
                .iter()
                .zip(&bindings)
                .map(|((_, id, _, _), (key, mods))| {
                    (
                        (*id).into(),
                        json!(gtk::accelerator_name(*key, *mods).as_str()),
                    )
                })
                .collect();
            if u.save_preferences(&[("shortcuts", Value::Object(values))], &[]) {
                *u.settings.shortcuts.borrow_mut() = bindings;
                u.notice("Сочетания сохранены", false);
            }
        });
        page.append(&apply);
        self.reset_section(
            page,
            "Сбросить сочетания",
            &["shortcuts"],
            |u| {
                *u.settings.shortcuts.borrow_mut() = SHORTCUTS
                    .iter()
                    .map(|(_, _, _, key)| gtk::accelerator_parse(*key).unwrap())
                    .collect();
            },
        );
    }

    fn connection_settings(self: &Rc<Self>, page: &gtk::Box) {
        let backend = group(page, "Сервер HomeMusic");
        let state = label(self.settings.connection.get().description(), "");
        state.set_widget_name("connection-status");
        state.set_wrap(true);
        self.settings.connection_label.set(Some(&state));
        let retry = button(
            "Повторить подключение",
            Some("view-refresh-symbolic"),
            "tonal",
        );
        retry.set_widget_name("connection-retry");
        retry.set_sensitive(!matches!(
            self.settings.connection.get(),
            Connection::Checking
        ));
        let u = Rc::downgrade(self);
        retry.connect_clicked(move |_| {
            if let Some(u) = u.upgrade() {
                u.connect_backend();
            }
        });
        self.settings.retry_button.set(Some(&retry));
        backend.append(&row("Статус подключения", "", &state));
        backend.append(&row("Проверка соединения", "", &retry));
        let copy = button("Копировать диагностику", None, "tonal");
        let u = Rc::downgrade(self);
        copy.connect_clicked(move |_| {
            if let Some(u) = u.upgrade() {
                u.window.clipboard().set_text(&format!(
                    "HomeMusic Native {}\nBackend: localhost\n{}",
                    env!("CARGO_PKG_VERSION"),
                    u.settings.connection.get().description()
                ));
                u.notice("Диагностика скопирована", false);
            }
        });
        backend.append(&row(
            "Диагностика",
            "Версия приложения и состояние подключения.",
            &copy,
        ));
        let network = group(page, "Сеть");
        let proxy = gtk::Entry::new();
        proxy.set_width_chars(18);
        proxy.set_placeholder_text(Some("http://127.0.0.1:2080"));
        let saved_proxy = read_pref("proxy");
        proxy.set_text(saved_proxy.as_str().unwrap_or_default());
        network.append(&row("Прокси", "Применяется после перезапуска.", &proxy));
        let save = button("Сохранить прокси", None, "tonal");
        save.set_halign(gtk::Align::Start);
        let u = Rc::downgrade(self);
        save.connect_clicked(move |_| {
            let Some(u) = u.upgrade() else { return };
            let value = proxy.text();
            let value = value.trim();
            let saved = if value.is_empty() {
                u.save_preferences(&[], &["proxy"])
            } else if ureq::Proxy::new(value).is_ok() {
                u.save_preferences(&[("proxy", json!(value))], &[])
            } else {
                u.notice("Укажите корректный URL прокси", true);
                return;
            };
            if saved {
                u.notice("Прокси сохранён · применяется после перезапуска", false);
            }
        });
        page.append(&save);
        self.reset_section(page, "Сбросить адрес прокси", &["proxy"], |_| {});
    }

    pub(super) fn remember_startup_page(&self, page: &str) {
        if START_PAGES[..5].contains(&page) && self.settings.last_page.borrow().as_str() != page {
            self.settings.last_page.replace(page.to_string());
        }
    }

    pub(super) fn save_startup_page(&self) {
        self.save_preferences(
            &[("last_page", json!(*self.settings.last_page.borrow()))],
            &[],
        );
    }

    pub(super) fn backend_ready(&self) -> bool {
        matches!(self.settings.connection.get(), Connection::Connected)
    }

    pub(super) fn connect_backend(self: &Rc<Self>) {
        if matches!(self.settings.connection.get(), Connection::Checking) {
            return;
        }
        self.settings.set_connection(Connection::Checking);
        let api = self.api.clone();
        let tx = self.tx.clone();
        if !self.net.run(move || {
            let result = api.login();
            if result.is_ok() {
                let stars = api.get_with_timeout("/api/starred", Duration::from_secs(3));
                let _ = tx.send_blocking(Event::Stars(stars));
            }
            let _ = tx.send_blocking(Event::Login(result));
        }) {
            self.settings.set_connection(Connection::Disconnected);
            self.notice("Не удалось начать подключение · повторите попытку", true);
        }
    }

    pub(super) fn connection_result(self: &Rc<Self>, result: Result<(), String>) {
        if let Err(error) = result {
            self.settings.set_connection(Connection::Disconnected);
            self.notice(&error, true);
            return;
        }
        self.settings.set_connection(Connection::Connected);
        self.notice("", false);
        self.load_sidebar();
        let first = !self.settings.authenticated.replace(true);
        if !first {
            self.invalidate_collection("likes");
            self.invalidate_collection("downloads");
        }
        self.restore_collections();
        let page = self.page.borrow().clone();
        if page.is_empty() {
            self.navigate(&startup_page(), true);
        } else if !matches!(page.as_str(), "settings" | "search") && !page.starts_with("search:") {
            self.navigate(&page, true);
        }
        self.load_collection("likes", false);
        self.load_collection("downloads", false);
        if first && read_pref("restore_playback").as_bool().unwrap_or(true) {
            self.restore_playback();
        }
        let api = self.api.clone();
        let tx = self.tx.clone();
        self.net.run(move || {
            let _ = tx.send_blocking(Event::Recent(api.get("/api/plays/recent?limit=100")));
        });
    }

    pub(super) fn handle_shortcut(
        self: &Rc<Self>,
        key: gdk::Key,
        mods: gdk::ModifierType,
    ) -> glib::Propagation {
        let focus = gtk::prelude::RootExt::focus(&self.window);
        if focus.as_ref().is_some_and(|focus| {
            focus.is::<gtk::Editable>()
                || focus.is::<gtk::TextView>()
                || focus.is_ancestor(&self.search)
                || focus == &self.search.clone().upcast::<gtk::Widget>()
                || (key == gdk::Key::space
                    && (focus.is::<gtk::Button>() || focus.is::<gtk::Switch>()))
        }) {
            return glib::Propagation::Proceed;
        }
        let binding = normalize(key, mods);
        let action = self
            .settings
            .shortcuts
            .borrow()
            .iter()
            .position(|candidate| *candidate == binding);
        let Some(action) = action else {
            return glib::Propagation::Proceed;
        };
        match SHORTCUTS[action].0 {
            Shortcut::Search => {
                self.search.grab_focus();
            }
            Shortcut::Settings => self.preferences(),
            Shortcut::Details => {
                let track = self.current.borrow().clone();
                if let Some(track) = track {
                    self.open_details(track);
                }
            }
            Shortcut::Play => self.toggle_play(),
            Shortcut::Next => self.next(false),
            Shortcut::Restart => {
                self.seek_to(0.);
            }
            Shortcut::Queue => self.show_queue(),
            Shortcut::Back => self.history_step(-1),
            Shortcut::Forward => self.history_step(1),
        }
        glib::Propagation::Stop
    }

    fn reset_section(
        self: &Rc<Self>,
        page: &gtk::Box,
        title: &str,
        keys: &'static [&'static str],
        apply: impl Fn(&Rc<Self>) + 'static,
    ) {
        let reset = button(title, None, "tonal");
        reset.set_widget_name("settings-reset");
        reset.set_halign(gtk::Align::Start);
        let u = Rc::downgrade(self);
        reset.connect_clicked(move |_| {
            if let Some(u) = u.upgrade() {
                if u.save_preferences(&[], keys) {
                    apply(&u);
                    u.show_preferences();
                    if let Some(reset) =
                        find_widget(&u.settings_view.clone().upcast(), "settings-reset")
                    {
                        reset.grab_focus();
                    }
                    u.notice("Настройки раздела сброшены", false);
                }
            }
        });
        page.append(&reset);
    }

    fn toggle(
        self: &Rc<Self>,
        page: &gtk::Box,
        title: &str,
        description: &str,
        key: &'static str,
        fallback: bool,
        apply: impl Fn(&Rc<Self>, bool) + 'static,
    ) {
        let toggle = gtk::Switch::new();
        toggle.set_widget_name(&format!("pref-{key}"));
        toggle.set_active(pref_bool(key, fallback));
        toggle.set_valign(gtk::Align::Center);
        let u = Rc::downgrade(self);
        let saved = Cell::new(toggle.is_active());
        toggle.connect_active_notify(move |toggle| {
            let active = toggle.is_active();
            if active == saved.get() {
                return;
            }
            if let Some(u) = u.upgrade()
                && u.save_preferences(&[(key, json!(active))], &[])
            {
                saved.set(active);
                apply(&u, active);
                return;
            }
            toggle.set_active(saved.get());
        });
        page.append(&row(title, description, &toggle));
    }

    fn choice(
        self: &Rc<Self>,
        page: &gtk::Box,
        title: &str,
        description: &str,
        key: &'static str,
        options: &[&str],
        initial: u32,
        value: impl Fn(u32) -> Value + 'static,
        apply: impl Fn(&Rc<Self>) + 'static,
    ) -> gtk::DropDown {
        let control = ripple::dropdown(options);
        control.set_widget_name(&format!("pref-{key}"));
        control.set_selected(initial);
        let saved = Cell::new(initial);
        let u = Rc::downgrade(self);
        control.connect_selected_notify(move |control| {
            let state = control.selected();
            if state == saved.get() || state == gtk::INVALID_LIST_POSITION {
                return;
            }
            if let Some(u) = u.upgrade() {
                if u.save_preferences(&[(key, value(state))], &[]) {
                    saved.set(state);
                    apply(&u);
                    return;
                }
            }
            control.set_selected(saved.get());
        });
        page.append(&row(title, description, &control));
        control
    }

    fn action(
        self: &Rc<Self>,
        page: &gtk::Box,
        title: &str,
        description: &str,
        icon: &str,
        apply: impl Fn(&Rc<Self>) + 'static,
    ) {
        let action = icon_button(icon, title, "tonal");
        let u = Rc::downgrade(self);
        action.connect_clicked(move |_| {
            if let Some(u) = u.upgrade() {
                apply(&u);
            }
        });
        page.append(&row(title, description, &action));
    }
}

fn progress_style(settings: &Value) -> u32 {
    settings["progress_style"]
        .as_u64()
        .filter(|style| *style <= 3)
        .map(|style| style as u32)
        .unwrap_or_else(|| {
            if settings["slider"].as_u64() == Some(1) {
                3
            } else {
                0
            }
        })
}

#[derive(Clone, Copy)]
pub(super) struct LyricsAppearance {
    size: u64,
    spacing: u64,
    alignment: u64,
    active_alignment: Option<u64>,
    agents: bool,
    neutral: bool,
    pub(super) soft_focus: bool,
    focus: bool,
    pub(super) animation: u64,
    pub(super) real_timing: bool,
    pub(super) hide_status: bool,
    pub(super) romanize: bool,
    pub(super) romanize_main: bool,
    pub(super) romanize_script: usize,
    pub(super) subtitle_size: u64,
    pub(super) follow: bool,
    pub(super) seek: bool,
    pub(super) offset_seconds: f64,
}

impl LyricsAppearance {
    pub(super) fn current() -> Self {
        let value = Value::Object(crate::config::read_settings().unwrap_or_default());
        Self {
            size: value["lyrics_size"].as_u64().unwrap_or(1),
            spacing: value["lyrics_spacing"].as_u64().unwrap_or(1),
            alignment: value["lyrics_alignment"].as_u64().unwrap_or_default(),
            active_alignment: value["lyrics_active_alignment"]
                .as_u64()
                .and_then(|v| v.checked_sub(1)),
            agents: value["lyrics_agents"].as_bool().unwrap_or(true),
            neutral: value["lyrics_neutral"].as_bool().unwrap_or(false),
            soft_focus: value["lyrics_soft_focus"].as_bool().unwrap_or(false),
            animation: value["lyrics_animation"].as_u64().unwrap_or(2).min(2),
            real_timing: value["lyrics_real_timing"].as_bool().unwrap_or(true),
            hide_status: value["lyrics_hide_status"].as_bool().unwrap_or(false),
            romanize: value["lyrics_romanize"].as_bool().unwrap_or(false),
            romanize_main: value["lyrics_romanize_main"].as_bool().unwrap_or(false),
            romanize_script: value["lyrics_romanize_script"].as_u64().unwrap_or(0).min(7) as usize,
            subtitle_size: value["lyrics_subtitle_size"].as_u64().unwrap_or(1),
            focus: value["lyrics_focus"].as_bool().unwrap_or(true),
            follow: value["lyrics_follow"].as_bool().unwrap_or(true),
            seek: value["lyrics_seek"].as_bool().unwrap_or(true),
            offset_seconds: value["lyrics_offset"]
                .as_f64()
                .unwrap_or(0.)
                .clamp(-30., 30.),
        }
    }

    pub(super) fn decorate(&self, parent: &impl IsA<gtk::Widget>) {
        let parent = parent.as_ref();
        for (class, active) in [
            ("lyrics-small", self.size == 0),
            ("lyrics-large", self.size == 2),
            ("lyrics-compact", self.spacing == 0),
            ("lyrics-airy", self.spacing == 2),
            ("lyrics-focus", self.focus),
            ("lyrics-neutral", self.neutral),
            ("lyrics-soft-focus", self.soft_focus),
            ("lyrics-subtitle-small", self.subtitle_size == 0),
            ("lyrics-subtitle-large", self.subtitle_size == 2),
            ("karaoke-sample", self.animation != 0),
            ("karaoke-smooth-sample", self.animation == 2),
        ] {
            if active {
                parent.add_css_class(class);
            } else {
                parent.remove_css_class(class);
            }
        }
    }

    pub(super) fn spacing(&self) -> i32 {
        match self.spacing {
            0 => 4,
            2 => 20,
            _ => 12,
        }
    }

    pub(super) fn format_label(&self, label: &gtk::Label) {
        self.format_line(label, false, false);
    }

    pub(super) fn format_line(&self, label: &gtk::Label, active: bool, alternate: bool) {
        let side = if alternate && self.agents {
            2
        } else if active {
            self.active_alignment.unwrap_or(self.alignment)
        } else {
            self.alignment
        };
        let (alignment, justification) = match side {
            1 => (0.5, gtk::Justification::Center),
            2 => (1., gtk::Justification::Right),
            _ => (0., gtk::Justification::Left),
        };
        label.set_xalign(alignment);
        label.set_justify(justification);
    }
}

fn update_lyrics_preview(preview: &gtk::Box) {
    let appearance = LyricsAppearance::current();
    appearance.decorate(preview);
    preview.set_spacing(appearance.spacing());
    let mut child = preview.first_child();
    while let Some(line) = child.and_downcast::<gtk::Label>() {
        appearance.format_line(&line, line.has_css_class("karaoke-active"), false);
        child = line.next_sibling();
    }
}

fn group(page: &gtk::Box, title: &str) -> gtk::Box {
    let group = vbox(6);
    group.add_css_class("settings-group");
    group.append(&label(title, "settings-group-title"));
    let card = vbox(0);
    card.add_css_class("settings-card");
    group.append(&card);
    page.append(&group);
    card
}

fn row(title: &str, description: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = hbox(12);
    row.add_css_class("settings-row");
    let copy = vbox(3);
    copy.set_hexpand(true);
    let name = label(title, "settings-row-title");
    name.set_wrap(true);
    copy.append(&name);
    if !description.is_empty() {
        let detail = label(description, "small muted");
        detail.set_wrap(true);
        copy.append(&detail);
        control.as_ref().update_property(&[
            gtk::accessible::Property::Label(title),
            gtk::accessible::Property::Description(description),
        ]);
    } else if !control.as_ref().is::<gtk::Label>() {
        control
            .as_ref()
            .update_property(&[gtk::accessible::Property::Label(title)]);
    }
    control.as_ref().set_valign(gtk::Align::Center);
    row.append(&copy);
    row.append(control);
    row
}

fn append_preview(group: &gtk::Box, title: &str, content: &impl IsA<gtk::Widget>) {
    let row = vbox(8);
    row.add_css_class("settings-row");
    row.add_css_class("settings-preview-row");
    row.append(&label(title, "settings-row-title"));
    row.append(content);
    group.append(&row);
}

fn clear_settings_body(page: &gtk::Box) {
    while let Some(child) = page.first_child() {
        page.remove(&child);
    }
}

fn filter_settings(page: &gtk::Box, section: &str, terms: &[&str]) -> bool {
    let mut found = false;
    let mut child = page.first_child();
    while let Some(group) = child {
        child = group.next_sibling();
        if !group.has_css_class("settings-group") {
            continue;
        }
        let title = group
            .first_child()
            .and_downcast::<gtk::Label>()
            .unwrap()
            .text();
        let group_terms = format!("{section} {}", title.to_lowercase());
        let mut group_found = false;
        let mut previews = Vec::new();
        let mut row = group.last_child().and_then(|card| card.first_child());
        while let Some(item) = row {
            row = item.next_sibling();
            let mut searchable = group_terms.clone();
            let mut copy = item.first_child().and_then(|copy| copy.first_child());
            while let Some(line) = copy.and_downcast::<gtk::Label>() {
                searchable.push(' ');
                searchable.push_str(&line.text().to_lowercase());
                copy = line.next_sibling();
            }
            let matches = terms.iter().all(|term| searchable.contains(term));
            if item.has_css_class("settings-preview-row") {
                previews.push(item.clone());
            }
            item.set_visible(matches);
            group_found |= matches;
        }
        for preview in previews {
            preview.set_visible(group_found);
        }
        group.set_visible(group_found);
        found |= group_found;
    }
    found
}

pub(super) fn pref_bool(key: &str, fallback: bool) -> bool {
    read_pref(key).as_bool().unwrap_or(fallback)
}

fn pref_index(key: &str, fallback: u32, count: u64) -> u32 {
    read_pref(key)
        .as_u64()
        .filter(|index| *index < count)
        .map(|index| index as u32)
        .unwrap_or(fallback)
}

fn normalize(key: gdk::Key, mods: gdk::ModifierType) -> Binding {
    (
        key.to_lower(),
        mods & gtk::accelerator_get_default_mod_mask(),
    )
}

fn allowed((key, mods): Binding) -> bool {
    if matches!(
        key,
        gdk::Key::Tab
            | gdk::Key::ISO_Left_Tab
            | gdk::Key::Return
            | gdk::Key::KP_Enter
            | gdk::Key::Escape
    ) {
        return false;
    }
    (key == gdk::Key::space || (gdk::Key::F1..=gdk::Key::F12).contains(&key) || !mods.is_empty())
        && gtk::accelerator_valid(key, mods)
}

fn bindings_conflict(bindings: &[Binding]) -> bool {
    let mut seen = HashSet::new();
    bindings.iter().any(|binding| !seen.insert(*binding))
}

fn find_widget(widget: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    if widget.widget_name() == name && widget.is_mapped() {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(found) = find_widget(&widget, name) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

pub(super) fn test_widget(name: &str) -> Option<gtk::Widget> {
    gtk::Window::list_toplevels()
        .iter()
        .find_map(|widget| find_widget(widget, name))
}
