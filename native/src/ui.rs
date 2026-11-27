mod artist;
mod artwork;
mod autoscroll;
mod collection_disk;
mod cover_viewer;
mod details;
mod downloads;
mod imports;
mod layout;
mod lyric_display;
mod lyric_sources;
mod lyric_translation;
mod material;
mod motion;
mod mpris;
mod pins;
mod playlists;
mod queue;
mod ripple;
mod search;
mod settings;
mod timeline;
mod tray;
mod sources;
mod ux_probe;
mod workspace;
use crate::{
    api::{self, Api, Track},
    audio::Player,
    theme,
};
use gtk::{gdk, gio, glib, prelude::*};
use search::set_metadata_line;
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

type Job = Box<dyn FnOnce() + Send>;
#[derive(Clone)]
struct Network(mpsc::SyncSender<Job>);
impl Network {
    fn new(n: usize) -> Self {
        Self::with_capacity(n, 256)
    }
    fn with_capacity(n: usize, queued: usize) -> Self {
        let (tx, rx) = mpsc::sync_channel::<Job>(queued);
        let rx = Arc::new(Mutex::new(rx));
        for _ in 0..n {
            let rx = rx.clone();
            std::thread::spawn(move || {
                loop {
                    let job = { rx.lock().unwrap().recv() };
                    match job {
                        Ok(j) => j(),
                        Err(_) => break,
                    }
                }
            });
        }
        Self(tx)
    }
    fn run(&self, f: impl FnOnce() + Send + 'static) -> bool {
        self.0.try_send(Box::new(f)).is_ok()
    }
}
#[derive(Clone)]
enum Item {
    Track(Track),
    Card {
        title: String,
        sub: String,
        path: String,
        cover: String,
    },
}
#[derive(Default)]
struct CollectionCache {
    value: Option<Value>,
    received: Option<Instant>,
    fetching: bool,
    revision: u64,
}

enum Event {
    Login(Result<(), String>),
    Stars(Result<Value, String>),
    Favorite(String, bool, Result<Value, String>),
    Page(u64, String, Result<Value, String>),
    CollectionCache(&'static str, u64, Result<Value, String>),
    EnrichedPage(u64, String, Value, Value),
    Resolved(u64, Result<Value, String>),
    NowLyrics(u64, Vec<details::LyricLine>),
    Notice(Result<Value, String>, String),
    Cover(String, i32, Result<Vec<u8>, String>),
    Collection(u64, Result<Value, String>),
    AlbumMetadata(u64, Result<Value, String>),
    ArtistMetadata(u64, Result<String, String>),
    Import(Result<Value, String>),
    Sidebar(Result<Value, String>),
    PersonalMixes(Result<Value, String>),
    Continuation(u64, String, Result<Value, String>),
    PlaylistChanged(String, Result<Value, String>, bool),
    Recent(Result<Value, String>),
    MixArtwork(Vec<Track>),
    More(u64, Result<Value, String>),
}
struct Ui {
    window: gtk::ApplicationWindow,
    api: Api,
    net: Network,
    navigation: Network,
    enrichment: Network,
    collection_writer: Network,
    collection_cache: RefCell<HashMap<&'static str, CollectionCache>>,
    home_mix_art: RefCell<Value>,
    images: Network,
    tx: async_channel::Sender<Event>,
    model: gio::ListStore,
    items: RefCell<Vec<Item>>,
    source_items: RefCell<Vec<Item>>,
    list_filter: gtk::SearchEntry,
    collection_view: gtk::DropDown,
    list_adjustment: gtk::Adjustment,
    list_anchors: RefCell<HashMap<String, f64>>,
    restoring_list: Cell<bool>,
    list_update_ms: Cell<f64>,
    personal_refresh_due: Cell<Option<u64>>,
    list: gtk::ListView,
    title: gtk::Label,
    brand_mark: gtk::Label,
    nav_buttons: Vec<(gtk::ToggleButton, String)>,
    switching_nav: Cell<bool>,

    panel_revealer: gtk::Revealer,
    show_details: Cell<Option<bool>>,
    panel_button: gtk::Button,
    sidebar_widget: gtk::Box,
    right_widget: gtk::Box,
    sidebar_width: Cell<i32>,
    right_width: Cell<i32>,
    sidebar_drag: gtk::GestureDrag,
    left_resize_handle: gtk::Box,
    right_resize_handle: gtk::Box,
    right_drag: gtk::GestureDrag,
    top_bar: gtk::CenterBox,
    player_bar: gtk::Box,
    volume_control: gtk::Scale,
    wave_frame: Cell<Option<i64>>,
    more_button: gtk::Button,
    more_offset: Cell<Option<u64>>,
    subtitle: gtk::Label,
    status: gtk::Label,
    hero: gtk::Box,
    empty: gtk::Box,
    search: gtk::SearchEntry,
    search_timer: RefCell<Option<glib::SourceId>>,
    search_scope: gtk::DropDown,
    search_kind: gtk::DropDown,
    search_filters: gtk::Box,
    request: Cell<u64>,
    resolve: Cell<u64>,
    page: RefCell<String>,
    cache: RefCell<VecDeque<(String, Value)>>,
    stars: RefCell<HashSet<String>>,
    stars_loaded: Cell<bool>,
    favorites_snapshot: RefCell<Value>,
    favorite_pending: RefCell<HashSet<String>>,
    personal_mixes: RefCell<Value>,
    player: Player,
    queue: RefCell<Vec<Track>>,
    index: Cell<usize>,
    user_queue_end: Cell<usize>,
    restore_position: Cell<Option<f64>>,
    resolving_pause: Cell<Option<bool>>,
    refill_after: Cell<Instant>,
    wave_waiting: Cell<bool>,
    wave_start_request: Cell<Option<u64>>,
    current: RefCell<Option<Track>>,
    wave: RefCell<Option<String>>,
    playing: Cell<bool>,
    started: Cell<bool>,
    playback: RefCell<String>,
    listened: Cell<f64>,
    last_tick: Cell<Instant>,
    last_progress: Cell<u64>,
    play_button: gtk::Button,
    lyrics_button: gtk::Button,
    now_lyrics: lyric_display::NowLyrics,
    seek: gtk::Scale,
    position: gtk::Label,
    duration: gtk::Label,
    timeline: timeline::Timeline,
    now_title: gtk::Label,
    now_artist: gtk::Label,
    now_album: gtk::Label,
    source_label: gtk::Label,
    personal_sidebar: gtk::Box,
    mix_pictures: Vec<gtk::Picture>,
    album_artist: RefCell<Option<Track>>,
    now_picture: gtk::Picture,
    now_symbol: gtk::Label,
    mini_title: gtk::Label,
    mini_artist: gtk::Label,
    mini_album: gtk::Label,
    mini_picture: gtk::Picture,
    like_button: gtk::Button,
    cover_cache: RefCell<VecDeque<(String, gdk::Texture)>>,
    cover_pending: RefCell<HashSet<String>>,
    cover_flush_scheduled: Cell<bool>,
    wanted_covers: Arc<Mutex<HashSet<String>>>,
    deferred_covers: RefCell<HashMap<usize, (glib::WeakRef<gtk::Picture>, String)>>,
    cover_waiters: RefCell<HashMap<String, Vec<glib::WeakRef<gtk::Picture>>>>,
    cover_bindings: RefCell<HashMap<usize, (glib::WeakRef<gtk::Picture>, String)>>,
    cover_retries: RefCell<HashMap<String, u8>>,
    mpris: RefCell<Option<mpris::Mpris>>,
    _theme: gtk::CssProvider,
    _monitor: Option<gio::FileMonitor>,
    test: bool,
    pins: RefCell<HashMap<String, pins::Pin>>,
    active_rows: RefCell<Vec<glib::WeakRef<gtk::ListItem>>>,
    row_state: RefCell<String>,
    played_history: RefCell<VecDeque<Track>>,
    history: RefCell<Vec<String>>,
    history_index: Cell<usize>,
    history_move: Cell<bool>,
    shelves: gtk::Box,
    sidebar_items: gtk::Box,
    page_cover: gtk::Picture,
    page_art: RefCell<String>,
    home_scroll: gtk::ScrolledWindow,
    page_tools: gtk::Box,
    collection_play: gtk::Button,
    collection_shuffle: gtk::Button,
    playlist_manage: gtk::Button,
    album_article: gtk::Button,
    downloads_jobs: gtk::Button,
    playlist_create: gtk::Button,
    sort: gtk::DropDown,
    album_info: RefCell<Option<(String, String)>>,
    content_list: gtk::Overlay,
    repeat: Cell<u8>,
    shuffled: Cell<bool>,
    queue_original: RefCell<Vec<Track>>,
    motion_mode: Cell<u32>,
    repeat_button: gtk::Button,
    shuffle_button: gtk::ToggleButton,
    mini_like: gtk::Button,
    preview_wave: RefCell<Option<String>>,
    queue_panel: gtk::Box,
    queue_model: RefCell<Option<gio::ListStore>>,
    history_panel: gtk::Box,
    panel_stack: gtk::Stack,
    panel_tabs: Vec<(gtk::ToggleButton, String)>,
    sidebar_revealer: gtk::Revealer,
    library_toggle: gtk::ToggleButton,
    sidebar_requested: Cell<bool>,
    updating_panels: Cell<bool>,
    sidebar_scroll: gtk::ScrolledWindow,

    settings_view: gtk::ScrolledWindow,
    settings: settings::Settings,
    artist_view: gtk::ScrolledWindow,
    artist_body: gtk::Box,
    artist_covers: RefCell<Vec<(glib::WeakRef<gtk::Picture>, String)>>,
    autoplay: Cell<bool>,
    autoplay_pending: Cell<bool>,
    continuation_generation: Cell<u64>,
    continuation_restarted: Cell<bool>,
}
pub fn build(app: &gtk::Application, present: bool) {
    if let Some(window) = app.active_window().or_else(|| app.windows().into_iter().next()) {
        if present {
            window.present();
        }
        return;
    }
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_theme_name(Some("Adwaita"));
    }
    let (provider, monitor) = theme::install();
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title(if std::env::var_os("HOMEMUSIC_TEST").is_some() {
            "HomeMusic · проверка"
        } else {
            "HomeMusic"
        })
        .default_width(1260)
        .default_height(820)
        .build();
    window.set_size_request(800, 620);
    window.set_decorated(false);
    let root = vbox(0);
    root.add_css_class("app-root");
    let top = hbox(8);
    top.add_css_class("nav-controls");
    let bar = gtk::CenterBox::new();
    bar.add_css_class("topbar");
    bar.set_start_widget(Some(&top));
    let handle = gtk::WindowHandle::new();
    handle.set_child(Some(&bar));
    root.append(&handle);
    let library_toggle = gtk::ToggleButton::new();
    library_toggle.set_child(Some(&symbol("view-list-symbolic")));
    library_toggle.add_css_class("library-toggle");
    library_toggle.set_widget_name("library-toggle");
    library_toggle.update_property(&[gtk::accessible::Property::Label("Медиатека")]);
    library_toggle.set_tooltip_text(Some("Медиатека"));
    library_toggle.set_active(true);
    top.append(&library_toggle);
    let back = icon_button("go-previous-symbolic", "Назад · Alt+←", "flat");
    let forward = icon_button("go-next-symbolic", "Вперёд · Alt+→", "flat");
    let home = icon_button("go-home-symbolic", "Главная", "tonal");
    top.append(&back);
    top.append(&forward);
    home.set_widget_name("home-button");
    let body = hbox(0);
    body.set_vexpand(true);
    root.append(&body);
    window.set_child(Some(&ripple::wrap(&root)));
    let sidebar = vbox(6);
    sidebar.add_css_class("sidebar");
    let sidebar_width = read_pref("sidebar_width")
        .as_i64()
        .unwrap_or(240)
        .clamp(200, 480) as i32;
    sidebar.set_width_request(sidebar_width);
    sidebar.set_hexpand(false);
    let sidebar_revealer = gtk::Revealer::new();
    sidebar_revealer.add_css_class("left-panel");
    let (sidebar_surface, sidebar_handle, sidebar_drag) = layout::resizable(&sidebar, true);
    sidebar_revealer.set_child(Some(&sidebar_surface));
    sidebar_revealer.set_reveal_child(true);
    sidebar_revealer.set_hexpand(false);
    body.append(&sidebar_revealer);
    let brand = hbox(9);
    brand.set_margin_bottom(10);
    let brand_mark = label("◉", "brand-mark");
    brand_mark.set_visible(false);
    brand.append(&brand_mark);
    brand.set_margin_start(20);
    brand.append(&label("Моя медиатека", "section-title"));
    sidebar.append(&brand);
    let create = icon_button("list-add-symbolic", "Создать плейлист", "flat");
    create.set_visible(false);
    create.set_widget_name("create-playlist");
    let mut navs = Vec::new();
    let mut first: Option<gtk::ToggleButton> = None;
    for (title, icon, key) in [
        ("Любимые треки", "emblem-favorite-symbolic", "likes"),
        ("Исполнители", "avatar-default-symbolic", "artists"),
        ("Плейлисты", "view-list-symbolic", "playlists"),
        ("Скачанное", "folder-download-symbolic", "downloads"),
    ] {
        let button = gtk::ToggleButton::new();
        button.add_css_class("nav");
        button.set_halign(gtk::Align::Fill);
        let row = hbox(12);
        row.append(&symbol(icon));
        row.append(&label(title, ""));
        button.set_child(Some(&row));
        if let Some(f) = &first {
            button.set_group(Some(f));
        } else {
            first = Some(button.clone());
        }
        sidebar.append(&button);
        navs.push((button, key.to_string()));
    }
    let sidebar_items = vbox(4);
    sidebar_items.set_margin_start(8);
    sidebar_items.set_margin_end(8);
    let sidebar_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&sidebar_items)
        .build();
    sidebar.append(&sidebar_scroll);

    let preferences = button("Настройки", Some("emblem-system-symbolic"), "flat");

    let spacer = vbox(0);
    spacer.set_vexpand(false);
    sidebar.append(&spacer);
    let import_button = button(
        "Импорт из Spotify",
        Some("document-import-symbolic"),
        "tonal",
    );
    import_button.set_visible(false);

    let center = vbox(12);
    center.add_css_class("content");
    center.set_hexpand(true);
    body.append(&center);

    let search = gtk::SearchEntry::new();
    search.add_css_class("search");
    search.set_placeholder_text(Some("Что хотите послушать?"));
    search.set_hexpand(false);
    search.set_width_request(480);
    let search_group = hbox(8);
    search_group.set_widget_name("home-search-group");
    search_group.append(&home);
    search_group.append(&search);
    bar.set_center_widget(Some(&search_group));
    let right_controls = hbox(4);
    bar.set_end_widget(Some(&right_controls));
    let refresh = icon_button("view-refresh-symbolic", "Обновить", "flat");
    refresh.set_visible(false);
    let details_toggle = icon_button("view-reveal-symbolic", "Сейчас играет", "flat");
    let _ = &details_toggle;
    set_md_icon(&details_toggle, "window-close-symbolic");
    right_controls.append(&preferences);
    let minimize = icon_button("window-minimize-symbolic", "Свернуть", "flat");
    minimize.set_widget_name("window-minimize");
    right_controls.append(&minimize);
    tray::install(app, &window, &minimize);
    let close = icon_button("window-close-symbolic", "Закрыть", "flat");
    close.add_css_class("custom-window-close");
    right_controls.append(&close);
    let w = window.clone();
    close.connect_clicked(move |_| w.close());

    let page_header = hbox(20);
    page_header.add_css_class("page-header");
    page_header.set_widget_name("page-header");
    let page_cover = gtk::Picture::new();
    page_cover.set_can_shrink(true);
    page_cover.set_size_request(140, 140);
    page_cover.set_content_fit(gtk::ContentFit::Cover);
    page_cover.set_visible(false);
    let page_cover_frame = cover_frame(&page_cover, 140);
    page_cover.bind_property("visible", &page_cover_frame, "visible").sync_create().build();
    page_header.append(&page_cover_frame);
    let heading = vbox(8);
    heading.set_valign(gtk::Align::End);
    page_header.append(&heading);
    let title = label("Главная", "page-title");
    title.set_wrap(true);
    title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    title.set_max_width_chars(42);
    heading.set_hexpand(true);
    let subtitle = label("", "muted");
    heading.append(&title);
    heading.append(&subtitle);
    page_header.set_margin_start(24);
    page_header.set_margin_end(24);
    center.append(&page_header);
    let hero = hbox(18);
    hero.add_css_class("hero");
    let hero_text = vbox(7);
    hero_text.set_hexpand(true);

    hero_text.append(&label("Моя волна", "hero-title"));

    let wave_button = button(
        "Включить волну",
        Some("media-playback-start-symbolic"),
        "primary",
    );
    wave_button.set_halign(gtk::Align::Start);
    wave_button.set_margin_top(8);
    hero_text.append(&wave_button);
    hero.append(&hero_text);
    hero.append(&label("≋", "hero-symbol"));
    let shelves = vbox(14);
    shelves.set_margin_start(24);
    shelves.set_margin_end(24);
    shelves.append(&hero);
    let home_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&shelves)
        .build();
    center.append(&motion::surface(&home_scroll));
    let settings_view = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .visible(false)
        .build();
    center.append(&motion::surface(&settings_view));
    let artist_body = vbox(20);
    artist_body.set_margin_start(24);
    artist_body.set_margin_end(24);
    let artist_view = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&artist_body)
        .visible(false)
        .build();
    center.append(&motion::surface(&artist_view));
    let tools = hbox(8);
    let queue_button = button("Очередь", Some("view-list-symbolic"), "");
    let mix_button = button("Слушать", Some("media-playback-start-symbolic"), "primary");
    tools.append(&mix_button);
    queue_button.set_visible(false);
    let shuffle = button(
        "Перемешать",
        Some("media-playlist-shuffle-symbolic"),
        "flat",
    );
    tools.append(&shuffle);
    let sort = ripple::dropdown(&["Исходный порядок", "По названию", "По исполнителю"]);
    sort.set_tooltip_text(Some("Сортировка"));
    tools.append(&sort);
    let list_filter = gtk::SearchEntry::builder()
        .placeholder_text("Фильтр списка")
        .tooltip_text("Фильтр текущего списка")
        .build();
    list_filter.set_visible(false);
    let collection_view = ripple::dropdown(&["Карточки", "Список"]);
    collection_view.set_tooltip_text(Some("Вид раздела"));
    collection_view.set_visible(false);
    let list_options = hbox(8);
    list_options.set_halign(gtk::Align::Start);
    list_options.set_margin_start(24);
    list_options.set_margin_end(24);
    list_options.append(&list_filter);
    list_options.append(&collection_view);
    let search_filters = hbox(8);
    let search_scope = ripple::dropdown(settings::SEARCH_SCOPES);
    search_scope.set_selected(settings::default_search_scope());
    search_scope.set_tooltip_text(Some("Где искать"));
    let search_kind = ripple::dropdown(settings::SEARCH_KINDS);
    search_kind.set_selected(settings::default_search_kind());
    search_kind.set_tooltip_text(Some("Тип результатов"));
    search_filters.add_css_class("search-tabs");
    search_filters.set_margin_start(24);
    search_filters.set_margin_end(24);
    let mut source_group: Option<gtk::ToggleButton> = None;
    for (index, name) in settings::SEARCH_SCOPES.iter().enumerate() {
        let tab = gtk::ToggleButton::with_label(name);
        if let Some(first) = &source_group {
            tab.set_group(Some(first));
        } else {
            source_group = Some(tab.clone());
        }
        tab.set_active(index as u32 == search_scope.selected());
        let scope = search_scope.clone();
        tab.connect_toggled(move |button| {
            if button.is_active() {
                scope.set_selected(index as u32);
            }
        });
        let weak = tab.downgrade();
        search_scope.connect_selected_notify(move |scope| {
            if let Some(tab) = weak.upgrade()
                && scope.selected() == index as u32
            {
                tab.set_active(true);
            }
        });
        search_filters.append(&tab);
    }
    search_filters.set_visible(false);
    center.insert_child_after(&search_filters, Some(&page_header));
    let playlist_manage = icon_button("view-more-symbolic", "Управление плейлистом", "flat");
    playlist_manage.set_widget_name("playlist-manage");
    tools.append(&playlist_manage);
    let album_article = button("Об альбоме", Some("text-x-generic-symbolic"), "tonal");
    album_article.set_widget_name("album-article");
    tools.append(&album_article);
    let downloads_jobs = button("Задачи загрузки", None, "tonal");
    downloads_jobs.set_widget_name("download-jobs");
    tools.append(&downloads_jobs);
    tools.append(&create);
    let tools_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .child(&tools)
        .build();
    tools_scroll.set_margin_start(24);
    tools_scroll.set_margin_end(24);
    center.append(&tools_scroll);
    center.append(&list_options);
    let model = gio::ListStore::new::<glib::BoxedAnyObject>();
    let selection = gtk::SingleSelection::new(Some(model.clone()));
    selection.set_autoselect(false);
    selection.set_can_unselect(true);
    let list = gtk::ListView::new(Some(selection), None::<gtk::ListItemFactory>);
    list.set_margin_start(16);
    list.set_margin_end(16);
    list.set_single_click_activate(false);
    list.set_vexpand(true);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&list)
        .build();
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&scroll));
    overlay.set_vexpand(true);
    let empty = vbox(10);
    empty.set_halign(gtk::Align::Center);
    empty.set_valign(gtk::Align::Center);
    empty.append(&label("Нет треков", "empty-title"));
    let e = label("", "muted");
    e.set_justify(gtk::Justification::Center);
    empty.append(&e);
    overlay.add_overlay(&empty);
    center.append(&motion::surface(&overlay));
    let more_button = button("Показать ещё", None, "tonal");
    more_button.set_visible(false);
    more_button.set_halign(gtk::Align::Center);
    center.append(&more_button);
    let status = label("Подключаемся к вашему серверу…", "status");
    status.set_ellipsize(gtk::pango::EllipsizeMode::End);
    center.append(&status);
    let panel = vbox(15);
    panel.add_css_class("now-panel");
    let panel_revealer = gtk::Revealer::new();
    // Horizontal collapse must be configured even when initially hidden. The GTK
    // default vertical transition retains the child's full width when closed.
    panel_revealer.set_transition_type(gtk::RevealerTransitionType::SlideLeft);
    let (right_surface, right_handle, right_drag) = layout::resizable(&panel, false);
    panel_revealer.set_child(Some(&right_surface));
    let right_width = read_pref("right_panel_width")
        .as_i64()
        .unwrap_or(280)
        .clamp(240, 480) as i32;
    panel.set_width_request(right_width);
    panel.set_hexpand(false);
    panel_revealer.set_hexpand(false);
    body.append(&panel_revealer);
    let panel_head = hbox(8);
    let panel_stack = gtk::Stack::new();
    panel_stack.set_vexpand(true);
    panel_stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    panel_stack.set_transition_duration(250);
    let tabs = hbox(2);
    tabs.add_css_class("panel-tabs");
    tabs.set_hexpand(true);
    let mut panel_tabs = vec![];
    let mut first_tab: Option<gtk::ToggleButton> = None;
    for (name, title) in [
        ("now", "Сейчас играет"),
        ("queue", "Очередь"),
        ("history", "История"),
    ] {
        let b = gtk::ToggleButton::new();
        let l = label(title, "small");
        l.set_wrap(true);
        l.set_max_width_chars(8);
        l.set_justify(gtk::Justification::Center);
        l.set_xalign(0.5);
        b.set_child(Some(&l));
        b.set_hexpand(true);
        if let Some(first) = &first_tab {
            b.set_group(Some(first));
        } else {
            first_tab = Some(b.clone());
            b.set_active(true);
        }
        tabs.append(&b);
        panel_tabs.push((b, name));
    }
    panel_head.set_margin_start(20);
    panel_head.set_margin_end(20);
    panel_head.append(&tabs);
    details_toggle.set_visible(false);
    panel_head.append(&details_toggle);
    panel.append(&panel_head);
    let details_panel = vbox(14);
    details_panel.set_margin_start(20);
    details_panel.set_margin_end(20);
    let now_scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&details_panel)
        .build();
    panel_stack.add_titled(&now_scroll, Some("now"), "Сейчас играет");
    let queue_panel = vbox(8);

    queue_panel.set_vexpand(true);
    panel_stack.add_titled(&queue_panel, Some("queue"), "Очередь");
    let history_panel = vbox(8);
    panel_stack.add_titled(&history_panel, Some("history"), "История");
    panel.append(&panel_stack);
    let art = gtk::Overlay::new();
    art.add_css_class("art-panel");
    art.set_overflow(gtk::Overflow::Hidden);
    let now_symbol = label("♫", "art-symbol");
    now_symbol.set_height_request(180);
    art.set_child(Some(&now_symbol));
    let now_picture = gtk::Picture::new();
    now_picture.set_content_fit(gtk::ContentFit::Cover);
    now_picture.set_can_shrink(true);
    now_picture.set_size_request(180, 180);
    now_picture.set_visible(false);
    now_picture.add_css_class("cover");
    art.add_overlay(&now_picture);
    let art_frame = gtk::AspectFrame::new(0.5, 0.5, 1., false);
    art_frame.set_child(Some(&art));
    details_panel.append(&art_frame);
    let now_title = label("Ничего не играет", "now-title");
    now_title.set_wrap(true);
    now_title.set_max_width_chars(19);
    let title_line = hbox(8);
    now_title.set_hexpand(true);
    let like_button = icon_button("emblem-favorite-symbolic", "В любимое", "flat");
    like_button.set_sensitive(false);
    like_button.set_valign(gtk::Align::Center);
    title_line.append(&now_title);
    title_line.append(&like_button);
    details_panel.append(&title_line);
    let now_artist = label("", "muted");
    now_artist.set_wrap(true);
    now_artist.set_max_width_chars(24);
    details_panel.append(&now_artist);
    let now_album = label("", "small muted");
    now_album.set_wrap(true);
    now_album.set_max_width_chars(24);
    details_panel.append(&now_album);
    let now_lyrics = lyric_display::NowLyrics::new();
    details_panel.append(now_lyrics.widget());
    let source_label = label("", "small muted now-source");
    source_label.set_wrap(true);
    source_label.set_visible(false);
    let source_button = button("Источник аудио", Some("audio-volume-high-symbolic"), "now-action");
    let about_track = button("О треке", Some("text-x-generic-symbolic"), "now-action");
    let about_artist = button("Об исполнителе", Some("avatar-default-symbolic"), "now-action");
    for button in [&source_button, &about_track, &about_artist] {
        if let Some(row) = button.child() {
            row.set_halign(gtk::Align::Start);
        }
    }
    let now_actions = vbox(2);
    now_actions.add_css_class("now-actions");
    details_panel.append(&source_label);
    now_actions.append(&source_button);
    now_actions.append(&about_track);
    now_actions.append(&about_artist);
    details_panel.append(&now_actions);
    let panel_space = vbox(0);
    panel_space.set_vexpand(false);
    details_panel.append(&panel_space);
    let lyrics = icon_button("text-x-generic-symbolic", "Текст песни · Ctrl+L", "flat");
    lyrics.set_widget_name("player-lyrics");
    let player_bar = hbox(22);
    player_bar.add_css_class("player");
    player_bar.set_spacing(12);
    root.append(&player_bar);
    let mini = hbox(12);
    mini.set_width_request(180);
    let mini_picture = gtk::Picture::new();
    mini_picture.set_size_request(46, 46);
    mini_picture.set_content_fit(gtk::ContentFit::Cover);
    mini_picture.add_css_class("cover");
    mini_picture.set_can_shrink(true);
    let mini_cover = cover_frame(&mini_picture, 64);
    mini_cover.set_widget_name("player-track-cover");
    mini.append(&mini_cover);
    let mini_text = vbox(4);
    mini_text.set_valign(gtk::Align::Center);
    mini_text.set_widget_name("player-track-text");
    let mini_title = label("Ничего не играет", "track-title");
    mini_title.set_widget_name("player-track-title");
    mini_title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    mini_title.set_width_chars(20);
    mini_title.set_max_width_chars(20);
    let mini_artist = label("", "muted small");
    mini_artist.set_ellipsize(gtk::pango::EllipsizeMode::End);
    mini_artist.set_width_chars(20);
    mini_artist.set_max_width_chars(20);
    mini_text.append(&mini_title);
    mini_text.append(&mini_artist);
    let mini_album = label("", "small muted");
    mini_album.set_width_chars(20);
    mini_album.set_max_width_chars(20);
    mini_album.set_ellipsize(gtk::pango::EllipsizeMode::End);
    mini_text.append(&mini_album);
    mini.append(&mini_text);
    let mini_like = icon_button("emblem-favorite-symbolic", "Нравится", "flat");
    mini_like.set_sensitive(false);
    mini.append(&mini_like);
    player_bar.append(&mini);
    let transport = vbox(2);
    transport.set_hexpand(true);
    let controls = hbox(6);
    controls.set_halign(gtk::Align::Center);
    let previous = icon_button("media-skip-backward-symbolic", "Предыдущий трек", "flat");
    let play_button = icon_button(
        "media-playback-start-symbolic",
        "Воспроизвести / пауза",
        "primary large-play",
    );
    play_button.set_widget_name("player-play");
    let next = icon_button("media-skip-forward-symbolic", "Следующий трек", "flat");
    let shuffle_player = gtk::ToggleButton::new();
    shuffle_player.set_child(Some(&symbol("media-playlist-shuffle-symbolic")));
    shuffle_player.add_css_class("flat");
    shuffle_player.set_tooltip_text(Some("Перемешивание"));
    let repeat_player = gtk::Button::new();
    repeat_player.set_child(Some(&symbol("media-playlist-repeat-symbolic")));
    repeat_player.set_tooltip_text(Some("Повторять очередь"));
    repeat_player.add_css_class("flat");
    shuffle_player.add_css_class("flat");
    controls.append(&shuffle_player);
    controls.append(&previous);
    controls.append(&play_button);
    controls.append(&next);
    controls.append(&repeat_player);
    transport.append(&controls);
    let timeline = hbox(10);
    let position = label("0:00", "small muted");
    let duration = label("0:00", "small muted");
    for clock in [&position, &duration] {
        clock.add_css_class("player-clock");
        clock.set_width_chars(7);
        clock.set_max_width_chars(7);
        clock.set_ellipsize(gtk::pango::EllipsizeMode::End);
        clock.set_xalign(0.5);
    }
    let seek = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0., 1., 1.);
    seek.set_hexpand(true);
    seek.set_draw_value(false);
    timeline.append(&position);
    let playback_timeline = timeline::Timeline::default();
    timeline.append(&material::slider(&seek, &playback_timeline));
    timeline.append(&duration);
    transport.append(&timeline);
    player_bar.append(&transport);
    let volume_row = hbox(4);
    volume_row.set_width_request(188);
    volume_row.set_hexpand(false);
    volume_row.set_halign(gtk::Align::End);
    volume_row.set_valign(gtk::Align::Center);
    let panel_button = icon_button("view-reveal-symbolic", "Сейчас играет", "flat panel-toggle");
    volume_row.append(&panel_button);
    volume_row.append(&lyrics);
    let volume_button = icon_button("audio-volume-high-symbolic", "Отключить звук", "flat");
    volume_button.set_widget_name("volume-mute");
    volume_row.append(&volume_button);
    let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0., 100., 1.);
    volume.set_draw_value(false);
    volume.set_widget_name("volume-control");
    // Older preferences stored linear gain; preserve their audible level.
    let initial_volume = read_pref("volume_curve_v2").as_f64().unwrap_or_else(|| {
        read_pref("volume")
            .as_f64()
            .map(|v| (v.clamp(0., 100.) / 100.).sqrt() * 100.)
            .unwrap_or(20.)
    });
    volume.set_value(initial_volume.clamp(0., 100.));
    volume.set_hexpand(false);
    volume.set_width_request(100);
    volume.set_tooltip_text(Some("Громкость"));
    volume_row.append(&material::volume(&volume));
    player_bar.append(&volume_row);
    let player = match Player::new() {
        Ok(p) => p,
        Err(e) => {
            status.set_text(&e);
            window.present();
            return;
        }
    };
    let sound_before_mute = Rc::new(Cell::new(volume.value().max(1.)));
    {
        let control = volume.clone();
        let remembered = sound_before_mute.clone();
        volume_button.connect_clicked(move |_| {
            if control.value() > 0. {
                control.set_value(0.);
            } else {
                control.set_value(remembered.get().min(control.adjustment().upper()));
            }
        });
    }
    let update_sound = |button: &gtk::Button, muted| {
        let (icon, action) = if muted {
            ("audio-volume-muted-symbolic", "Включить звук")
        } else {
            ("audio-volume-high-symbolic", "Отключить звук")
        };
        set_md_icon(button, icon);
        button.set_tooltip_text(Some(action));
        button.update_property(&[gtk::accessible::Property::Label(action)]);
    };
    update_sound(&volume_button, volume.value() == 0.);
    volume.connect_value_changed(move |control| {
        if control.value() > 0. {
            sound_before_mute.set(control.value());
        }
        update_sound(&volume_button, control.value() == 0.);
    });
    player.set("volume", &volume.value().to_string());
    if std::env::var_os("HOMEMUSIC_TEST").is_some() {
        player.set("mute", "yes");
    }
    let (tx, rx) = async_channel::unbounded();
    let ui = Rc::new(Ui {
        window,
        api: Api::new(),
        net: Network::new(2),
        navigation: Network::new(2),
        enrichment: Network::with_capacity(1, 8),
        collection_writer: Network::with_capacity(1, 8),
        collection_cache: RefCell::new(HashMap::new()),
        home_mix_art: RefCell::new(Value::Null),
        images: Network::new(2),
        tx,
        model,
        items: RefCell::new(vec![]),
        source_items: RefCell::new(vec![]),
        list_filter: list_filter.clone(),
        collection_view: collection_view.clone(),
        list_adjustment: scroll.vadjustment(),
        list_anchors: RefCell::new(HashMap::new()),
        restoring_list: Cell::new(false),
        list_update_ms: Cell::new(0.),
        personal_refresh_due: Cell::new(None),
        list,
        title,
        brand_mark,
        nav_buttons: navs.clone(),
        switching_nav: Cell::new(false),
        panel_revealer,
        show_details: Cell::new(settings::right_panel_startup()),
        panel_button: panel_button.clone(),
        sidebar_widget: sidebar.clone(),
        right_widget: panel.clone(),
        sidebar_width: Cell::new(sidebar_width),
        right_width: Cell::new(right_width),
        sidebar_drag: sidebar_drag.clone(),
        left_resize_handle: sidebar_handle.clone(),
        right_resize_handle: right_handle.clone(),
        right_drag: right_drag.clone(),
        top_bar: bar.clone(),
        player_bar: player_bar.clone(),
        volume_control: volume.clone(),
        wave_frame: Cell::new(None),
        more_button,
        more_offset: Cell::new(None),
        subtitle,
        status,
        hero,
        empty,
        search,
        search_timer: RefCell::new(None),
        search_scope: search_scope.clone(),
        search_kind: search_kind.clone(),
        search_filters,
        request: Cell::new(0),
        resolve: Cell::new(0),
        page: RefCell::new(String::new()),
        cache: RefCell::new(VecDeque::new()),
        stars: RefCell::new(HashSet::new()),
        stars_loaded: Cell::new(false),
        favorites_snapshot: RefCell::new(Value::Null),
        favorite_pending: RefCell::new(HashSet::new()),
        personal_mixes: RefCell::new(Value::Null),
        player,
        queue: RefCell::new(vec![]),
        index: Cell::new(0),
        user_queue_end: Cell::new(0),
        restore_position: Cell::new(None),
        resolving_pause: Cell::new(None),
        refill_after: Cell::new(Instant::now()),
        wave_waiting: Cell::new(false),
        wave_start_request: Cell::new(None),
        current: RefCell::new(None),
        wave: RefCell::new(None),
        playing: Cell::new(false),
        started: Cell::new(false),
        playback: RefCell::new(String::new()),
        listened: Cell::new(0.),
        last_tick: Cell::new(Instant::now()),
        last_progress: Cell::new(0),
        play_button,
        lyrics_button: lyrics.clone(),
        now_lyrics,
        seek,
        position,
        duration,
        timeline: playback_timeline,
        now_title,
        now_artist,
        now_album,
        source_label,
        personal_sidebar: vbox(2),
        mix_pictures: (0..4)
            .map(|_| {
                let p = gtk::Picture::new();
                p.set_size_request(48, 48);
                p
            })
            .collect(),
        album_artist: RefCell::new(None),
        now_picture,
        now_symbol,
        mini_title,
        mini_artist,
        mini_album,
        mini_picture,
        like_button,
        cover_cache: RefCell::new(VecDeque::new()),
        cover_pending: RefCell::new(HashSet::new()),
        cover_flush_scheduled: Cell::new(false),
        wanted_covers: Arc::new(Mutex::new(HashSet::new())),
        deferred_covers: RefCell::new(HashMap::new()),
        cover_waiters: RefCell::new(HashMap::new()),
        cover_bindings: RefCell::new(HashMap::new()),
        cover_retries: RefCell::new(HashMap::new()),
        mpris: RefCell::new(None),
        _theme: provider,
        _monitor: monitor,
        test: std::env::var_os("HOMEMUSIC_TEST").is_some(),
        pins: RefCell::new(pins::load()),
        active_rows: RefCell::new(vec![]),
        row_state: RefCell::new(String::new()),
        played_history: RefCell::new(VecDeque::new()),
        history: RefCell::new(vec![]),
        history_index: Cell::new(0),
        history_move: Cell::new(false),
        shelves,
        sidebar_items,
        page_cover,
        page_art: RefCell::new(String::new()),
        home_scroll,
        page_tools: tools,
        collection_play: mix_button.clone(),
        collection_shuffle: shuffle.clone(),
        playlist_manage: playlist_manage.clone(),
        album_article: album_article.clone(),
        downloads_jobs: downloads_jobs.clone(),
        playlist_create: create.clone(),
        sort: sort.clone(),
        album_info: RefCell::new(None),
        content_list: overlay,
        repeat: Cell::new(0),
        shuffled: Cell::new(false),
        queue_original: RefCell::new(vec![]),
        motion_mode: Cell::new(read_pref("motion_mode").as_u64().unwrap_or(
            if read_pref("animations").as_bool() == Some(false) {
                0
            } else {
                1
            },
        ) as u32),
        repeat_button: repeat_player.clone(),
        shuffle_button: shuffle_player.clone(),
        mini_like: mini_like.clone(),
        preview_wave: RefCell::new(None),
        queue_panel,
        queue_model: RefCell::new(None),
        history_panel,
        panel_stack,
        panel_tabs: panel_tabs
            .iter()
            .map(|(b, n)| (b.clone(), n.to_string()))
            .collect(),
        sidebar_revealer,
        library_toggle: library_toggle.clone(),
        sidebar_requested: Cell::new(true),
        updating_panels: Cell::new(false),
        sidebar_scroll,

        settings_view,
        settings: settings::Settings::new(),
        artist_view,
        artist_body,
        artist_covers: RefCell::new(vec![]),
        autoplay: Cell::new(read_pref("autoplay").as_bool().unwrap_or(true)),
        autoplay_pending: Cell::new(false),
        continuation_generation: Cell::new(0),
        continuation_restarted: Cell::new(false),
    });
    cover_viewer::install(&ui);
    layout::install(&ui, &sidebar_handle, &sidebar_drag, true);
    layout::install(&ui, &right_handle, &right_drag, false);
    {
        let u = ui.clone();
        details_toggle.connect_clicked(move |_| {
            let show = false;
            u.show_details.set(Some(show));
            u.update_panels();
        });
    }
    {
        let u = ui.clone();
        ui.more_button.connect_clicked(move |_| u.load_more());
    }
    {
        let u = ui.clone();
        library_toggle.connect_toggled(move |b| {
            if u.updating_panels.get() {
                return;
            }
            u.sidebar_requested.set(b.is_active());
            if b.is_active() && u.compact_layout() {
                u.show_details.set(Some(false));
            }
            u.update_panels();
        });
    }
    {
        let u = ui.clone();
        source_button.connect_clicked(move |_| {
            if let Some(t) = u.current.borrow().clone() {
                u.source_dialog(&t);
            }
        });
    }
    {
        let u = ui.clone();
        album_article.connect_clicked(move |_| {
            if let Some((title, artist)) = u.album_info.borrow().clone() {
                details::show_album(&u.window, u.api.clone(), title, artist);
            }
        });
    }
    motion::apply(&ui.window.clone().upcast(), ui.motion_mode.get());
    {
        let u = ui.clone();
        mini_like.connect_clicked(move |_| {
            if let Some(t) = u.current.borrow().clone() {
                u.favorite(&t, !t.liked);
            }
        });
    }
    {
        let u = ui.clone();
        about_track.connect_clicked(move |_| {
            if let Some(t) = u.current.borrow().clone() {
                u.open_track_info(t);
            }
        });
    }
    {
        let u = ui.clone();
        about_artist.connect_clicked(move |_| {
            if let Some(t) = u.current.borrow().clone() {
                u.open_artist(&t);
            }
        });
    }
    {
        let u = ui.clone();
        panel_button.connect_clicked(move |_| {
            if u.panel_revealer.reveals_child() {
                u.show_details.set(Some(false));
                u.update_panels();
            } else {
                let tab = u
                    .panel_stack
                    .visible_child_name()
                    .unwrap_or_else(|| "now".into());
                u.select_panel(&tab);
            }
        });
    }
    {
        let u = ui.clone();
        ui.panel_stack.connect_visible_child_notify(move |stack| {
            for (button, name) in &u.panel_tabs {
                button.set_active(stack.visible_child_name().as_deref() == Some(name.as_str()));
            }
            if stack.visible_child_name().as_deref() == Some("history") {
                u.fill_history_panel();
            } else if stack.visible_child_name().as_deref() == Some("queue") {
                u.fill_queue_panel();
            }
            u.update_panel_button();
        });
    }
    {
        let u = ui.clone();
        playlist_manage.connect_clicked(move |b| {
            let path = u.page.borrow().clone();
            if path.starts_with("/api/playlists/") {
                u.playlist_menu(path, u.title.text().to_string(), b.upcast_ref());
            }
        });
    }
    {
        let u = ui.clone();
        ui.artist_view
            .vadjustment()
            .connect_value_changed(move |_| u.flush_artist_covers());
    }
    for (button, name) in panel_tabs {
        let u = ui.clone();
        button.connect_toggled(move |b| {
            if b.is_active() && u.panel_stack.visible_child_name().as_deref() != Some(name) {
                u.select_panel(name);
            }
        });
    }
    ui.install_factory();
    ui.install_queue_drag();
    {
        let weak = Rc::downgrade(&ui);
        scroll.vadjustment().connect_value_changed(move |_| {
            if let Some(u) = weak.upgrade() {
                if !u.cover_flush_scheduled.replace(true) {
                    let weak=Rc::downgrade(&u);
                    glib::timeout_add_local_once(Duration::from_millis(80),move ||{if let Some(u)=weak.upgrade(){u.cover_flush_scheduled.set(false);u.flush_deferred_covers();}});
                }
            }
        });
    }
    for filter in [&search_scope, &search_kind] {
        let u = ui.clone();
        filter.connect_selected_notify(move |_| {
            let query = u.search.text().trim().to_string();
            if !query.is_empty() && u.page.borrow().starts_with("search") {
                u.navigate(&format!("search:{query}"), true);
            }
        });
    }
    {
        let u = ui.clone();
        sort.connect_selected_notify(move |s| {
            if u.restoring_list.get() {
                return;
            }
            if let Some(key) = Ui::list_sort_section(&u.page.borrow()) {
                let mut sorts = read_pref("list_sorts");
                if !sorts.is_object() {
                    sorts = json!({});
                }
                sorts[key] = json!(s.selected());
                u.save_preferences(&[("list_sorts", sorts)], &[]);
            }
            u.apply_list_options();
        });
    }
    {
        let weak = Rc::downgrade(&ui);
        list_filter.connect_changed(move |_| {
            if let Some(u) = weak.upgrade() {
                if !u.restoring_list.get() {
                    u.apply_list_options();
                }
            }
        });
    }

    {
        let weak = Rc::downgrade(&ui);
        collection_view.connect_selected_notify(move |control| {
            let Some(u) = weak.upgrade() else { return };
            if u.restoring_list.get() || control.selected() > 1 {
                return;
            }
            let page = u.page.borrow().clone();
            if Ui::is_collection(&page) {
                let mut views = read_pref("list_views");
                if !views.is_object() {
                    views = json!({});
                }
                views[&page] = json!(control.selected());
                u.save_preferences(&[("list_views", views)], &[]);
                u.fill_card_page();
            }
        });
    }

    {
        let u = ui.clone();
        back.connect_clicked(move |_| u.history_step(-1));
    }
    {
        let u = ui.clone();
        forward.connect_clicked(move |_| u.history_step(1));
    }
    {
        let u = ui.clone();
        home.connect_clicked(move |_| u.navigate("home", false));
    }
    {
        let u = ui.clone();
        create.connect_clicked(move |_| u.create_playlist());
    }
    {
        let weak = Rc::downgrade(&ui);
        downloads_jobs.connect_clicked(move |_| {
            if let Some(u) = weak.upgrade() {
                let route = if u.page.borrow().as_str() == "downloads" { "download_jobs" } else { "downloads" };
                u.navigate(route, false);
            }
        });
    }
    {
        let u = ui.clone();
        preferences.connect_clicked(move |_| u.preferences());
    }
    {
        let u = ui.clone();
        lyrics.connect_clicked(move |_| {
            if let Some(t) = u.current.borrow().clone() {
                u.open_details(t);
            }
        });
    }
    {
        let weak = Rc::downgrade(&ui);
        ui.now_lyrics.widget().connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade()
                && let Some(track) = ui.current.borrow().clone()
            {
                ui.open_details(track);
            }
        });
    }
    {
        let u = ui.clone();
        repeat_player.connect_clicked(move |b| {
            let mode = (u.repeat.get() + 1) % 3;
            u.repeat.set(mode);
            set_md_icon(
                b,
                if mode == 2 {
                    "repeat-one"
                } else {
                    "media-playlist-repeat-symbolic"
                },
            );
            if mode == 0 {
                b.remove_css_class("selected")
            } else {
                b.add_css_class("selected")
            }
            b.set_tooltip_text(Some(match mode {
                1 => "Повтор всей очереди",
                2 => "Повтор трека",
                _ => "Повтор выключен",
            }));
        });
    }
    {
        let u = ui.clone();
        shuffle_player.connect_toggled(move |b| {
            u.set_shuffle(b.is_active());
        });
    }

    *ui.mpris.borrow_mut() = mpris::install(&ui);
    for (b, key) in navs {
        let u = ui.clone();
        b.connect_toggled(move |b| {
            if b.is_active() && !u.switching_nav.get() {
                u.navigate(&key, false);
            }
        });
    }
    {
        let u = ui.clone();
        refresh.connect_clicked(move |_| {
            let key = u.page.borrow().clone();
            u.navigate(&key, true);
        });
    }
    {
        let u = ui.clone();
        ui.search.connect_changed(move |s| {
            if let Some(id) = u.search_timer.borrow_mut().take() {
                id.remove();
            }
            let q = s.text().trim().to_string();
            let weak = Rc::downgrade(&u);
            let id = glib::timeout_add_local_once(Duration::from_millis(450), move || {
                if let Some(u) = weak.upgrade() {
                    u.search_timer.borrow_mut().take();
                    if q.chars().count() >= 2 {
                        u.navigate(&format!("search:{q}"), false);
                    }
                }
            });
            *u.search_timer.borrow_mut() = Some(id);
        });
    }
    {
        let u = ui.clone();
        ui.search.connect_activate(move |s| {
            if let Some(id) = u.search_timer.borrow_mut().take() {
                id.remove();
            }
            let q = s.text().trim().to_string();
            if !q.is_empty() {
                u.navigate(&format!("search:{q}"), false);
            }
        });
    }
    {
        let u = ui.clone();
        wave_button.connect_clicked(move |_| u.start_wave());
    }
    {
        let u = ui.clone();
        mix_button.connect_clicked(move |_| {
            let first = u.items.borrow().iter().position(|i| matches!(i, Item::Track(_)));
            if let Some(first) = first {
                u.activate(first);
            }
        });
    }
    {
        let u = ui.clone();
        queue_button.connect_clicked(move |_| u.show_queue());
    }
    {
        let u = ui.clone();
        shuffle.connect_clicked(move |_| {
            let mut q: Vec<_> = u
                .items
                .borrow()
                .iter()
                .filter_map(|i| {
                    if let Item::Track(t) = i {
                        Some(t.clone())
                    } else {
                        None
                    }
                })
                .collect();
            let seed = unique()
                .bytes()
                .fold(0usize, |a, b| a.wrapping_mul(31).wrapping_add(b as usize));
            for i in (1..q.len()).rev() {
                q.swap(i, seed.wrapping_mul(i + 17) % (i + 1));
            }
            if !q.is_empty() {
                u.invalidate_continuation();
                u.user_queue_end.set(0);
                *u.queue_original.borrow_mut() = q.clone();
                *u.wave.borrow_mut() = None;
                *u.queue.borrow_mut() = q;
                u.play_index(0);
            }
        });
    }
    {
        let u = ui.clone();
        import_button.connect_clicked(move |_| u.import_dialog());
    }
    {
        let u = ui.clone();
        ui.play_button.connect_clicked(move |_| {
            u.toggle_play();
        });
    }
    {
        let u = ui.clone();
        previous.connect_clicked(move |_| {
            if u.player.number("time-pos") > 3. {
                u.seek_to(0.);
            } else if u.index.get() > 0 {
                u.advance_to(u.index.get() - 1, false);
            }
        });
    }
    {
        let u = ui.clone();
        next.connect_clicked(move |_| u.next(false));
    }
    {
        let u = ui.clone();
        volume.connect_value_changed(move |s| u.player.set("volume", &s.value().to_string()));
    }
    ui.install_seek();
    ui.apply_seek_step();
    {
        let u = ui.clone();
        ui.like_button.connect_clicked(move |_| {
            let t = u.current.borrow().clone();
            if let Some(t) = t {
                u.favorite(&t, !t.liked);
            }
        });
    }
    {
        let u = ui.clone();
        ui.list.connect_activate(move |_, p| u.activate(p as usize));
    }
    {
        let u = Rc::downgrade(&ui);
        glib::timeout_add_local(Duration::from_millis(500), move || {
            if let Some(u) = u.upgrade() {
                u.tick();
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        });
    }
    let keys = gtk::EventControllerKey::new();
    {
        let u = ui.clone();
        keys.connect_key_pressed(move |_, key, _, mods| u.handle_shortcut(key, mods));
    }
    ui.window.add_controller(keys);
    autoscroll::install(&ui.window);
    for (label, action) in [
        (&ui.mini_title, 0),
        (&ui.now_title, 0),
        (&ui.mini_artist, 1),
        (&ui.mini_album, 2),
        (&ui.now_artist, 1),
        (&ui.now_album, 2),
    ] {
        let weak=Rc::downgrade(&ui);
        ui.link_track_field(label,move || {let u=weak.upgrade()?;let track=u.current.borrow().clone();track},action==1);
    }

    {
        let u = ui.clone();
        let click = gtk::GestureClick::new();
        click.connect_released(move |_, _, _, _| {
            let album = u.album_info.borrow().clone();
            if let Some((title, artist)) = album {
                details::show_album(&u.window, u.api.clone(), title, artist);
            } else if u.page.borrow().contains("/artist") {
                details::show_artist(&u.window, u.api.clone(), u.title.text().to_string());
            }
        });

        ui.title.add_controller(click);
    }
    {
        let u = ui.clone();
        let click = gtk::GestureClick::new();
        click.connect_released(move |_, _, _, _| {
            // Drop the RefCell guard before navigation resets album metadata.
            let artist = u.album_artist.borrow().clone();
            if let Some(track) = artist {
                u.open_artist(&track);
            }
        });

        ui.subtitle.add_controller(click);
    }

    // Reserve only the two navigation buttons; do not compete with primary drags.
    for (button, direction) in [(8, -1), (9, 1)] {
        let mouse = gtk::GestureClick::new();
        mouse.set_button(button);
        mouse.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&ui);
        mouse.connect_pressed(move |_, _, _, _| {
            if let Some(u) = weak.upgrade() {
                u.history_step(direction);
            }
        });
        ui.window.add_controller(mouse);
    }
    {
        let u = ui.clone();
        ui.window.connect_close_request(move |_| {
            if !u.test {
                let volume = u.player.number("volume");
                u.save_preferences(&[("volume_curve_v2", json!(volume)), ("volume", json!((volume / 100.).powi(2) * 100.))], &[]);
            }
            u.save_playback();
            u.save_startup_page();
            u.report("interrupted");
            u.player.command(&["stop"]);
            glib::Propagation::Proceed
        });
    }
    {
        let u = ui.clone();
        glib::spawn_future_local(async move {
            while let Ok(event) = rx.recv().await {
                u.handle(event);
            }
        });
    }
    if ui.test {
        install_test_control(&ui);
    }
    ui.apply_appearance();
    ui.apply_volume_limit();
    if present {
        ui.window.present();
    }
    ui.connect_backend();
}

impl Ui {
    fn notice(&self, text: &str, error: bool) {
        self.status.set_text(text);
        self.status.set_visible(!text.is_empty());
        if error {
            self.status.add_css_class("error");
        } else {
            self.status.remove_css_class("error");
        }
    }
    fn load_collection(self: &Rc<Self>, route: &'static str, refresh: bool) {
        let revision = {
            let mut cache = self.collection_cache.borrow_mut();
            let entry = cache.entry(route).or_default();
            if entry.fetching || (!refresh && entry.received.is_some_and(|time| time.elapsed() < Duration::from_secs(12))) {
                return;
            }
            entry.fetching = true;
            entry.revision
        };
        let api = self.api.clone();
        let tx = self.tx.clone();
        if !self.navigation.run(move || {
            let path = if route == "likes" { "/api/nd/liked" } else { "/api/downloads?status=done&limit=500" };
            let result = api.get_with_timeout(path, Duration::from_secs(40))
                .map(|value| collection_disk::snapshot(route, value));
            let _ = tx.send_blocking(Event::CollectionCache(route, revision, result));
        }) {
            self.collection_cache.borrow_mut().entry(route).or_default().fetching = false;
            if self.page.borrow().as_str() == route {
                self.notice("Список занят. Повторите попытку.", true);
            }
        }
    }

    fn restore_collections(&self) {
        let mut cache = self.collection_cache.borrow_mut();
        for route in ["likes", "downloads"] {
            if let Some(value) = collection_disk::path(&self.api, route).and_then(|path| collection_disk::load(&path)) {
                cache.entry(route).or_default().value = Some(value);
            }
        }
    }

    fn open_cached_collection(self: &Rc<Self>, route: &'static str, refresh: bool) {
        let cached = self.collection_cache.borrow().get(route).and_then(|entry| entry.value.clone());
        if let Some(value) = cached {
            self.handle(Event::Page(self.request.get(), route.to_owned(), Ok(value)));
        } else {
            self.subtitle.set_text("Загружаем…");
        }
        self.load_collection(route, refresh);
    }

    fn invalidate_collection(&self, route: &'static str) {
        let mut cache = self.collection_cache.borrow_mut();
        let entry = cache.entry(route).or_default();
        entry.revision += 1;
        entry.value = None;
        entry.received = None;
        entry.fetching = false;
    }

    fn request(
        self: &Rc<Self>,
        kind: String,
        path: String,
        query: Vec<(String, String)>,
        post: Option<Value>,
    ) {
        let generation = self.request.get() + 1;
        self.request.set(generation);
        self.notice("Загружаем…", false);
        if (post.is_none() || kind.starts_with("wave_preview:"))
            && !matches!(kind.as_str(), "downloads" | "download_jobs")
        {
            let cached = self
                .cache
                .borrow()
                .iter()
                .find(|(k, _)| k == &kind)
                .map(|(_, v)| v.clone());
            if let Some(v) = cached {
                self.handle(Event::Page(generation, kind, Ok(v)));
                return;
            }
        }
        let api = self.api.clone();
        let tx = self.tx.clone();
        let is_navigation = post.is_none();
        let pool = if is_navigation { &self.navigation } else { &self.net };
        let enrichment = self.enrichment.clone();
        if !pool.run(move || {
            let result = if let Some(body) = post {
                api.post(&path, body)
            } else if query.is_empty() {
                api.get_with_timeout(&path, Duration::from_secs(30))
            } else {
                let params: Vec<_> = query.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                api.query_with_timeout(&path, &params, Duration::from_secs(25))
            };
            let local = result.as_ref().ok().filter(|_| {
                path.starts_with("/api/nd/artists/") || path.starts_with("/api/nd/albums/")
            }).cloned();
            let home = kind == "home" && result.is_ok();
            let _ = tx.send_blocking(Event::Page(generation, kind.clone(), result));
            if let Some(local) = local {
                let artist = path.starts_with("/api/nd/artists/");
                enrichment.run(move || {
                    let complete = if artist {
                        artist::complete_artist(&api, local.clone())
                    } else {
                        complete_album(&api, local.clone())
                    };
                    if complete != local {
                        let _ = tx.send_blocking(Event::EnrichedPage(generation, kind, local, complete));
                    }
                });
            } else if home {
                enrichment.run(move || {
                    if let Ok(mix) = api.get_with_timeout("/api/mixes/today", Duration::from_secs(12)) {
                        let tracks = api::tracks(&mix["mix"]["tracks"]);
                        let _ = tx.send_blocking(Event::MixArtwork(tracks.into_iter().take(4).collect()));
                    }
                });
            }
        }) {
            if self.wave_start_request.get() == Some(generation) {
                self.wave_start_request.set(None);
                self.update_play_control();
            }
            self.notice("Запрос не отправлен. Повторите попытку", true);
        }
    }
    fn refresh_home_field(self: &Rc<Self>, key: &str, value: Value) {
        let displayed = self.page.borrow().as_str() == "home";
        let mut cache = self.cache.borrow_mut();
        let updated = cache.iter_mut().find(|(key, _)| key == "home").and_then(|(_, home)| {
            if home[key] == value { return None; }
            home[key] = value;
            displayed.then(|| home.clone())
        });
        drop(cache);
        if let Some(home) = updated { self.fill_home(&home); }
    }

    fn remember_list_anchor(&self) {
        if self.content_list.is_visible() && self.list_filter.text().is_empty()
            && !self.items.borrow().is_empty()
        {
            let mut anchors = self.list_anchors.borrow_mut();
            if anchors.len() >= 64 {
                anchors.clear();
            }
            anchors.insert(self.page.borrow().clone(), self.list_adjustment.value());
        }
    }

    fn navigate(self: &Rc<Self>, key: &str, refresh: bool) {
        self.remember_startup_page(key);
        if !self.history_move.get() && self.history.borrow().last().map(String::as_str) != Some(key)
        {
            let n = self.history_index.get();
            let mut history = self.history.borrow_mut();
            if !history.is_empty() {
                history.truncate(n + 1);
            }
            history.push(key.into());
            self.history_index.set(history.len() - 1);
        }
        self.remember_list_anchor();
        self.restoring_list.set(true);
        self.list_filter.set_text("");
        let sort = Self::list_sort_section(key)
            .and_then(|section| read_pref("list_sorts")[section].as_u64())
            .unwrap_or(0).min(2);
        self.sort.set_selected(sort as u32);
        self.collection_view.set_visible(Self::is_collection(key));
        self.collection_view.set_selected(
            read_pref("list_views")[key]
                .as_u64()
                .filter(|view| *view <= 1)
                .map_or_else(settings::default_collection_view, |view| view as u32),
        );
        self.list_filter.set_visible(Self::list_sort_section(key).is_some());
        self.restoring_list.set(false);
        self.search_filters.set_visible(key.starts_with("search"));
        self.notice("", false);
        // Invalidate pending responses even for locally rendered destinations.
        self.request.set(self.request.get() + 1);
        *self.page.borrow_mut() = key.into();
        self.set_items(vec![]);
        self.subtitle.set_text("Загружаем…");
        self.more_button.set_visible(false);
        self.settings_view.set_visible(false);
        self.artist_view.set_visible(false);
        self.shelves.set_visible(key == "home");
        self.home_scroll.set_visible(key == "home");
        self.page_tools.set_visible(key != "home");
        self.configure_collection_actions(key);
        self.content_list.set_visible(key != "home");
        self.load_cover("", &self.page_cover);
        self.page_cover.set_visible(false);
        self.title.remove_css_class("media-link");
        self.subtitle.remove_css_class("media-link");
        self.title.set_cursor_from_name(None);
        self.subtitle.set_cursor_from_name(None);
        *self.album_info.borrow_mut() = None;
        *self.album_artist.borrow_mut() = None;

        let section = if key.starts_with("search:") {
            "search"
        } else if key.starts_with("/api/playlists/") {
            "playlists"
        } else if key == "download_jobs" {
            "downloads"
        } else if key.starts_with("/api/nd/") {
            "library"
        } else if matches!(key, "mix" | "wave") {
            "home"
        } else {
            key
        };
        self.switching_nav.set(true);
        for (button, name) in &self.nav_buttons {
            button.set_active(name == section);
        }
        self.switching_nav.set(false);
        if refresh {
            self.cache.borrow_mut().retain(|(k, _)| k != key);
        }
        if let Some(name) = key.strip_prefix("artist_tracks:") {
            let tracks = self
                .cache
                .borrow()
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| api::tracks(&v["tracks"]))
                .unwrap_or_default();
            self.title.set_text(&format!("Популярные треки · {name}"));
            self.subtitle.set_text(&format!("{} треков", tracks.len()));
            self.more_button.set_visible(false);
            self.set_items(tracks.into_iter().map(Item::Track).collect());
            self.notice("", false);
            return;
        }
        if let Some(id) = key.strip_prefix("personal:") {
            let group = self.personal_mixes.borrow()["groups"]
                .as_array()
                .and_then(|g| g.iter().find(|g| g["id"].as_str() == Some(id)))
                .cloned();
            if let Some(group) = group {
                self.hero.set_visible(false);
                self.title
                    .set_text(group["title"].as_str().unwrap_or("Микс"));
                self.subtitle.set_text("Из вашей музыки");
                self.set_items(
                    api::tracks(&group["tracks"])
                        .into_iter()
                        .map(Item::Track)
                        .collect(),
                );
                self.more_button.set_visible(false);
            } else {
                self.title.set_text("Микс обновлён");
                self.subtitle
                    .set_text("Выберите актуальный микс дня в медиатеке");
            }
            return;
        }
        if key == "settings" {
            self.title.set_text("Настройки");
            self.subtitle.set_text("");
            self.page_tools.set_visible(false);
            self.home_scroll.set_visible(false);
            self.content_list.set_visible(false);
            self.show_preferences();
            self.settings_view.set_visible(true);
            motion::page(&self.settings_view, self.motion_mode.get());
            self.more_button.set_visible(false);
            return;
        }
        self.hero.set_visible(key == "home");
        let (title, sub, path) = match key {
            "home" => ("Главная", "", "/api/nd/home"),
            "search" => {
                self.title.set_text("Поиск");
                self.subtitle.set_text("");
                self.search.grab_focus();
                let query = self.search.text().trim().to_string();
                if query.is_empty() {
                    self.set_items(vec![]);
                    self.more_button.set_visible(false);
                } else {
                    self.navigate(&format!("search:{query}"), false);
                }
                return;
            }
            "likes" => {
                self.title.set_text("Любимые треки");
                self.open_cached_collection("likes", refresh);
                return;
            }
            "library" => ("Моя библиотека", "", "/api/nd/albums"),
            "artists" => ("Исполнители", "", "/api/nd/artists"),
            "playlists" => ("Ваши плейлисты", "", "/api/playlists"),
            "downloads" => {
                self.title.set_text("Скачанное");
                self.open_cached_collection("downloads", refresh);
                return;
            }
            "download_jobs" => ("Задачи загрузки", "", "/api/downloads"),
            "mix" => ("Микс дня", "", "/api/mixes/today"),
            _ if key.starts_with("search:") => {
                self.title.set_text("Результаты поиска");
                self.subtitle
                    .set_text(&format!("По запросу «{}»", &key[7..]));
                self.request(
                    key.into(),
                    if self.search_scope.selected() == 0 {
                        "/api/search"
                    } else {
                        "/api/search/providers"
                    }
                    .into(),
                    if self.search_scope.selected() == 2 {
                        vec![
                            ("q".into(), key[7..].into()),
                            ("provider".into(), "reuploads".into()),
                        ]
                    } else {
                        vec![("q".into(), key[7..].into())]
                    },
                    None,
                );
                return;
            }
            _ if key.starts_with('/') => {
                self.title.set_text("Медиатека");
                self.subtitle.set_text("");
                self.request(key.into(), key.into(), vec![], None);
                return;
            }
            _ => return,
        };
        self.title.set_text(title);
        self.subtitle.set_text(sub);
        self.request(key.into(), path.into(), vec![], None);
    }
    fn is_album(route: &str) -> bool {
        route.starts_with("/api/nd/albums/") || route.starts_with("/api/catalog/album/")
    }

    fn configure_collection_actions(&self, route: &str) {
        let playlist = route.starts_with("/api/playlists/");
        let album = Self::is_album(route);
        let downloaded = route == "downloads";
        let jobs = route == "download_jobs";
        let index = route == "playlists";
        let playback = !matches!(route, "home" | "settings" | "playlists" | "download_jobs" | "artists" | "library");
        if let Some(header) = self.title.parent().and_then(|heading| heading.parent()) {
            for class in ["playlist-heading", "album-heading"] {
                header.remove_css_class(class);
            }
            if playlist || downloaded {
                header.add_css_class("playlist-heading");
            } else if album {
                header.add_css_class("album-heading");
            }
        }
        self.collection_play.set_visible(playback);
        self.collection_play.set_sensitive(false);
        self.collection_shuffle.set_visible(playback && !album);
        self.collection_shuffle.set_sensitive(false);
        self.playlist_manage.set_visible(playlist);
        self.album_article.set_visible(album);
        self.downloads_jobs.set_visible(downloaded || jobs);
        self.downloads_jobs.set_label(if jobs { "Скачанное" } else { "Задачи загрузки" });
        self.playlist_create.set_visible(index);
        self.sort.set_visible(Self::list_sort_section(route).is_some() || route.starts_with("search:"));
        set_action_label(&self.collection_play, if album {
            "Воспроизвести альбом"
        } else if playlist {
            "Воспроизвести плейлист"
        } else {
            "Воспроизвести список"
        });
        if let Some(label) = self.collection_play.child().and_then(|row| row.last_child())
            .and_downcast::<gtk::Label>()
        {
            label.set_text(if album { "Слушать альбом" } else if playlist { "Слушать плейлист" } else { "Слушать" });
        }
    }

    fn update_collection_playback(&self) {
        let available = self.items.borrow().iter().any(|item| matches!(item, Item::Track(_)));
        self.collection_play.set_sensitive(available);
        self.collection_shuffle.set_sensitive(available);
    }

    fn is_collection(route: &str) -> bool {
        matches!(route, "library" | "artists" | "playlists")
    }

    fn list_sort_section(route: &str) -> Option<&str> {
        match route {
            "likes" | "library" | "artists" | "playlists" | "local" | "downloads" => Some(route),
            _ if route.starts_with("/api/playlists/") => Some("playlist-tracks"),
            _ if route.starts_with("artist_tracks:") => Some("artist-tracks"),
            _ => None,
        }
    }

    fn set_items(&self, mut items: Vec<Item>) {
        for item in &mut items {
            if let Item::Track(t) = item {
                *t = self.pinned(t.clone());
            }
        }
        *self.source_items.borrow_mut() = items;
        self.render_items();
        self.update_collection_playback();
        let anchor = self.list_anchors.borrow().get(self.page.borrow().as_str()).copied();
        if let Some(anchor) = anchor.filter(|_| self.model.n_items() > 0) {
            let first = self.model.item(0);
            let model = self.model.clone();
            let adjustment = self.list_adjustment.clone();
            let after_layout = Cell::new(false);
            self.list.add_tick_callback(move |_, _| {
                if !after_layout.replace(true) {
                    return glib::ControlFlow::Continue;
                }
                if model.item(0) == first {
                    adjustment.set_value(anchor);
                }
                glib::ControlFlow::Break
            });
        }
    }

    fn apply_list_options(self: &Rc<Self>) {
        self.render_items();
        if Self::is_collection(&self.page.borrow()) {
            self.fill_card_page();
        } else if self.page.borrow().starts_with("search:") && self.artist_view.is_visible() {
            self.fill_search();
        }
        self.update_collection_playback();
    }

    fn render_items(&self) {
        let started = Instant::now();
        let filter = if Self::list_sort_section(&self.page.borrow()).is_some() {
            self.list_filter.text().trim().to_lowercase()
        } else {
            String::new()
        };
        let mut items: Vec<_> = self.source_items.borrow().iter().filter(|item| {
            let (title, subtitle) = match item {
                Item::Track(track) => (&track.title, &track.artist),
                Item::Card { title, sub, .. } => (title, sub),
            };
            filter.is_empty() || title.to_lowercase().contains(&filter)
                || subtitle.to_lowercase().contains(&filter)
        }).cloned().collect();
        let sort = self.sort.selected();
        if sort != 0 && (Self::list_sort_section(&self.page.borrow()).is_some()
            || self.page.borrow().starts_with("search:"))
        {
            items.sort_by_cached_key(|item| {
                match item {
                    Item::Track(t) if sort == 1 => &t.title,
                    Item::Track(t) => &t.artist,
                    Item::Card { title, .. } if sort == 1 => title,
                    Item::Card { sub, .. } => sub,
                }.to_lowercase()
            });
        }
        let page = self.page.borrow();
        self.empty.set_visible(items.is_empty() && page.as_str() != "home");
        if let Some(empty) = self.empty.first_child().and_downcast::<gtk::Label>() {
            empty.set_text(match page.as_str() {
                "downloads" => "Нет скачанных треков",
                "download_jobs" => "Нет задач загрузки",
                _ => "Нет треков",
            });
        }

        let objects: Vec<_> = items
            .iter()
            .cloned()
            .map(glib::BoxedAnyObject::new)
            .collect();
        self.model.splice(0, self.model.n_items(), &objects);
        *self.items.borrow_mut() = items;
        self.row_state.borrow_mut().clear();
        self.refresh_track_rows();
        self.list_update_ms
            .set(started.elapsed().as_secs_f64() * 1000.);
    }
    fn show_queue(self: &Rc<Self>) {
        self.select_panel("queue");
    }
    fn show_wave_page(self: &Rc<Self>, page: &str) {
        self.home_scroll.set_visible(false);
        self.settings_view.set_visible(false);
        self.artist_view.set_visible(false);
        self.list_filter.set_visible(false);
        self.collection_view.set_visible(false);
        self.search_filters.set_visible(false);
        self.configure_collection_actions(page);
        self.load_cover("", &self.page_cover);
        self.page_cover.set_visible(false);
        self.page_tools.set_visible(true);
        self.more_button.set_visible(false);
        self.content_list.set_visible(true);
        self.hero.set_visible(false);
        self.title.remove_css_class("media-link");
        self.subtitle.remove_css_class("media-link");
        self.title.set_cursor_from_name(None);
        self.subtitle.set_cursor_from_name(None);
        self.album_info.borrow_mut().take();
        self.album_artist.borrow_mut().take();
        *self.page.borrow_mut() = page.into();
        self.set_items(vec![]);
    }

    fn start_wave(self: &Rc<Self>) {
        if self.wave_start_request.get() == Some(self.request.get()) {
            return;
        }
        self.invalidate_continuation();
        *self.wave.borrow_mut() = None;
        self.show_wave_page("wave");
        self.title.set_text("Моя волна");
        self.subtitle.set_text("Подбираем музыку под ваш вкус");
        self.wave_start_request.set(Some(self.request.get() + 1));
        self.request(
            "wave".into(),
            "/api/wave/start".into(),
            vec![],
            Some(json!({"client":"native/0.1","origin":"home_card"})),
        );
        self.update_play_control();
    }
    fn activate(self: &Rc<Self>, pos: usize) {
        let item = self.items.borrow().get(pos).cloned();
        match item {
            Some(Item::Card { path, .. }) => {
                if let Some(id) = path.strip_prefix("download:").and_then(|id| id.parse().ok()) {
                    self.open_download(id);
                } else {
                    self.navigate(&path, false);
                }
            }
            Some(Item::Track(_t)) => {
                self.invalidate_continuation();
                if self.page.borrow().as_str() == "wave_preview" {
                    *self.wave.borrow_mut() = self.preview_wave.borrow().clone();
                }
                if !matches!(
                    self.page.borrow().as_str(),
                    "wave" | "wave_preview" | "queue"
                ) {
                    *self.wave.borrow_mut() = None;
                }
                let queue: Vec<_> = self
                    .items
                    .borrow()
                    .iter()
                    .filter_map(|i| {
                        if let Item::Track(t) = i {
                            Some(t.clone())
                        } else {
                            None
                        }
                    })
                    .collect();
                let p = self
                    .items
                    .borrow()
                    .iter()
                    .take(pos)
                    .filter(|item| matches!(item, Item::Track(_)))
                    .count();
                self.user_queue_end.set(p);
                *self.queue_original.borrow_mut() = queue.clone();
                *self.queue.borrow_mut() = queue;
                self.play_index(p);
                if self.shuffled.get() {
                    self.shuffle_queue();
                }
            }
            _ => (),
        }
    }
    fn play_index(self: &Rc<Self>, index: usize) {
        self.open_queue_entry(index, None, false);
    }

    fn advance_to(self: &Rc<Self>, index: usize, finished: bool) {
        let paused = !finished
            && settings::pref_bool("keep_paused_on_skip", false)
            && self.current.borrow().is_some()
            && self.resolving_pause.get().unwrap_or_else(|| self.player.flag("pause"));
        self.open_queue_entry(index, None, paused);
    }
    fn open_queue_entry(self: &Rc<Self>, index: usize, position: Option<f64>, paused: bool) {
        let track = self
            .queue
            .borrow()
            .get(index)
            .cloned()
            .map(|t| self.pinned(t));
        let Some(track) = track else {
            return;
        };
        if self.started.get() {
            self.report("skip");
        }
        self.index.set(index);
        self.restore_position.set(position);
        self.resolving_pause.set(Some(paused));
        self.update_play_control();
        self.player.command(&["stop"]);
        self.reset_timeline(track.duration_s.unwrap_or(0.), position.unwrap_or(0.));
        self.playing.set(false);
        self.started.set(false);
        self.listened.set(0.);
        self.last_progress.set(0);
        *self.playback.borrow_mut() = unique();
        *self.current.borrow_mut() = Some(track.clone());
        self.update_now(&track);
        if self.queue_is_visible() {
            self.fill_queue_panel();
        }
        self.source_label.set_visible(false);
        self.notice("Находим лучший файл…", false);
        let generation = self.resolve.get() + 1;
        self.resolve.set(generation);
        let api = self.api.clone();
        let tx = self.tx.clone();
        if !self.net.run(move || {
            let selected = track.playback_ref.as_deref().unwrap_or(&track.reference);
            let mut r = api.query("/api/listen", &[("ref", selected)]);
            if r.is_err() && track.playback_ref.is_some() && selected.starts_with("file:") {
                if let Some(provider) = selected.split(':').nth(1) {
                    let query = format!("{} - {}", track.artist, track.title);
                    if let Ok(found) = api.query(
                        "/api/search/providers",
                        &[("q", query.as_str()), ("provider", provider)],
                    ) {
                        if found["candidates"].as_array().is_some_and(|candidates| {
                            candidates.iter().any(|c| {
                                format!("file:{}:{}", text(c, "provider"), text(c, "provider_id"))
                                    == selected
                            })
                        }) {
                            r = api.query("/api/listen", &[("ref", selected)]);
                        }
                    }
                }
            }
            let _ = tx.send_blocking(Event::Resolved(generation, r));
        }) {
            self.resolving_pause.set(None);
            self.update_play_control();
            self.notice("Не удалось открыть аудиопоток. Нажмите Play, чтобы повторить", true);
        }
    }
    fn next(self: &Rc<Self>, finished: bool) {
        if self.queue.borrow().is_empty() {
            return;
        }
        if self.current.borrow().is_none() {
            self.play_index(0);
            return;
        }
        if finished && self.repeat.get() == 2 {
            if self.started.replace(false) {
                self.report("finish");
            }
            self.play_index(self.index.get());
            return;
        }
        if finished {
            if self.started.replace(false) {
                self.report("finish");
            }
        }
        let next = self.index.get() + 1;
        if next < self.queue.borrow().len() {
            self.advance_to(next, finished);
        } else if self.repeat.get() != 0 {
            self.advance_to(0, finished);
        } else if let Some(sid) = self.wave.borrow().clone() {
            self.wave_waiting.set(true);
            self.continue_session(&sid);
        } else if self.autoplay.get() && !self.autoplay_pending.replace(true) {
            self.wave_waiting.set(true);
            let seed = self.current.borrow().clone();
            self.wave_from(seed.as_ref(), true);
        } else {
            self.player.command(&["stop"]);
            self.reset_timeline(self.timeline_duration(), 0.);
            self.playing.set(false);
            set_md_icon(&self.play_button, "media-playback-start-symbolic");
        }
    }
    fn update_now(self: &Rc<Self>, t: &Track) {
        self.now_lyrics.load(self, Some(t));
        self.now_title.set_text(&t.title);
        self.refresh_track_rows();
        self.now_artist.set_text(&t.artist);
        self.now_album
            .set_text(t.album_title.as_deref().unwrap_or(""));
        self.mini_title.set_text(&t.title);
        self.mini_artist.set_text(&t.artist);
        self.mini_album
            .set_text(t.album_title.as_deref().unwrap_or(""));
        self.update_like_controls();
        self.now_picture.set_paintable(None::<&gdk::Texture>);
        self.now_picture.set_visible(false);
        self.mini_picture.set_paintable(None::<&gdk::Texture>);
        self.now_symbol.set_visible(true);
        let cover = t.cover();
        self.load_cover(&cover, &self.now_picture);
        self.load_cover(&cover, &self.mini_picture);
    }
    fn update_like_controls(&self) {
        let current = self.current.borrow();
        let Some(track) = current.as_ref() else { return };
        let pending = self.favorite_pending.borrow().contains(&track.reference);
        for button in [&self.mini_like, &self.like_button] {
            button.set_sensitive(!pending);
            set_like_icon(button, track.liked);
            set_action_label(button, if pending {
                "Сохраняем…"
            } else if track.liked {
                "Убрать из любимого"
            } else {
                "Добавить в любимое"
            });
        }
    }
    fn favorite(self: &Rc<Self>, track: &Track, liked: bool) {
        if !self.favorite_pending.borrow_mut().insert(track.reference.clone()) {
            return;
        }
        self.update_like_controls();
        let api = self.api.clone();
        let tx = self.tx.clone();
        let track = track.clone();
        self.notice("Сохраняем…", false);
        let reference = track.reference.clone();
        if !self.net.run(move || {
            let body = json!({"targets":[{"ref":track.reference,"nd_song_id":track.nd_song_id,
                "target_data":{"title":track.title,"artist":track.artist,"album":track.album_title,"duration_s":track.duration_s}}],
                "unstar":!liked,"client":"native/0.1","event_id":unique()});
            let result=api.post("/api/star",body);
            let _=tx.send_blocking(Event::Favorite(track.reference,liked,result));
        }) {
            self.favorite_pending.borrow_mut().remove(&reference);
            self.update_like_controls();
            self.notice("Не удалось сохранить лайк. Повторите попытку", true);
        }
    }
    fn download(self: &Rc<Self>, track: &Track) {
        let api = self.api.clone();
        let tx = self.tx.clone();
        let reference = track.reference.clone();
        let track = track.clone();
        let extras = ["download_artwork", "download_lyrics", "download_info"]
            .map(|key| read_pref(key).as_bool().unwrap_or(true));
        self.net.run(move || {
            let r = api
                .post("/api/downloads", json!({"track_refs":[reference]}))
                .and_then(|value| {
                    details::save_extras(&api, &track, extras[0], extras[1], extras[2])
                        .map(|_| value)
                });
            let _ = tx.send_blocking(Event::Notice(r, "Трек добавлен в загрузки".into()));
        });
    }
    fn report(&self, kind: &str) {
        if self.test || !self.settings.record_history.get() {
            return;
        }
        let Some(t) = self.current.borrow().clone() else {
            return;
        };
        let pos = self.player.number("time-pos").max(0.);
        let duration = self.player.number("duration").max(0.);
        let api = self.api.clone();
        let context = self.wave.borrow().as_ref().map(|s| format!("wave:{s}"));
        let body = json!({"events":[{"event_id":unique(),"ts":glib::DateTime::now_utc().unwrap().format_iso8601().unwrap().to_string(),"kind":kind,"ref":t.reference,"nd_song_id":t.nd_song_id,"playback_id":self.playback.borrow().clone(),"device_id":"homemusic-native-local","listened_ms":(self.listened.get()*1000.)as u64,"position_s":pos,"duration_s":duration,"client":"native/0.1","context":context,"track_metadata":{"title":t.title,"artist_credit":t.artist,"artists":[{"name":t.artist}],"album":t.album_title,"duration_s":t.duration_s}}]});
        self.net.run(move || {
            let _ = api.post("/api/plays/events", body);
        });
    }
    fn load_more(self: &Rc<Self>) {
        let Some(offset) = self.more_offset.get() else {
            return;
        };
        let page = self.page.borrow().clone();
        let Some(query) = page.strip_prefix("search:") else {
            return;
        };
        let query = query.to_string();
        let generation = self.request.get();
        let api = self.api.clone();
        let tx = self.tx.clone();
        self.more_button.set_sensitive(false);
        self.net.run(move || {
            let result = api.query(
                "/api/search/tracks",
                &[("q", &query), ("offset", &offset.to_string())],
            );
            let _ = tx.send_blocking(Event::More(generation, result));
        });
    }
    fn toggle_play(self: &Rc<Self>) {
        if self.current.borrow().is_none() {
            if self.queue.borrow().is_empty() {
                self.start_wave();
            } else {
                self.play_index(0);
            }
        } else if let Some(paused) = self.resolving_pause.get() {
            self.resolving_pause.set(Some(!paused));
        } else if self.player.flag("idle-active") {
            self.open_queue_entry(self.index.get(), self.restore_position.get(), false);
        } else {
            self.player.toggle();
        }
        self.update_play_control();
    }

    fn update_play_control(&self) {
        let requested_play = self.resolving_pause.get().map_or_else(
            || !self.player.flag("idle-active") && !self.player.flag("pause"),
            |paused| !paused,
        );
        let busy = self.resolving_pause.get().is_some()
            || self.wave_start_request.get() == Some(self.request.get());
        let icon = if requested_play {
            "media-playback-pause-symbolic"
        } else {
            "media-playback-start-symbolic"
        };
        set_md_icon(&self.play_button, icon);
        if busy {
            self.play_button.add_css_class("buffering");
        } else {
            self.play_button.remove_css_class("buffering");
        }
        let label = if requested_play { "Пауза" } else { "Воспроизвести" };
        if self.play_button.tooltip_text().as_deref() != Some(label) {
            set_action_label(&self.play_button, label);
        }
    }
    fn update_panels(&self) {
        self.apply_panel_widths();
        let compact = self.compact_layout();
        let right = self.show_details.get().unwrap_or(!compact);
        let left = self.sidebar_requested.get() && !(compact && right);
        self.updating_panels.set(true);
        self.library_toggle.set_active(left);
        self.updating_panels.set(false);
        for (revealer, visible) in [
            (&self.sidebar_revealer, left),
            (&self.panel_revealer, right),
        ] {
            if revealer.reveals_child() != visible {
                motion::reveal(revealer, visible, self.motion_mode.get());
            }
        }
        self.update_panel_button();
    }
    fn tick(self: &Rc<Self>) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if self
            .personal_refresh_due
            .get()
            .is_some_and(|due| due <= now)
        {
            self.personal_refresh_due.set(None);
            let api = self.api.clone();
            let tx = self.tx.clone();
            self.net.run(move || {
                let _ = tx.send_blocking(Event::PersonalMixes(api.get("/api/mixes/personal")));
            });
        }
        let volume_width = ((self.window.width() as f64 * 0.085).round() as i32).clamp(100, 180);
        if self.volume_control.width_request() != volume_width {
            self.volume_control.set_width_request(volume_width);
        }
        self.flush_deferred_covers();
        let width = if self.window.width() < 1000 { 320 } else { 480 };
        if self.search.width_request() != width {
            self.search.set_width_request(width);
        }
        self.update_panels();
        if let Some(position) = self.restore_position.get() {
            let duration = self.player.number("duration");
            if !self.player.flag("idle-active") && duration > 0. {
                self.update_timeline_duration(duration);
                self.seek_to(position);
            }
        }
        if self.refill_after.get() <= Instant::now()
            && self
                .queue
                .borrow()
                .len()
                .saturating_sub(self.index.get() + 1)
                < 15
        {
            let sid = self.wave.borrow().clone();
            if let Some(sid) = sid {
                self.continue_session(&sid);
            }
        }
        let elapsed = self
            .last_tick
            .replace(Instant::now())
            .elapsed()
            .as_secs_f64()
            .min(1.);
        if let Some(m) = self.mpris.borrow().as_ref() {
            m.publish(self);
        }
        if self.current.borrow().is_none() {
            return;
        }
        let time = self.player.number("time-pos").max(0.);
        let dur = self.player.number("duration").max(0.);
        if self.player.take_end() {
            self.next(true);
            return;
        }
        let paused = self.player.flag("pause");
        let active = !paused && !self.player.flag("idle-active") && time > 0.;
        self.playing.set(active);
        self.update_timeline_duration(dur);
        self.show_timeline();
        self.refresh_track_rows();
        if self.player.take_failure() {
            self.notice("Ошибка воспроизведения. Нажмите Play, чтобы повторить", true);
        }
        self.update_play_control();
        if active {
            self.listened.set(self.listened.get() + elapsed);
            if !self.started.replace(true) {
                let t = self.current.borrow().clone();
                if let Some(t) = t {
                    let mut history = self.played_history.borrow_mut();
                    history.push_front(t);
                    history.truncate(100);
                }
                self.report("start");
            }
            let n = (self.listened.get() / 15.) as u64;
            if n > self.last_progress.get() {
                self.last_progress.set(n);
                self.report("progress");
            }
        }
    }
    fn handle(self: &Rc<Self>, event: Event) {
        match event {
            Event::NowLyrics(generation, lines) => {
                self.now_lyrics.loaded(generation, lines, self.player.position().unwrap_or(0.));
            }
            Event::Continuation(generation, session, result) => {
                if generation != self.continuation_generation.get() {
                    return;
                }
                let active = self.wave.borrow().clone();
                if session.is_empty() || active.as_deref() == Some(session.as_str()) {
                    self.receive_continuation(result);
                } else {
                    self.autoplay_pending.set(false);
                    self.wave_waiting.set(false);
                }
            }
            Event::CollectionCache(route, revision, result) => {
                let active = self.page.borrow().as_str() == route;
                let mut cache = self.collection_cache.borrow_mut();
                let entry = cache.entry(route).or_default();
                if entry.revision != revision {
                    return;
                }
                entry.fetching = false;
                let had_value = entry.value.is_some();
                let first_result = entry.received.is_none();
                let changed = match &result {
                    Ok(value) => {
                        entry.received = Some(Instant::now());
                        let changed = entry.value.as_ref() != Some(value);
                        if changed {
                            entry.value = Some(value.clone());
                        }
                        changed
                    }
                    Err(_) => true,
                };
                drop(cache);
                if (first_result || changed) && let Ok(value) = &result
                    && let Some(path) = collection_disk::path(&self.api, route)
                {
                    let value = value.clone();
                    self.collection_writer.run(move || { let _ = collection_disk::save(&path, &value); });
                }
                if active && changed {
                    if had_value { self.remember_list_anchor(); }
                    self.handle(Event::Page(self.request.get(), route.to_owned(), result));
                } else if route == "likes" && changed && let Ok(value) = result {
                    self.refresh_home_field("daily", json!(api::tracks(&value["songs"]).into_iter().take(8).collect::<Vec<_>>()));
                }
            }
            Event::EnrichedPage(generation, route, local, value) => {
                if self.request.get() == generation && self.page.borrow().as_str() == route {
                    self.remember_list_anchor();
                    self.handle(Event::Page(generation, route, Ok(value)));
                } else if let Some((_, cached)) = self.cache.borrow_mut().iter_mut().find(|(key, _)| key == &route)
                    && *cached == local
                {
                    *cached = value;
                }
            }
            Event::MixArtwork(tracks) => {
                for (p, t) in self.mix_pictures.iter().zip(tracks.iter()) {
                    self.load_cover(&t.cover(), p);
                }
                let value = json!(tracks);
                *self.home_mix_art.borrow_mut() = value.clone();
                self.refresh_home_field("mix_art", value);
            }
            Event::Recent(result) => {
                if let Ok(v) = result {
                    *self.played_history.borrow_mut() =
                        api::tracks(&v["tracks"]).into_iter().take(100).collect();
                    if self.panel_stack.visible_child_name().as_deref() == Some("history") {
                        self.fill_history_panel();
                    }
                }
            }
            Event::PlaylistChanged(path, result, deleted) => match result {
                Err(e) => self.notice(&e, true),
                Ok(_) => {
                    self.cache
                        .borrow_mut()
                        .retain(|(k, _)| k != &path && k != "playlists");
                    self.load_sidebar();
                    if *self.page.borrow() == path {
                        if deleted {
                            self.navigate("playlists", true)
                        } else {
                            self.navigate(&path, true)
                        }
                    }
                }
            },
            Event::PersonalMixes(result) => {
                if let Ok(value) = result {
                    self.personal_refresh_due
                        .set(value["next_refresh_at"].as_f64().map(|n| n as u64 + 1));
                    *self.personal_mixes.borrow_mut() = value;
                    self.fill_personal_sidebar();
                    if let Some(value) = self
                        .cache
                        .borrow()
                        .iter()
                        .find(|(key, _)| key == "home")
                        .map(|(_, v)| v.clone())
                    {
                        self.fill_home(&value);
                    }
                    let page = self.page.borrow().clone();
                    if page.starts_with("personal:") {
                        self.navigate(&page, false);
                    }
                } else {
                    self.personal_refresh_due.set(Some(
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs()
                            + 60,
                    ));
                }
            }
            Event::Sidebar(result) => {
                if let Ok(v) = result {
                    self.fill_sidebar(&v);
                }
            }
            Event::More(generation, result) => {
                if generation != self.request.get() {
                    return;
                }
                self.more_button.set_sensitive(true);
                match result {
                    Err(e) => self.notice(&e, true),
                    Ok(v) => {
                        let mut items = self.source_items.borrow_mut();
                        for track in api::tracks(&v["tracks"]) {
                            if !items.iter().any(|i| matches!(i, Item::Track(t) if t.reference == track.reference)) {
                                items.push(Item::Track(self.pinned(track)));
                            }
                        }
                        drop(items);
                        self.render_items();
                        self.more_offset.set(v["next_offset"].as_u64());
                        self.more_button
                            .set_visible(self.more_offset.get().is_some());
                        self.notice(
                            &format!("Показано {} треков", self.items.borrow().len()),
                            false,
                        );
                    }
                }
            }
            Event::Stars(Ok(v)) => {
                self.stars_loaded.set(true);
                *self.favorites_snapshot.borrow_mut() = v.clone();
                *self.stars.borrow_mut() = pins::favorite_refs(&v);
            }
            Event::Stars(Err(_)) => {}
            Event::Favorite(reference, liked, result) => {
                self.favorite_pending.borrow_mut().remove(&reference);
                match result {
                    Err(e) => self.notice(&e, true),
                    Ok(_) => {
                        let mut aliases = vec![reference.clone()];
                        for track in self
                            .queue
                            .borrow()
                            .iter()
                            .chain(self.current.borrow().iter())
                        {
                            if track.reference == reference {
                                if let Some(id) = &track.nd_song_id {
                                    aliases.push(format!("nd:{id}"));
                                }
                            }
                        }
                        for item in self.items.borrow().iter() {
                            if let Item::Track(track) = item {
                                if track.reference == reference {
                                    if let Some(id) = &track.nd_song_id {
                                        aliases.push(format!("nd:{id}"));
                                    }
                                }
                            }
                        }
                        for alias in aliases {
                            if liked {
                                self.stars.borrow_mut().insert(alias);
                            } else {
                                self.stars.borrow_mut().remove(&alias);
                            }
                        }
                        self.invalidate_collection("likes");
                        if let Some(path) = collection_disk::path(&self.api, "likes") {
                            self.collection_writer.run(move || collection_disk::forget(&path));
                        }
                        self.load_collection("likes", true);
                        for track in self.queue.borrow_mut().iter_mut() {
                            if track.reference == reference {
                                track.liked = liked;
                            }
                        }
                        for item in self.source_items.borrow_mut().iter_mut() {
                            if let Item::Track(track) = item {
                                if track.reference == reference {
                                    track.liked = liked;
                                }
                            }
                        }
                        for (index, item) in self.items.borrow_mut().iter_mut().enumerate() {
                            if let Item::Track(track) = item {
                                if track.reference == reference {
                                    track.liked = liked;
                                    self.model.splice(
                                        index as u32,
                                        1,
                                        &[glib::BoxedAnyObject::new(Item::Track(track.clone()))],
                                    );
                                }
                            }
                        }
                        if let Some(track) = self.current.borrow_mut().as_mut() {
                            if track.reference == reference {
                                track.liked = liked;
                            }
                        }
                        self.notice(
                            if liked {
                                "Добавлено в любимые"
                            } else {
                                "Убрано из любимого"
                            },
                            false,
                        );
                    }
                }
                self.update_like_controls();
            }
            Event::Login(result) => self.connection_result(result),
            Event::Page(generation, kind, result) => {
                if generation != self.request.get() {
                    return;
                }
                if self.wave_start_request.get() == Some(generation) {
                    self.wave_start_request.set(None);
                    self.update_play_control();
                }
                match result {
                    Err(e) => self.notice(&e, true),
                    Ok(mut v) => {
                        if kind == "home" {
                            if let Some(likes) = self.collection_cache.borrow().get("likes").and_then(|entry| entry.value.as_ref()) {
                                v["daily"] = json!(api::tracks(&likes["songs"]).into_iter().take(8).collect::<Vec<_>>());
                            }
                            if v["mix_art"].is_null() {
                                v["mix_art"] = self.home_mix_art.borrow().clone();
                            }
                        }
                        if (!kind.starts_with("wave") || kind.starts_with("wave_preview:"))
                            && !matches!(kind.as_str(), "likes" | "downloads" | "download_jobs")
                        {
                            let mut cache = self.cache.borrow_mut();
                            cache.retain(|(k, _)| k != &kind);
                            cache.push_back((kind.clone(), v.clone()));
                            while cache.len() > 6 {
                                cache.pop_front();
                            }
                        }
                        let mut items = Vec::new();
                        if kind.starts_with("/api/catalog/artist/")
                            || kind.starts_with("/api/nd/artists/")
                        {
                            self.title.add_css_class("media-link");
                            self.title.set_cursor_from_name(Some("pointer"));
                            self.fill_artist(&v);
                            self.artist_view.set_visible(true);
                            self.content_list.set_visible(false);
                            self.page_tools.set_visible(false);
                            self.title.set_text(&text(&v["artist"], "name"));
                            self.subtitle.set_text(&format!(
                                "{} релизов",
                                ["albums", "singles", "compilations"]
                                    .iter()
                                    .map(|key| v[*key].as_array().map(Vec::len).unwrap_or(0))
                                    .sum::<usize>()
                            ));
                            if let Some(url) = v["artist"]["picture_url"]
                                .as_str()
                                .or(v["artist"]["cover_url"].as_str())
                            {
                                self.page_cover.set_visible(true);
                                self.load_cover(url, &self.page_cover);
                            }
                            motion::page(&self.artist_view, self.motion_mode.get());
                            self.notice("", false);
                            return;
                        }

                        if kind == "home" {
                            self.fill_home(&v);
                        } else if kind.starts_with("/api/catalog/artist/") {
                            self.title.set_text(&text(&v["artist"], "name"));
                            items = api::tracks(&v["top_tracks"])
                                .into_iter()
                                .map(Item::Track)
                                .collect();
                            for x in v["albums"].as_array().unwrap_or(&vec![]) {
                                items.push(Item::Card {
                                    title: text(x, "title"),
                                    sub: text(x, "artist"),
                                    path: format!("/api/catalog/album/{}", text(x, "ref")),
                                    cover: x["cover_url"]
                                        .as_str()
                                        .or(x["picture_url"].as_str())
                                        .map(String::from)
                                        .or_else(|| {
                                            x["cover_art"]
                                                .as_str()
                                                .map(|id| format!("/api/nd/cover/{id}?size=96"))
                                        })
                                        .unwrap_or_default(),
                                });
                            }
                        } else if kind == "artists" || kind.starts_with("/api/nd/artists/") {
                            let values = if kind == "artists" { &v } else { &v["albums"] };
                            if kind != "artists" {
                                self.title.set_text(&text(&v["artist"], "name"));
                            }
                            for x in values.as_array().unwrap_or(&vec![]) {
                                let artist = kind == "artists";
                                items.push(Item::Card {
                                    title: text(x, if artist { "name" } else { "title" }),
                                    sub: if artist {
                                        format!(
                                            "{} альбомов",
                                            x["album_count"].as_u64().unwrap_or(0)
                                        )
                                    } else {
                                        text(x, "artist")
                                    },
                                    path: format!(
                                        "/api/nd/{}/{}",
                                        if artist { "artists" } else { "albums" },
                                        text(x, "id")
                                    ),
                                    cover: x["cover_url"]
                                        .as_str()
                                        .or(x["picture_url"].as_str())
                                        .map(String::from)
                                        .or_else(|| {
                                            x["cover_art"]
                                                .as_str()
                                                .map(|id| format!("/api/nd/cover/{id}?size=96"))
                                        })
                                        .unwrap_or_default(),
                                });
                            }
                        } else if kind == "library" || kind == "playlists" {
                            if kind == "playlists" {
                                items.push(Item::Card {
                                    title: "Любимые треки".into(),
                                    sub: "Актуальное избранное".into(),
                                    path: "likes".into(),
                                    cover: String::new(),
                                });
                            }
                            for x in v.as_array().unwrap_or(&vec![]) {
                                let playlist = kind == "playlists";
                                items.push(Item::Card {
                                    title: text(x, if playlist { "name" } else { "title" }),
                                    sub: if playlist {
                                        format!("{} треков", x["song_count"].as_u64().unwrap_or(0))
                                    } else {
                                        text(x, "artist")
                                    },
                                    path: format!(
                                        "{}/{}",
                                        if playlist {
                                            "/api/playlists"
                                        } else {
                                            "/api/nd/albums"
                                        },
                                        text(x, "id")
                                    ),
                                    cover: x["cover_url"]
                                        .as_str()
                                        .or(x["picture_url"].as_str())
                                        .map(String::from)
                                        .or_else(|| {
                                            x["cover_art"]
                                                .as_str()
                                                .map(|id| format!("/api/nd/cover/{id}?size=96"))
                                        })
                                        .unwrap_or_default(),
                                });
                            }
                        } else if kind == "downloads" {
                            items = downloads::tracks(&v).into_iter().map(Item::Track).collect();
                            self.subtitle.set_text(&format!("{} треков", items.len()));
                            if let Some(cover) = items.iter().find_map(|item| {
                                if let Item::Track(track) = item {
                                    let cover = track.cover();
                                    (!cover.is_empty()).then_some(cover)
                                } else {
                                    None
                                }
                            }) {
                                self.page_cover.set_visible(true);
                                self.load_cover(&cover, &self.page_cover);
                            }
                        } else if kind == "download_jobs" {
                            items = downloads::rows(&v);
                            self.subtitle.set_text("");
                        } else {
                            let value = if kind == "likes" || kind == "likes_play" {
                                &v["songs"]
                            } else if kind == "home" || kind == "mix" || kind == "mix_play" {
                                &v["mix"]["tracks"]
                            } else {
                                &v["tracks"]
                            };
                            items = api::tracks(value).into_iter().map(Item::Track).collect();
                        }
                        for item in &mut items {
                            if let Item::Track(track) = item {
                                track.liked = kind == "likes"
                                    || kind == "likes_play"
                                    || self.stars.borrow().contains(&track.reference)
                                    || track.nd_song_id.as_ref().is_some_and(|id| {
                                        self.stars.borrow().contains(&format!("nd:{id}"))
                                    });
                            }
                        }
                        self.more_offset.set(if kind.starts_with("search:") {
                            v["tracks_next"].as_u64()
                        } else {
                            None
                        });
                        self.more_button
                            .set_visible(self.more_offset.get().is_some());
                        self.more_button.set_sensitive(true);
                        if kind.starts_with('/') {
                            if Self::is_album(&kind) {
                                self.title.add_css_class("media-link");
                                self.subtitle.add_css_class("media-link");
                                self.title.set_cursor_from_name(Some("pointer"));
                                self.subtitle.set_cursor_from_name(Some("pointer"));
                                *self.album_artist.borrow_mut() = Some(Track {
                                    artist: text(&v, "artist"),
                                    artist_ref: v["artist_ref"].as_str().map(String::from),
                                    nd_artist_id: v["artist_id"].as_str().map(String::from),
                                    ..Track::default()
                                });
                                *self.album_info.borrow_mut() =
                                    Some((text(&v, "title"), text(&v, "artist")));
                            }
                            let title = text(&v, "title");
                            let name = text(&v, "name");
                            if !title.is_empty() || !name.is_empty() {
                                self.title
                                    .set_text(if title.is_empty() { &name } else { &title });
                            }
                            let playlist = kind.starts_with("/api/playlists/");
                            self.subtitle.set_text(&format!(
                                "{} · {} треков",
                                if playlist { "Плейлист".into() } else { text(&v, "artist") },
                                items.len()
                            ));
                            let cover = v["cover_url"].as_str().map(str::to_owned).or_else(|| {
                                if playlist {
                                    items.iter().find_map(|item| match item {
                                        Item::Track(track) if !track.cover().is_empty() => Some(track.cover()),
                                        _ => None,
                                    })
                                } else {
                                    None
                                }
                            });
                            if let Some(cover) = cover {
                                self.page_cover.set_visible(true);
                                self.load_cover(&cover, &self.page_cover);
                            }
                        }
                        if kind.starts_with("search:") {
                            if self.search_scope.selected() > 0 {
                                items = v["candidates"]
                                    .as_array()
                                    .unwrap_or(&vec![])
                                    .iter()
                                    .map(|c| {
                                        Item::Track(Track {
                                            reference: format!(
                                                "file:{}:{}",
                                                text(c, "provider"),
                                                text(c, "provider_id")
                                            ),
                                            title: text(c, "title"),
                                            artist: text(c, "artist"),
                                            duration_s: c["duration_s"].as_f64(),
                                            cover_url: c["cover_url"].as_str().map(String::from),
                                            ..Track::default()
                                        })
                                    })
                                    .collect();
                            } else {
                                let library = api::tracks(&v["library"]);
                                if self.search_kind.selected() == 4 {
                                    items.clear();
                                }
                                for t in library.into_iter().rev() {
                                    if !items.iter().any(|i| matches!(i,Item::Track(x) if x.reference == t.reference)) {
                                        items.insert(0,Item::Track(t));
                                    }
                                }
                            }
                            for (field, route) in [("albums", "album"), ("artists", "artist")] {
                                for x in v[field].as_array().unwrap_or(&vec![]) {
                                    items.push(Item::Card {
                                        title: x["title"]
                                            .as_str()
                                            .or(x["name"].as_str())
                                            .unwrap_or("")
                                            .into(),
                                        sub: if field == "artists" {
                                            "Исполнитель".into()
                                        } else {
                                            [year_text(x), text(x, "artist")]
                                                .into_iter()
                                                .filter(|s| !s.is_empty())
                                                .collect::<Vec<_>>()
                                                .join(" · ")
                                        },
                                        path: format!("/api/catalog/{route}/{}", text(x, "ref")),
                                        cover: x["cover_url"]
                                            .as_str()
                                            .or(x["picture_url"].as_str())
                                            .map(String::from)
                                            .or_else(|| {
                                                x["cover_art"]
                                                    .as_str()
                                                    .map(|id| format!("/api/nd/cover/{id}?size=96"))
                                            })
                                            .unwrap_or_default(),
                                    });
                                }
                            }
                        }
                        if kind.starts_with("search:") && self.search_scope.selected() == 0 {
                            match self.search_kind.selected() {
                                1 | 4 => items.retain(|i| matches!(i,Item::Track(_))),
                                2 => items.retain(|i| matches!(i,Item::Card{path,..} if path.contains("/album/"))),
                                3 => items.retain(|i| matches!(i,Item::Card{path,..} if path.contains("/artist/"))),
                                _ => (),
                            }
                            let errors = v["errors"]
                                .as_array()
                                .map(|e| {
                                    e.iter()
                                        .filter_map(Value::as_str)
                                        .collect::<Vec<_>>()
                                        .join(" · ")
                                })
                                .unwrap_or_default();
                            if !errors.is_empty() {
                                self.subtitle.set_text(&errors);
                            }
                        }
                        let n = items.len();
                        if kind == "likes" { self.subtitle.set_text(&format!("{n} треков")); }
                        self.set_items(items);
                        if kind.starts_with("search:") {
                            self.fill_search();
                        } else if Self::is_collection(&kind) {
                            self.fill_card_page();
                        }
                        match kind.as_str() {
                            "home" => motion::page(&self.home_scroll, self.motion_mode.get()),
                            _ if self.artist_view.is_visible() => motion::page(&self.artist_view, self.motion_mode.get()),
                            _ => motion::page(&self.content_list, self.motion_mode.get()),
                        }
                        if kind.starts_with("wave_preview:") {
                            *self.preview_wave.borrow_mut() =
                                v["session_id"].as_str().map(String::from);
                            self.notice("", false);
                            return;
                        }
                        if kind == "mix_play" || kind == "likes_play" {
                            self.invalidate_continuation();
                            let tracks: Vec<_> = self
                                .items
                                .borrow()
                                .iter()
                                .filter_map(|i| {
                                    if let Item::Track(t) = i {
                                        Some(t.clone())
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                            *self.wave.borrow_mut() = None;
                            self.user_queue_end.set(0);
                            *self.queue_original.borrow_mut() = tracks.clone();
                            *self.queue.borrow_mut() = tracks;
                            self.play_index(0);
                            return;
                        }
                        if kind == "mix" {
                            if v["onboarding"].as_bool() == Some(true) {
                                self.subtitle.set_text(
                                    "Перенесите любимую музыку — и волна узнает ваш вкус",
                                );
                            } else if let Some(t) = v["mix"]["title"].as_str() {
                                self.subtitle.set_text(t);
                            }
                        }
                        if kind.starts_with("wave") {
                            self.invalidate_continuation();
                            self.autoplay_pending.set(false);
                            if let Some(sid) = v["session_id"].as_str() {
                                *self.wave.borrow_mut() = Some(sid.into());
                            }
                            let q: Vec<_> = self
                                .items
                                .borrow()
                                .iter()
                                .filter_map(|i| {
                                    if let Item::Track(t) = i {
                                        Some(t.clone())
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                            if !q.is_empty() {
                                if kind == "wave_autoplay" || kind == "wave_next" {
                                    let previous =
                                        self.current.borrow().as_ref().map(|t| t.reference.clone());
                                    let fresh: Vec<_> = q
                                        .into_iter()
                                        .filter(|t| Some(&t.reference) != previous.as_ref())
                                        .collect();
                                    if fresh.is_empty() {
                                        self.notice("Волна пока не нашла продолжение", false);
                                        return;
                                    }
                                    let next = self.queue.borrow().len();
                                    self.queue.borrow_mut().extend(fresh);
                                    self.play_index(next);
                                } else {
                                    self.user_queue_end.set(0);
                                    *self.queue.borrow_mut() = q;
                                    self.play_index(0);
                                }
                                return;
                            }
                        }
                        self.notice(if n == 0 { "" } else { "" }, false);
                    }
                }
            }
            Event::Resolved(generation, result) => {
                if generation != self.resolve.get() {
                    return;
                }
                let paused = self.resolving_pause.take().unwrap_or(false);
                match result {
                    Err(e) => self.notice(&e, true),
                    Ok(v) => {
                        if let Some(path) = v["stream_url"].as_str() {
                            let mut url = format!("{}{path}", self.api.base);
                            let require_full = self.current.borrow().as_ref()
                                .is_some_and(|track| track.require_full_audio);
                            if require_full && path.starts_with("/api/listen/stream?") {
                                url.push_str("&full=1");
                            }
                            if path.starts_with("/api/listen/stream?")
                                && read_pref("audio_quality") == "mobile"
                            {
                                url.push_str("&q=mobile");
                            }
                            let buffer = read_pref("audio_buffer").as_u64().unwrap_or(0);
                            if self.player.load(&url, &self.api.cookie(), paused, buffer) {
                                self.notice(
                                    if v["kind"] == "preview" && !require_full { "Доступно превью 30 секунд" } else { "" },
                                    false,
                                );
                                self.source_label.set_visible(true);
                                self.source_label.set_text(&format!(
                                    "{}{}",
                                    v["provider"].as_str().unwrap_or(if v["kind"] == "library" {
                                        "Локальная библиотека"
                                    } else {
                                        "HomeMusic"
                                    }),
                                    v["bitrate_kbps"]
                                        .as_u64()
                                        .map(|n| format!(" · {n} кбит/с"))
                                        .unwrap_or_default()
                                ));
                                let refs: Vec<_> = self
                                    .queue
                                    .borrow()
                                    .iter()
                                    .skip(self.index.get() + 1)
                                    .take(2)
                                    .map(|t| t.reference.clone())
                                    .collect();
                                if !refs.is_empty() {
                                    let api = self.api.clone();
                                    self.net.run(move || {
                                        let _ =
                                            api.post("/api/listen/prefetch", json!({"refs":refs}));
                                    });
                                }
                            } else {
                                self.notice("Аудиоплеер не смог открыть поток. Нажмите Play, чтобы повторить", true);
                            }
                        } else {
                            self.notice("Сервер не вернул аудиопоток. Нажмите Play, чтобы повторить", true);
                        }
                    }
                }
                self.update_play_control();
            }
            Event::Notice(r, text) => match r {
                Ok(_) => self.notice(&text, false),
                Err(e) => self.notice(&e, true),
            },
            Event::ArtistMetadata(generation, result) => {
                if generation != self.request.get() {
                    return;
                }
                match result {
                    Ok(route) => self.navigate(&route, false),
                    Err(message) => self.notice(&message, true),
                }
            }
            Event::AlbumMetadata(generation, result) => {
                if generation != self.request.get() {
                    return;
                }
                match result {
                    Ok(value) => {
                        let track = api::tracks(&json!([value]))
                            .into_iter()
                            .next()
                            .unwrap_or_default();
                        if track.nd_album_id.as_deref().is_some_and(|s| !s.is_empty())
                            || track.album_ref.as_deref().is_some_and(|s| !s.is_empty())
                        {
                            self.open_album(&track);
                        } else {
                            self.notice("Альбом этого трека пока неизвестен", false);
                        }
                    }
                    Err(_) => self.notice(
                        "Не удалось получить альбом этой записи. Повторите нажатие позже.",
                        true,
                    ),
                }
            }
            Event::Collection(generation, result) => {
                if generation != self.resolve.get() {
                    return;
                }
                match result {
                    Ok(value) => {
                        let tracks = api::tracks(if value["songs"].is_array() {
                            &value["songs"]
                        } else {
                            &value["tracks"]
                        });
                        if tracks.is_empty() {
                            self.notice("В подборке пока нет доступных треков", false);
                        } else {
                            self.invalidate_continuation();
                            *self.wave.borrow_mut() = None;
                            self.user_queue_end.set(0);
                            *self.queue_original.borrow_mut() = tracks.clone();
                            *self.queue.borrow_mut() = tracks;
                            self.play_index(0);
                        }
                    }
                    Err(error) => self.notice(&error, true),
                }
            }
            Event::Cover(url, size, result) => {
                self.cover_pending.borrow_mut().remove(&url);
                let waiters = self.cover_waiters.borrow_mut().remove(&url);
                if result.is_err() {
                    let retry = {
                        let mut retries = self.cover_retries.borrow_mut();
                        if retries.len() > 256 {
                            retries.clear();
                        }
                        let attempts = retries.entry(url.clone()).or_default();
                        *attempts += 1;
                        *attempts <= 2
                    };
                    if retry {
                        let weak = Rc::downgrade(self);
                        let retry_key = url.clone();
                        glib::timeout_add_local_once(Duration::from_millis(750), move || {
                            if let Some(u) = weak.upgrade() {
                                for weak in waiters.unwrap_or_default() {
                                    if let Some(p) = weak.upgrade() {
                                        let still_bound = u
                                            .cover_bindings
                                            .borrow()
                                            .get(&(p.as_ptr() as usize))
                                            .is_some_and(|(_, key)| key == &retry_key);
                                        if still_bound && p.is_mapped() {
                                            if let Some((_, source)) = retry_key.split_once('|') {
                                                u.load_cover(source, &p);
                                            }
                                        }
                                    }
                                }
                            }
                        });
                    }
                    return;
                }
                if let Ok(bytes) = result {
                    self.cover_retries.borrow_mut().remove(&url);
                    let stream =
                        gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(bytes));
                    if let Ok(pixbuf) = gtk::gdk_pixbuf::Pixbuf::from_stream_at_scale(
                        &stream,
                        size,
                        size,
                        true,
                        gio::Cancellable::NONE,
                    ) {
                        let texture = gdk::Texture::for_pixbuf(&pixbuf);
                        if let Some(waiters) = waiters {
                            for weak in waiters {
                                if let Some(p) = weak.upgrade() {
                                    if self
                                        .cover_bindings
                                        .borrow()
                                        .get(&(p.as_ptr() as usize))
                                        .map(|(_, key)| key)
                                        != Some(&url)
                                    {
                                        continue;
                                    }
                                    self.set_loaded_cover(&p, &texture);
                                }
                            }
                        }
                        let mut cache = self.cover_cache.borrow_mut();
                        cache.push_back((url, texture));
                        while cache.len() > 80 {
                            cache.pop_front();
                        }
                    }
                }
            }
            Event::Import(r) => match r {
                Ok(v) => self.preview_import(v),
                Err(e) => self.notice(&e, true),
            },
        }
    }
    fn defer_cover(self: &Rc<Self>, source: &str, picture: &gtk::Picture) {
        self.cover_bindings.borrow_mut().insert(
            picture.as_ptr() as usize,
            (picture.downgrade(), format!("deferred|{source}")),
        );
        self.deferred_covers.borrow_mut().insert(
            picture.as_ptr() as usize,
            (picture.downgrade(), source.into()),
        );
    }
    fn flush_deferred_covers(self: &Rc<Self>) {
        *self.wanted_covers.lock().unwrap() = self
            .cover_bindings
            .borrow()
            .values()
            .filter_map(|(weak, key)| {
                weak.upgrade()
                    .filter(|p| {
                        p.is_mapped()
                            && p.compute_bounds(&self.window).is_some_and(|b| {
                                b.y() < self.window.height() as f32 && b.y() + b.height() > 0.
                            })
                    })
                    .map(|_| key.clone())
            })
            .collect();
        self.cover_bindings
            .borrow_mut()
            .retain(|_, (weak, _)| weak.upgrade().is_some());
        self.cover_waiters.borrow_mut().retain(|_, waiters| {
            waiters.retain(|weak| weak.upgrade().is_some());
            !waiters.is_empty()
        });
        let mut pending = self.deferred_covers.borrow_mut();
        let mut ready = vec![];
        pending.retain(|_, (weak, source)| {
            let Some(p) = weak.upgrade() else {
                return false;
            };
            let visible = p.is_mapped()
                && p.compute_bounds(&self.window).is_some_and(|b| {
                    b.y() < self.window.height() as f32 + 80. && b.y() + b.height() > -80.
                });
            if visible {
                ready.push((p, source.clone()));
                false
            } else {
                true
            }
        });
        drop(pending);
        for (p, source) in ready {
            self.load_cover(&source, &p);
        }
    }
    fn set_loaded_cover(&self, picture: &gtk::Picture, texture: &gdk::Texture) {
        if picture.paintable().as_ref() == Some(texture.upcast_ref::<gdk::Paintable>()) {
            return;
        }
        picture.set_paintable(Some(texture));
        picture.set_visible(true);
        if picture == &self.mini_picture && read_pref("palette_source").as_str() == Some("cover") {
            theme::update_cover(&self._theme, picture);
            self.timeline.canvas.queue_draw();
        }
        if picture == &self.now_picture || picture == &self.mini_picture {
            motion::page(picture, self.motion_mode.get());
        }
    }

    fn load_cover(self: &Rc<Self>, source: &str, picture: &gtk::Picture) {
        if picture == &self.page_cover {
            self.page_art.borrow_mut().replace_range(.., source);
        }
        if source.is_empty() {
            let key = picture.as_ptr() as usize;
            self.cover_bindings.borrow_mut().remove(&key);
            self.deferred_covers.borrow_mut().remove(&key);
            picture.set_paintable(None::<&gdk::Texture>);
            return;
        }
        let size = if picture.width_request() > 160 {
            384
        } else if picture.width_request() > 64 {
            256
        } else {
            128
        };
        let mut url = source.to_string();
        for old in ["250x250", "1000x1000"] {
            url = url.replace(old, &format!("{size}x{size}"));
        }
        for old in ["size=48", "size=96", "size=300", "size=1000"] {
            url = url.replace(old, &format!("size={size}"));
        }
        let key = format!("{size}|{url}");
        self.cover_bindings.borrow_mut().insert(
            picture.as_ptr() as usize,
            (picture.downgrade(), key.clone()),
        );
        if let Some((_, texture)) = self.cover_cache.borrow().iter().find(|(u, _)| u == &key) {
            self.set_loaded_cover(picture, texture);
            return;
        }
        picture.set_paintable(None::<&gdk::Texture>);
        self.cover_waiters
            .borrow_mut()
            .entry(key.clone())
            .or_default()
            .push(picture.downgrade());
        if !self.cover_pending.borrow_mut().insert(key.clone()) {
            return;
        }
        let api = self.api.clone();
        let tx = self.tx.clone();
        let pool = if size > 256
            || picture == &self.page_cover
            || picture == &self.now_picture
            || picture == &self.mini_picture
        {
            &self.net
        } else {
            &self.images
        };
        let pending_key = key.clone();
        let weak_picture = picture.downgrade();
        let weak_ui = Rc::downgrade(self);
        let original = source.to_string();
        let wanted = self.wanted_covers.clone();
        let visible_request = picture.is_mapped();
        wanted.lock().unwrap().insert(key.clone());
        if !pool.run(move || {
            // Recycled/offscreen rows must not hold up the newly visible viewport.
            let result = if visible_request && !wanted.lock().unwrap().contains(&key) {
                Err("Artwork request no longer visible".into())
            } else {
                api.image(&url)
            };
            let _ = tx.send_blocking(Event::Cover(key, size, result));
        }) {
            self.cover_pending.borrow_mut().remove(&pending_key);
            let weak = weak_ui.clone();
            glib::timeout_add_local_once(Duration::from_millis(250), move || {
                if let (Some(u), Some(p)) = (weak.upgrade(), weak_picture.upgrade()) {
                    let still_bound = u
                        .cover_bindings
                        .borrow()
                        .get(&(p.as_ptr() as usize))
                        .is_some_and(|(_, key)| key == &pending_key);
                    if still_bound {
                        u.defer_cover(&original, &p);
                    }
                }
            });
        }
    }
    fn install_factory(self: &Rc<Self>) {
        let factory = gtk::SignalListItemFactory::new();
        let u = self.clone();
        factory.connect_setup(move |_, obj| {
            let item = obj.downcast_ref::<gtk::ListItem>().unwrap();
            let row = hbox(12);
            let play = icon_button("media-playback-start-symbolic", "Слушать трек", "flat");
            row.append(&play);
            let picture = gtk::Picture::new();
            picture.set_can_shrink(true);
            picture.set_size_request(44, 44);
            picture.set_content_fit(gtk::ContentFit::Cover);
            picture.add_css_class("cover");
            row.append(&cover_frame(&picture, 48));
            let texts = vbox(3);
            texts.set_hexpand(true);
            texts.set_valign(gtk::Align::Center);
            let title = label("", "track-title");
            title.set_ellipsize(gtk::pango::EllipsizeMode::End);
            title.set_max_width_chars(50);
            let weak = item.downgrade();
            let sub = u.metadata_line(move || {
                let object = weak
                    .upgrade()?
                    .item()?
                    .downcast::<glib::BoxedAnyObject>()
                    .ok()?;
                let track = match &*object.borrow::<Item>() {
                    Item::Track(t) => Some(t.clone()),
                    _ => None,
                };
                track
            });
            let weak_item = item.downgrade();
            u.link_track_title(&title, move || {
                let object = weak_item
                    .upgrade()?
                    .item()?
                    .downcast::<glib::BoxedAnyObject>()
                    .ok()?;
                let track = match &*object.borrow::<Item>() {
                    Item::Track(t) => Some(t.clone()),
                    _ => None,
                };
                track
            });
            texts.append(&title);
            texts.append(&sub);
            row.append(&texts);
            row.append(&label("", "small muted"));
            let like = icon_button("emblem-favorite-symbolic", "Добавить в любимое", "flat");
            row.append(&like);
            let download = icon_button("folder-download-symbolic", "Скачать трек", "flat");
            row.append(&download);
            let gesture = gtk::GestureClick::new();
            gesture.set_button(3);
            let weak = item.downgrade();
            let ui = u.clone();
            gesture.connect_pressed(move |gesture, _, x, y| {
                if let Some(item) = weak.upgrade() {
                    ui.context_menu(&item, gesture.widget().as_ref().unwrap(), x, y);
                }
            });
            row.add_controller(gesture);
            let drag = gtk::DragSource::new();
            drag.set_actions(gdk::DragAction::MOVE);
            let weak = item.downgrade();
            let source_ui = u.clone();
            drag.connect_prepare(move |source, _, _| {
                // A pending gesture can outlive a row detached by link navigation.
                let _surface=source.widget()?.native()?.surface()?;
                let item = weak.upgrade()?;
                let path = source_ui.page.borrow().clone();
                if !path.starts_with("/api/playlists/") || source_ui.sort.selected() != 0 {
                    return None;
                }
                Some(gdk::ContentProvider::for_value(
                    &format!("playlist|{}|{}", path, item.position()).to_value(),
                ))
            });
            row.add_controller(drag);
            let drop = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);
            let weak = item.downgrade();
            let ui = u.clone();
            drop.connect_drop(move |_, v, _, _| {
                let Ok(s) = v.get::<String>() else {
                    return false;
                };
                let Some((source, position)) =
                    s.strip_prefix("playlist|").and_then(|s| s.rsplit_once('|'))
                else {
                    return false;
                };
                if source != ui.page.borrow().as_str() {
                    return false;
                }
                let Ok(from) = position.parse() else {
                    return false;
                };
                weak.upgrade()
                    .is_some_and(|i| ui.reorder_playlist(from, i.position() as usize))
            });
            row.add_controller(drop);

            u.active_rows.borrow_mut().push(item.downgrade());
            item.set_child(Some(&row));
            let weak = item.downgrade();
            let ui = u.clone();
            play.connect_clicked(move |_| {
                if let Some(item) = weak.upgrade() {
                    ui.activate_or_toggle(item.position() as usize);
                }
            });
            let weak = item.downgrade();
            let ui = u.clone();
            like.connect_clicked(move |_| {
                if let Some(item) = weak.upgrade() {
                    if let Some(obj) = item.item().and_downcast::<glib::BoxedAnyObject>() {
                        if let Item::Track(t) = &*obj.borrow::<Item>() {
                            ui.favorite(t, !t.liked);
                        }
                    }
                }
            });
            let weak = item.downgrade();
            let ui = u.clone();
            download.connect_clicked(move |_| {
                if let Some(item) = weak.upgrade() {
                    if let Some(obj) = item.item().and_downcast::<glib::BoxedAnyObject>() {
                        if let Item::Track(t) = &*obj.borrow::<Item>() {
                            ui.download(t);
                        }
                    }
                }
            });
        });
        let u = self.clone();
        factory.connect_bind(move |_, obj| {
            let item = obj.downcast_ref::<gtk::ListItem>().unwrap();
            let Some(value) = item.item().and_downcast::<glib::BoxedAnyObject>() else {
                return;
            };
            let data = value.borrow::<Item>();
            let row = item.child().unwrap();
            let picture = child(&row, 1)
                .downcast::<gtk::Overlay>()
                .unwrap()
                .last_child()
                .unwrap()
                .downcast::<gtk::Picture>()
                .unwrap();
            picture.set_paintable(None::<&gdk::Texture>);
            let texts = child(&row, 2);
            let title = child(&texts, 0).downcast::<gtk::Label>().unwrap();
            let sub = child(&texts, 1).downcast::<gtk::Box>().unwrap();
            let duration = child(&row, 3).downcast::<gtk::Label>().unwrap();
            let like = child(&row, 4);
            let download = child(&row, 5);
            match &*data {
                Item::Track(t) => {
                    let active = u
                        .current
                        .borrow()
                        .as_ref()
                        .is_some_and(|c| c.reference == t.reference);
                    if active {
                        row.add_css_class("playing")
                    } else {
                        row.remove_css_class("playing")
                    }
                    if let Ok(b) = child(&row, 0).downcast::<gtk::Button>() {
                        set_md_icon(
                            &b,
                            if active && u.playing.get() {
                                "media-playback-pause-symbolic"
                            } else {
                                "media-playback-start-symbolic"
                            },
                        );
                    }
                    title.set_text(&t.title);
                    let page = u.page.borrow();
                    let album = if Self::is_album(&page) || page.starts_with("wave") {
                        ""
                    } else {
                        t.album_title.as_deref().unwrap_or("")
                    };
                    set_metadata_line(&sub, &t.artist, album);
                    duration.set_text(
                        &t.duration_s
                            .filter(|n| *n > 0.)
                            .map(time_text)
                            .unwrap_or_else(|| "—".into()),
                    );
                    like.set_visible(true);
                    like.set_tooltip_text(Some(if t.liked {
                        "Убрать из любимого"
                    } else {
                        "Добавить в любимое"
                    }));
                    set_like_icon(&like.clone().downcast::<gtk::Button>().unwrap(), t.liked);
                    if t.liked {
                        like.add_css_class("liked");
                    } else {
                        like.remove_css_class("liked");
                    }
                    download.set_visible(false);
                    u.defer_cover(&t.cover(), &picture);
                }
                Item::Card {
                    title: t,
                    sub: s,
                    cover,
                    ..
                } => {
                    title.set_text(t);
                    set_metadata_line(&sub, s, "");
                    duration.set_text("");
                    like.set_visible(false);
                    download.set_visible(false);
                    u.defer_cover(cover, &picture);
                }
            }
        });
        let u = self.clone();
        factory.connect_unbind(move |_, obj| {
            let item = obj.downcast_ref::<gtk::ListItem>().unwrap();
            if let Some(row) = item.child() {
                let picture = child(&row, 1)
                    .downcast::<gtk::Overlay>()
                    .unwrap()
                    .last_child()
                    .unwrap()
                    .downcast::<gtk::Picture>()
                    .unwrap();
                u.cover_bindings
                    .borrow_mut()
                    .remove(&(picture.as_ptr() as usize));
                child(&row, 1)
                    .downcast::<gtk::Overlay>()
                    .unwrap()
                    .last_child()
                    .unwrap()
                    .downcast::<gtk::Picture>()
                    .unwrap()
                    .set_paintable(None::<&gdk::Texture>);
            }
        });
        self.list.set_factory(Some(&factory));
    }
    fn import_dialog(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder()
            .title("Экспорт Spotify: ZIP, CSV или JSON")
            .build();
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Экспорт Spotify"));
        for ext in ["*.zip", "*.csv", "*.json"] {
            filter.add_pattern(ext);
        }
        dialog.set_default_filter(Some(&filter));
        let dir = gio::File::for_path(crate::config::root().join("spotify-inbox"));
        dialog.set_initial_folder(Some(&dir));
        let u = self.clone();
        dialog.open(Some(&self.window), gio::Cancellable::NONE, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    u.notice("Читаем экспорт Spotify…", false);
                    let api = u.api.clone();
                    let tx = u.tx.clone();
                    u.net.run(move || {
                        let r = api.upload(&path);
                        let _ = tx.send_blocking(Event::Import(r));
                    });
                }
            }
        });
    }
    fn preview_import(self: &Rc<Self>, v: Value) {
        let dialog = gtk::Window::builder()
            .title("Перенос из Spotify")
            .transient_for(&self.window)
            .modal(true)
            .default_width(480)
            .build();
        let content = vbox(16);
        content.set_margin_top(24);
        content.set_margin_bottom(24);
        content.set_margin_start(24);
        content.set_margin_end(24);
        content.append(&label("Ваша музыка нашлась", "section-title"));
        let collections = v["collections"].as_array().cloned().unwrap_or_default();
        let text = collections
            .iter()
            .map(|c| {
                format!(
                    "{} · {} треков",
                    self::text(c, "title"),
                    c["count"].as_u64().unwrap_or(0)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let details = label(&text, "");
        details.set_wrap(true);
        let sc = gtk::ScrolledWindow::builder()
            .max_content_height(300)
            .propagate_natural_height(true)
            .child(&details)
            .build();
        content.append(&sc);
        let note = label(
            "Плейлисты и лайки будут перенесены.\nФайлы скачиваются при первом прослушивании.",
            "muted",
        );
        note.set_wrap(true);
        content.append(&note);
        let start = button(
            "Перенести музыку",
            Some("document-import-symbolic"),
            "primary",
        );
        content.append(&start);
        dialog.set_child(Some(&ripple::wrap(&content)));
        let u = self.clone();
        let d = dialog.clone();
        start.connect_clicked(move|_|{let api=u.api.clone();let tx=u.tx.clone();let id=v["id"].as_u64().unwrap_or(0);let ids:Vec<_>=collections.iter().filter_map(|c|c["id"].as_u64()).collect();u.net.run(move||{let r=api.post(&format!("/api/imports/{id}/start"),json!({"collection_ids":ids,"mode":"economy","download_albums":false,"keep_token":false}));let _=tx.send_blocking(Event::Notice(r,"Перенос запущен. Ваши лайки и плейлисты появятся после сопоставления треков.".into()));});d.close();});
        dialog.present();
    }
}

fn label(text: &str, classes: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.);
    for c in classes.split_whitespace() {
        l.add_css_class(c);
    }
    l
}
fn hbox(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Horizontal, spacing)
}
fn vbox(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, spacing)
}
fn button(text: &str, icon: Option<&str>, classes: &str) -> gtk::Button {
    let b = gtk::Button::new();
    if let Some(i) = icon {
        let row = hbox(8);
        row.set_halign(gtk::Align::Center);
        row.append(&symbol(i));
        row.append(&label(text, ""));
        b.set_child(Some(&row));
    } else {
        b.set_label(text);
    }
    for c in classes.split_whitespace() {
        b.add_css_class(c);
    }
    b
}
fn icon_button(icon: &str, tip: &str, classes: &str) -> gtk::Button {
    let b = gtk::Button::new();
    set_md_icon(&b, icon);
    set_action_label(&b, tip);
    for c in classes.split_whitespace() {
        b.add_css_class(c);
    }
    b
}
fn set_action_label(button: &gtk::Button, name: &str) {
    button.set_tooltip_text(Some(name));
    button.update_property(&[gtk::accessible::Property::Label(name)]);
}
fn child(parent: &gtk::Widget, index: usize) -> gtk::Widget {
    let mut w = parent.first_child().unwrap();
    for _ in 0..index {
        w = w.next_sibling().unwrap();
    }
    w
}
fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").into()
}
fn time_text(v: f64) -> String {
    let seconds = if v.is_finite() { v.max(0.) as u64 } else { 0 };
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
fn unique() -> String {
    format!(
        "n{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

fn install_test_control(ui: &Rc<Ui>) {
    // Opt-in test-only IPC; never available in normal launches. No credentials enter this socket.
    use std::{
        io::{BufRead, BufReader, Write},
        os::unix::{fs::PermissionsExt, net::UnixListener},
    };
    let path = crate::config::socket_path();
    let Ok(listener) = UnixListener::bind(&path) else {
        return;
    };
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    let (tx, rx) = async_channel::unbounded::<(Value, mpsc::Sender<Value>)>();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                break;
            };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut line = String::new();
            if BufReader::new(&stream).read_line(&mut line).is_err() {
                continue;
            }
            let Ok(command) = serde_json::from_str(&line) else {
                continue;
            };
            let (rtx, rrx) = mpsc::channel();
            let _ = tx.send_blocking((command, rtx));
            if let Ok(reply) = rrx.recv_timeout(Duration::from_secs(5)) {
                let _ = writeln!(stream, "{reply}");
            }
        }
    });
    let u = ui.clone();
    glib::spawn_future_local(async move {
        while let Ok((cmd, reply)) = rx.recv().await {
            let mut capture = Value::Null;
            match cmd["action"].as_str().unwrap_or("") {
                "screenshot" => {
                    let name = cmd["name"].as_str().unwrap_or("window");
                    capture = match capture_test_window(&u.window, name) {
                        Ok(path) => json!({"path": path}),
                        Err(error) => json!({"error": error}),
                    };
                }
                "search_options" => {
                    u.search_scope
                        .set_selected(cmd["scope"].as_u64().unwrap_or(0) as u32);
                    u.search_kind
                        .set_selected(cmd["kind"].as_u64().unwrap_or(0) as u32);
                }
                "search" => {
                    u.search.set_text(cmd["query"].as_str().unwrap_or(""));
                }
                "clear_fixture_playback" => {
                    u.player.command(&["stop"]);
                    u.playing.set(false);
                    u.current.borrow_mut().take();
                    u.now_lyrics.load(&u, None);
                    u.queue.borrow_mut().clear();
                    u.queue_original.borrow_mut().clear();
                    u.index.set(0);
                    u.user_queue_end.set(0);
                    u.started.set(false);
                    u.reset_timeline(0., 0.);
                }
                "play" => u.activate(cmd["index"].as_u64().unwrap_or(0) as usize),
                "pause" => u.toggle_play(),
                "next" => u.next(false),
                "toggle_play" => u.toggle_play(),
                "enqueue" => {
                    let track = u
                        .items
                        .borrow()
                        .get(cmd["index"].as_u64().unwrap_or(0) as usize)
                        .and_then(|i| {
                            if let Item::Track(t) = i {
                                Some(t.clone())
                            } else {
                                None
                            }
                        });
                    if let Some(t) = track {
                        u.enqueue(t, cmd["next"].as_bool().unwrap_or(false));
                    }
                }
                "scroll_media" => {
                    let scroll = match cmd["area"].as_str().unwrap_or("artist") {
                        "home" => &u.home_scroll,
                        "sidebar" => &u.sidebar_scroll,
                        _ => &u.artist_view,
                    };
                    let a = scroll.vadjustment();
                    a.set_value(
                        (a.upper() - a.page_size()).max(0.)
                            * cmd["fraction"].as_f64().unwrap_or(0.),
                    );
                }
                "save_playback" => u.save_playback(),
                "restore_playback" => u.restore_playback(),
                "close_dialogs" => {
                    for widget in gtk::Window::list_toplevels() {
                        if let Ok(window) = widget.downcast::<gtk::Window>() {
                            if window != u.window.clone().upcast::<gtk::Window>() {
                                window.close();
                            }
                        }
                    }
                }
                "single_queue" => {
                    let current = u.current.borrow().clone();
                    if let Some(t) = current {
                        *u.wave.borrow_mut() = None;
                        u.invalidate_continuation();
                        u.user_queue_end.set(0);
                        *u.queue.borrow_mut() = vec![t];
                        u.play_index(0);
                    }
                }
                "queue_last" => {
                    let n = u.queue.borrow().len();
                    if n > 0 {
                        u.play_index(n - 1);
                    }
                }
                "autoplay" => {
                    u.autoplay.set(cmd["enabled"].as_bool().unwrap_or(true));
                }
                "preview_wave" => {
                    let index = cmd["index"].as_u64().unwrap_or(0) as usize;
                    let seed = u.items.borrow().get(index).and_then(|i| {
                        if let Item::Track(t) = i {
                            Some(t.clone())
                        } else {
                            None
                        }
                    });
                    if let Some(seed) = seed {
                        u.preview_wave_from(&seed);
                    }
                }
                "shuffle" => u.shuffle_button.set_active(!u.shuffle_button.is_active()),
                "repeat" => u.repeat_button.emit_clicked(),
                "queue_panel" => u.show_queue(),
                "size" => u.window.set_default_size(
                    cmd["width"].as_i64().unwrap_or(952) as i32,
                    cmd["height"].as_i64().unwrap_or(850) as i32,
                ),
                "panel_toggle" => u.panel_button.emit_clicked(),
                "resize_panel" => {
                    let drag = if cmd["side"] == "left" {
                        &u.sidebar_drag
                    } else {
                        &u.right_drag
                    };
                    let dx = cmd["dx"].as_f64().unwrap_or(0.);
                    drag.emit_by_name::<()>("drag-begin", &[&0.0f64, &0.0f64]);
                    drag.emit_by_name::<()>("drag-update", &[&dx, &0.0f64]);
                    drag.emit_by_name::<()>("drag-end", &[&dx, &0.0f64]);
                }
                "library_toggle" => u.library_toggle.set_active(!u.library_toggle.is_active()),
                "close_panel" => {
                    u.show_details.set(Some(false));
                    u.update_panels();
                }
                "panel_tab" => u.select_panel(cmd["tab"].as_str().unwrap_or("now")),
                "queue_move" => {
                    u.reorder_queue(
                        cmd["from"].as_u64().unwrap_or(0) as usize,
                        cmd["to"].as_u64().unwrap_or(0) as usize,
                    );
                }
                "play_track" => {
                    if let Ok(track) = serde_json::from_value::<Track>(cmd["track"].clone()) {
                        *u.page.borrow_mut() = "test_tracks".into();
                        u.home_scroll.set_visible(false);
                        u.artist_view.set_visible(false);
                        u.settings_view.set_visible(false);
                        u.content_list.set_visible(true);
                        u.set_items(vec![Item::Track(track.clone())]);
                        u.invalidate_continuation();
                        u.user_queue_end.set(0);
                        *u.queue.borrow_mut() = vec![track];
                        u.play_index(0);
                    }
                }
                "collection_view" => u.collection_view.set_selected(cmd["index"].as_u64().unwrap_or(0).min(1) as u32),
                "filter_list" => u.list_filter.set_text(cmd["text"].as_str().unwrap_or("")),
                "sort_list" => u.sort.set_selected(cmd["index"].as_u64().unwrap_or(0).min(2) as u32),
                "row_toggle" => u.activate_or_toggle(cmd["index"].as_u64().unwrap_or(0) as usize),
                "pin" => {
                    let current = u.current.borrow().clone();
                    if let Some(t) = current {
                        u.pin_source(
                            &t,
                            pins::Pin {
                                reference: cmd["ref"].as_str().unwrap_or(&t.reference).into(),
                                provider: cmd["provider"].as_str().unwrap_or("test").into(),
                                duration_s: cmd["duration"].as_f64(),
                            },
                        );
                    }
                }

                "motion" => {
                    let mode = cmd["mode"].as_u64().unwrap_or(1) as u32;
                    u.motion_mode.set(mode);
                    motion::apply(&u.window.clone().upcast(), mode);
                    motion::page(&u.content_list, mode);
                }
                "quick" => match cmd["kind"].as_str().unwrap_or("mix") {
                    "likes" => u.request("likes_play".into(), "/api/nd/liked".into(), vec![], None),
                    "wave" => u.start_wave(),
                    _ => u.request("mix_play".into(), "/api/mixes/today".into(), vec![], None),
                },
                "import_status" => u.import_status(),
                "sources" => {
                    if let Some(t) = u.current.borrow().clone() {
                        u.source_dialog(&t);
                    }
                }
                "wave" => {
                    let t = u
                        .items
                        .borrow()
                        .get(cmd["index"].as_u64().unwrap_or(0) as usize)
                        .and_then(|i| {
                            if let Item::Track(t) = i {
                                Some(t.clone())
                            } else {
                                None
                            }
                        });
                    u.wave_from(t.as_ref(), false);
                }
                "context" => {
                    let index = cmd["index"].as_u64().unwrap_or(0) as usize;
                    let item = u.items.borrow().get(index).cloned();
                    if let Some(item) = item {
                        u.context_item(
                            item,
                            index,
                            &u.list.clone().upcast::<gtk::Widget>(),
                            100.,
                            100.,
                        );
                    }
                }
                "preferences" => u.preferences(),
                "fixture_select_settings" => {
                    let name = cmd["widget"].as_str().unwrap_or_default();
                    let index = cmd["index"].as_u64().unwrap_or_default() as u32;
                    if name == "settings-sections" {
                        if let Some(tab) = settings::test_widget(&format!("settings-section-{index}"))
                            .and_downcast::<gtk::ToggleButton>()
                        {
                            tab.emit_clicked();
                        }
                    } else if let Some(widget) = settings::test_widget(name).and_downcast::<gtk::DropDown>() {
                        widget.set_selected(index);
                    }
                }
                "fixture_toggle_setting" => {
                    if let Some(widget) = settings::test_widget(cmd["widget"].as_str().unwrap_or_default())
                        .and_downcast::<gtk::Switch>()
                    {
                        widget.set_active(cmd["enabled"].as_bool().unwrap_or_default());
                    }
                }
                "fixture_value_setting" => {
                    if let Some(widget) = settings::test_widget(cmd["widget"].as_str().unwrap_or_default())
                        .and_downcast::<gtk::Scale>()
                    {
                        widget.set_value(cmd["value"].as_f64().unwrap_or_default());
                    }
                }
                "fixture_search_settings" => {
                    if let Some(widget) = settings::test_widget("settings-search").and_downcast::<gtk::SearchEntry>()
                    {
                        widget.set_text(cmd["query"].as_str().unwrap_or_default());
                    }
                }
                "fixture_text_setting" => {
                    if let Some(entry) = settings::test_widget(cmd["widget"].as_str().unwrap_or_default())
                        .and_downcast::<gtk::Entry>()
                        .filter(|entry| entry.is_sensitive())
                    {
                        entry.set_text(cmd["text"].as_str().unwrap_or_default());
                        entry.emit_activate();
                    }
                }
                "fixture_click_settings" => {
                    if let Some(widget) = settings::test_widget(cmd["widget"].as_str().unwrap_or_default())
                        .and_downcast::<gtk::Button>()
                        .filter(|button| button.is_sensitive())
                    {
                        widget.emit_clicked();
                    }
                }
                "download_index" | "details_index" => {
                    let track = u
                        .items
                        .borrow()
                        .get(cmd["index"].as_u64().unwrap_or(0) as usize)
                        .and_then(|i| {
                            if let Item::Track(t) = i {
                                Some(t.clone())
                            } else {
                                None
                            }
                        });
                    if let Some(t) = track {
                        if cmd["action"] == "download_index" {
                            u.download(&t);
                        } else {
                            u.open_details(t);
                        }
                    }
                }
                "details" => {
                    if let Some(t) = u.current.borrow().clone() {
                        u.open_details(t);
                    }
                }
                "history_back" => u.history_step(-1),
                "more" => u.load_more(),
                "navigate" => u.navigate(cmd["page"].as_str().unwrap_or("home"), !cmd["cached"].as_bool().unwrap_or(false)),
                "scroll_list" => {
                    if let Some(scroll) = u
                        .list
                        .ancestor(gtk::ScrolledWindow::static_type())
                        .and_downcast::<gtk::ScrolledWindow>()
                    {
                        let a = scroll.vadjustment();
                        a.set_value(
                            (a.upper() - a.page_size())
                                * cmd["fraction"].as_f64().unwrap_or(0.).clamp(0., 1.),
                        );
                    }
                }
                "seek" => {
                    u.seek_to(cmd["position"].as_f64().unwrap_or(0.));
                }
                "quit" => u.window.close(),
                _ => (),
            };
            let mut value = json!({"title":u.title.text().as_str(),"subtitle":u.subtitle.text().as_str(),"primary":Some(u.brand_mark.color().to_string()),"status":u.status.text().as_str(),"items":u.items.borrow().len(),"track":u.current.borrow().as_ref().map(|t|t.title.clone()),"position":u.player.number("time-pos"),"duration":u.player.number("duration"),"paused":u.player.flag("pause"),"playing":u.playing.get(),"theme":theme::path().to_string_lossy(),"queue":u.queue.borrow().len(),"details":u.panel_revealer.reveals_child(),"width":u.window.width(),"covers_pending":u.cover_pending.borrow().len(),"engine":"GStreamer","repeat":u.repeat.get(),"shuffled":u.shuffled.get(),"queue_refs":u.queue.borrow().iter().map(|t|t.reference.clone()).collect::<Vec<_>>(),"index":u.index.get(),"user_queue_end":u.user_queue_end.get(),"volume":u.player.number("volume"),"page":u.page.borrow().clone(),"motion_mode":u.motion_mode.get(),"page_opacity":u.content_list.opacity(),"page_margin":u.content_list.margin_top(),"settings_height":u.settings_view.height(),"settings_visible":u.settings_view.is_visible(),"queue_panel":u.queue_is_visible(), "wave_id":u.wave.borrow().clone(), "autoplay":u.autoplay.get(), "lyrics":details::test_state()});
            value.as_object_mut().unwrap().extend(json!({"panel_tab":u.panel_stack.visible_child_name().map(|s|s.to_string()),"history_count":u.played_history.borrow().len(),"source_pin_count":u.pins.borrow().len(),"playback_ref":u.current.borrow().as_ref().and_then(|t|t.playback_ref.clone()),"canonical_ref":u.current.borrow().as_ref().map(|t|t.reference.clone()),"metadata_duration":u.current.borrow().as_ref().and_then(|t|t.duration_s),"sidebar_scroll_height":u.sidebar_scroll.height(),"sidebar_scroll_parent_height":u.sidebar_revealer.height(),"cover_cache_count":u.cover_cache.borrow().len(),"cover_pixels":u.cover_cache.borrow().iter().map(|(_,t)|t.width()).max(),"artist_visible":u.artist_view.is_visible(),"playing_rows":u.active_rows.borrow().iter().filter_map(|w|w.upgrade()).filter(|i|i.child().is_some_and(|r|r.has_css_class("playing"))).count()}).as_object().unwrap().clone());
            value["window_visible"] = json!(u.window.is_visible());
            value["window_minimize_rect"] = json!(settings::test_widget("window-minimize")
                .and_then(|widget| widget.compute_bounds(&u.window))
                .map(|b| (b.x(), b.y(), b.width(), b.height())));
            value["window_css_classes"] = json!(u.window.css_classes().iter().map(|s| s.as_str()).collect::<Vec<_>>());
            if u.page.borrow().as_str() == "home" {
                value["home_mix_columns"] = json!(settings::test_widget("home-mix-grid")
                    .and_downcast::<gtk::FlowBox>().map(|grid| grid.max_children_per_line()));
                value["home_has_track_waves"] = json!(settings::test_widget("home-wave-grid").is_some());
            }
            value["ripple"] = ripple::test_press();
            value["bars"] = material::test_state();
            value["cover_viewer"] = cover_viewer::test_state();
            value["page_cover_rect"] = json!(named_test_bounds(&u.window.clone().upcast(), &u.window, "page-cover-preview"));
            value["settings_motion"] = json!(settings::test_widget("settings-pages").as_ref().map(motion::state));
            value["preview_motion"] = json!(settings::test_widget("motion-preview-pages").as_ref().map(motion::state));
            value["player_lyrics_visible"] = json!(u.lyrics_button.is_visible());
            value["player_live_lyrics_visible"] = json!(u.now_lyrics.widget().is_visible());
            value["player_cover_fit"] = json!(u.mini_picture.content_fit() == gtk::ContentFit::Cover);
            value["timeline_fraction"] = json!(u.timeline.fraction.get());
            value["timeline_rect"] = json!(u.timeline.canvas.compute_bounds(&u.window)
                .map(|b| (b.x(), b.y(), b.width(), b.height())));
            value["ux"] = ux_probe::state(&u);
            if u.settings_view.is_visible() {
                value["settings-sections"] = json!(u.settings_section());
                if let Some(name) = cmd["probe_setting"].as_str() {
                    let widget = settings::test_widget(name);
                    value["setting_visible"] = json!(widget.is_some());
                    value["setting_rect"] = json!(widget.and_then(|w| w.compute_bounds(&u.window))
                        .map(|b| (b.x(), b.y(), b.width(), b.height())));

                }
            }
            if u.settings_view.is_visible() || matches!(u.page.borrow().as_str(), "downloads" | "download_jobs") {
                for name in ["pref-theme_mode", "connection-status", "download-status", "shortcut-settings", "shortcut-apply"] {
                    if let Some(widget) = settings::test_widget(name) {
                        value[name] = if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                            json!(label.text().as_str())
                        } else if let Some(dropdown) = widget.downcast_ref::<gtk::DropDown>() {
                            json!(dropdown.selected())
                        } else {
                            Value::Null
                        };
                        value[format!("{name}-rect")] = json!(widget.compute_bounds(&u.window).map(|b| (b.x(), b.y(), b.width(), b.height())));
                    }
                }
            }
            value["list_update_ms"] = json!(u.list_update_ms.get());
            let visible: Vec<_> = u
                .cover_bindings
                .borrow()
                .values()
                .filter_map(|(weak, _)| weak.upgrade())
                .filter(|p| {
                    p.is_mapped()
                        && p.compute_bounds(&u.window).is_some_and(|b| {
                            b.y() < u.window.height() as f32 && b.y() + b.height() > 0.
                        })
                })
                .collect();
            value["visible_cover_slots"] = json!(visible.len());
            value["visible_cover_ready"] =
                json!(visible.iter().filter(|p| p.paintable().is_some()).count());
            value["main_list_rect"] = json!(u.list.compute_bounds(&u.window).map(|b| (
                b.x(),
                b.y(),
                b.width(),
                b.height()
            )));
            value["play_button_rect"] = json!(u.play_button.compute_bounds(&u.window).map(|b| (
                b.x(), b.y(), b.width(), b.height()
            )));
            value["list_sort"] = json!(u.sort.selected());
            value["resolving"] = json!(u.resolving_pause.get().is_some());
            value["collection_view"] = json!(u.collection_view.selected());
            value["mini_title_rect"] = json!(u.mini_title.compute_bounds(&u.window).map(|b| (
                b.x(),
                b.y(),
                b.width(),
                b.height()
            )));
            value["context_origins"] =
                json!(count_css_class(&u.window.clone().upcast(), "context-open"));
            value["keyboard_navigation"] = json!(u.window.has_css_class("keyboard-navigation"));
            value["mini_like_color"] = json!(u.mini_like.child().map(|w| w.color().to_string()));
            value["search_sections"] =
                json!(u.page.borrow().starts_with("search:") && u.artist_view.is_visible());
            value["cover_debug"] = json!({"deferred":u.deferred_covers.borrow().len(),"pending":u.cover_pending.borrow().len(),"sample":u.deferred_covers.borrow().values().take(4).filter_map(|(w,_)|w.upgrade()).map(|p|json!({"mapped":p.is_mapped(),"visible":p.is_visible(),"bounds":p.compute_bounds(&u.window).map(|b|(b.x(),b.y(),b.width(),b.height()))})).collect::<Vec<_>>()});
            value["content_right"] = json!(
                u.content_list
                    .parent()
                    .and_then(|w| w.parent())
                    .and_then(|w| w.compute_bounds(&u.window))
                    .map(|b| b.x() + b.width())
            );
            value["panel_button_rect"] = json!(u.panel_button.compute_bounds(&u.window).map(|b| (
                b.x(),
                b.y(),
                b.width(),
                b.height()
            )));
            value["height"] = json!(u.window.height());
            value["resize_handles"] = json!(
                [
                    Some(u.left_resize_handle.clone().upcast::<gtk::Widget>()),
                    Some(u.right_resize_handle.clone().upcast::<gtk::Widget>())
                ]
                .iter()
                .map(|w| w
                    .as_ref()
                    .and_then(|w| w.compute_bounds(&u.window))
                    .map(|b| (b.x(), b.y(), b.width(), b.height())))
                .collect::<Vec<_>>()
            );
            value["content_width"] = json!(u.content_list.parent().and_then(|w| w.parent()).map(|w| w.width()));
            value["sidebar_open"] = json!(u.sidebar_revealer.reveals_child());
            value["sidebar_width"] = json!(u.sidebar_revealer.width());
            value["panel_width"] = json!(u.panel_revealer.width());
            value["library_active"] = json!(u.library_toggle.is_active());
            value["track_titles"] = json!(test_track_title_layout(&u.list.clone().upcast()));
            value["list_scroll"] = json!(
                u.list
                    .ancestor(gtk::ScrolledWindow::static_type())
                    .and_downcast::<gtk::ScrolledWindow>()
                    .map(|s| {
                        let a = s.vadjustment();
                        (a.value(), a.upper(), a.page_size())
                    })
            );
            value["player_alignment"] = json!([
                named_test_bounds(&u.window.clone().upcast(), &u.window, "player-track-cover"),
                named_test_bounds(&u.window.clone().upcast(), &u.window, "player-track-text")
            ]);
            value["volume_mute_rect"] = json!(named_test_bounds(
                &u.window.clone().upcast(), &u.window, "volume-mute"
            ));
            value["download_jobs_rect"] = json!(u.downloads_jobs.compute_bounds(&u.window)
                .map(|b| (b.x(), b.y(), b.width(), b.height())));
            value["collection_play_rect"] = json!(u.collection_play.compute_bounds(&u.window)
                .map(|b| (b.x(), b.y(), b.width(), b.height())));
            value["collection_actions"] = json!({
                "play": u.collection_play.is_visible() && u.collection_play.is_sensitive(),
                "shuffle": u.collection_shuffle.is_visible() && u.collection_shuffle.is_sensitive(),
                "playlist": u.playlist_manage.is_visible(),
                "album": u.album_article.is_visible(),
                "jobs": u.downloads_jobs.is_visible(),
            });
            value["volume_rect"] = json!(named_test_bounds(
                &u.window.clone().upcast(),
                &u.window,
                "volume-control"
            ));
            value["artist_more_rect"] = json!(named_test_bounds(
                &u.window.clone().upcast(),
                &u.window,
                "artist-all-tracks"
            ));
            value["subtitle_rect"] = json!(u.subtitle.compute_bounds(&u.window).map(|b| (
                b.x(),
                b.y(),
                b.width(),
                b.height()
            )));
            value["chrome"] = json!({"top":u.top_bar.compute_bounds(&u.window).map(|b|b.height()),"bottom":u.player_bar.compute_bounds(&u.window).map(|b|b.height()),"panel_button_active":u.panel_button.has_css_class("selected"),"panel_button_height":u.panel_button.height(),"panel_button_tip":u.panel_button.tooltip_text().map(|s|s.to_string()),"sidebar_preferred":u.sidebar_width.get(),"right_preferred":u.right_width.get(),"panel_opacity":u.panel_stack.opacity()});
            value["screenshot"] = capture;
            value["queue_rows"] =
                json!(queue_positions(&u.queue_panel.clone().upcast(), &u.window));
            let _ = reply.send(value);
        }
    });
}

fn glyph(name: &str) -> &'static str {
    match name {
        "media-playback-start-symbolic" => "\u{e037}",
        "media-playback-pause-symbolic" => "\u{e034}",
        "media-skip-backward-symbolic" => "\u{e045}",
        "media-skip-forward-symbolic" => "\u{e044}",
        "go-home-symbolic" => "\u{e88a}",
        "system-search-symbolic" => "\u{e8b6}",
        "go-previous-symbolic" => "\u{e5c4}",
        "go-next-symbolic" => "\u{e5c8}",
        "media-playlist-shuffle-symbolic" => "\u{e043}",
        "media-playlist-repeat-symbolic" => "\u{e040}",
        "repeat-one" => "\u{e041}",
        "view-more-symbolic" => "\u{e5d3}",
        "view-list-symbolic" => "\u{e03d}",
        "document-open-recent-symbolic" => "\u{e889}",
        "folder-music-symbolic" => "\u{e030}",
        "media-optical-symbolic" => "\u{e019}",
        "avatar-default-symbolic" => "\u{e7fd}",
        "emblem-favorite-symbolic" => "\u{e87d}",
        "list-add-symbolic" => "\u{e145}",
        "folder-download-symbolic" => "\u{f090}",
        "emblem-system-symbolic" => "\u{e8b8}",
        "window-minimize-symbolic" => "\u{e15b}",
        "window-close-symbolic" => "\u{e5cd}",
        "view-refresh-symbolic" => "\u{e5d5}",
        "view-reveal-symbolic" => "\u{e8f4}",
        "object-rotate-left-symbolic" => "\u{e419}",
        "object-rotate-right-symbolic" => "\u{e41a}",
        "zoom-out-symbolic" => "\u{e900}",
        "zoom-in-symbolic" => "\u{e8ff}",
        "zoom-fit-best-symbolic" => "\u{e56b}",
        "view-fullscreen-symbolic" => "\u{e5d0}",
        "view-restore-symbolic" => "\u{e5d1}",
        "preferences-desktop-locale-symbolic" => "\u{e894}",
        "audio-volume-high-symbolic" => "\u{e050}",
        "audio-volume-muted-symbolic" => "\u{e04f}",
        "text-x-generic-symbolic" => "\u{e01c}",
        "document-import-symbolic" => "\u{e2c6}",
        _ => "\u{e405}",
    }
}
fn symbol(name: &str) -> gtk::Label {
    let l = label(glyph(name), "md-icon");
    l.set_size_request(24, 24);
    l.set_xalign(0.5);
    l.set_yalign(0.5);
    l.set_halign(gtk::Align::Center);
    l.set_valign(gtk::Align::Center);
    l
}
fn set_md_icon(b: &gtk::Button, name: &str) {
    if let Some(label) = b.child().and_downcast::<gtk::Label>() {
        if label.text().as_str() != glyph(name) {
            label.set_text(glyph(name));
        }
    } else {
        b.set_child(Some(&symbol(name)));
    }
}
fn read_pref(key: &str) -> Value {
    crate::config::preference(key)
}

fn complete_album(api: &Api, local: Value) -> Value {
    let title = text(&local, "title");
    let artist = text(&local, "artist");
    if title.trim().is_empty() || artist.trim().is_empty() {
        return local;
    }
    for query in [format!("{artist} {title}"), title.clone()] {
        if let Ok(search) = api.query_with_timeout(
            "/api/search", &[("q", query.as_str())], Duration::from_secs(5),
        ) {
            if let Some(albums) = search["albums"].as_array() {
                for album in albums
                    .iter()
                    .filter(|a| artist::same_release(&local, a))
                    .take(4)
                {
                    let reference = text(album, "ref");
                    if reference.is_empty() {
                        continue;
                    }
                    if let Ok(full) = api.get_with_timeout(&format!("/api/catalog/album/{reference}"), Duration::from_secs(4)) {
                        if artist::same_release(&local, &full)
                            && full["tracks"].as_array().is_some_and(|v| !v.is_empty())
                        {
                            return full;
                        }
                    }
                }
            }
        }
    }
    local
}

fn mount_dialog(window: &gtk::Window, content: &impl IsA<gtk::Widget>) {
    window.set_decorated(false);
    let root = vbox(0);
    let bar = hbox(8);
    bar.add_css_class("dialog-bar");
    let title = label(window.title().as_deref().unwrap_or(""), "section-title");
    title.set_hexpand(true);
    bar.append(&title);
    let close = icon_button("window-close-symbolic", "Закрыть", "flat");
    let weak = window.downgrade();
    close.connect_clicked(move |_| {
        if let Some(w) = weak.upgrade() {
            w.close();
        }
    });
    bar.append(&close);
    let handle = gtk::WindowHandle::new();
    handle.set_child(Some(&bar));
    root.append(&handle);
    content.as_ref().set_vexpand(true);
    root.append(content);
    window.set_child(Some(&ripple::wrap(&root)));
    autoscroll::install(window);
}

fn cover_frame(picture: &gtk::Picture, size: i32) -> gtk::Overlay {
    let frame = gtk::Overlay::new();
    frame.set_hexpand(false);
    frame.set_valign(gtk::Align::Center);
    frame.add_css_class("cover-frame");
    let fallback = symbol("folder-music-symbolic");
    fallback.set_size_request(size, size);
    frame.set_child(Some(&fallback));
    frame.add_overlay(picture);
    frame.set_measure_overlay(picture, false);
    frame.set_overflow(gtk::Overflow::Hidden);
    frame
}
fn set_like_icon(button: &gtk::Button, liked: bool) {
    set_md_icon(button, "emblem-favorite-symbolic");
    if let Some(label) = button.child().and_downcast::<gtk::Label>() {
        let attrs = gtk::pango::AttrList::new();
        let mut font = gtk::pango::FontDescription::from_string("Material Symbols Rounded");
        font.set_variations(Some(if liked { "FILL=1" } else { "FILL=0" }));
        attrs.insert(gtk::pango::AttrFontDesc::new(&font));
        label.set_attributes(Some(&attrs));
    }
    if liked {
        button.add_css_class("liked")
    } else {
        button.remove_css_class("liked")
    }
}

fn queue_positions(widget: &gtk::Widget, window: &gtk::ApplicationWindow) -> Vec<Value> {
    let mut rows = vec![];
    if let Some(index) = widget
        .widget_name()
        .strip_prefix("queue-row-")
        .and_then(|s| s.parse::<u64>().ok())
    {
        if let Some(b) = widget.compute_bounds(window) {
            if b.y() > 0. && b.y() < window.height() as f32 {
                rows.push(json!({"index":index,"x":b.x(),"y":b.y(),"width":b.width(),"height":b.height()}));
            }
        }
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        rows.extend(queue_positions(&c, window));
        child = c.next_sibling();
    }
    rows
}

// Snapshot this owned test widget, never the compositor or another application's pixels.
fn capture_test_window(window: &gtk::ApplicationWindow, name: &str) -> Result<String, String> {
    use gtk::gsk::prelude::*;
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("Invalid screenshot name".into());
    }
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
    let node = snapshot.to_node().ok_or_else(|| {
        window.queue_draw();
        "Window has no rendered content"
    })?;
    let renderer = window.renderer().ok_or("Window has no renderer")?;
    let texture = renderer.render_texture(&node, None);
    let path = crate::config::test_root().join(format!("{name}.png"));
    texture.save_to_png(&path).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

fn test_track_title_layout(widget: &gtk::Widget) -> Vec<Value> {
    let mut result = Vec::new();
    if widget.has_css_class("track-title") {
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            result.push(json!({"text":label.text().as_str(), "width":label.width(), "ellipsized":label.layout().is_ellipsized()}));
        }
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        result.extend(test_track_title_layout(&current));
        child = current.next_sibling();
    }
    result
}

fn named_test_bounds(
    widget: &gtk::Widget,
    window: &impl IsA<gtk::Widget>,
    name: &str,
) -> Option<(f32, f32, f32, f32)> {
    if widget.widget_name() == name && widget.is_mapped() {
        return widget
            .compute_bounds(window)
            .map(|b| (b.x(), b.y(), b.width(), b.height()));
    }
    let mut child = widget.first_child();
    while let Some(w) = child {
        if let Some(rect) = named_test_bounds(&w, window, name) {
            return Some(rect);
        }
        child = w.next_sibling();
    }
    None
}

fn count_css_class(widget: &gtk::Widget, class: &str) -> usize {
    let mut count = usize::from(widget.has_css_class(class));
    let mut child = widget.first_child();
    while let Some(w) = child {
        count += count_css_class(&w, class);
        child = w.next_sibling();
    }
    count
}

fn year_text(value: &Value) -> String {
    value["year"]
        .as_i64()
        .map(|n| n.to_string())
        .or_else(|| value["year"].as_str().map(String::from))
        .or_else(|| {
            value["release_date"]
                .as_str()
                .map(|s| s.split('-').next().unwrap_or("").to_string())
        })
        .unwrap_or_default()
}
