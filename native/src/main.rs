mod api;
mod audio;
mod config;
mod theme;
mod ui;
use gtk::{gio, glib, prelude::*};
fn main() -> gtk::glib::ExitCode {
    if config::test() {
        let _ = config::test_root();
        let _ = config::backend();
    }
    let id = if config::test() {
        format!(
            "io.github.balsammist.HomeMusic.Test.p{}",
            std::process::id()
        )
    } else {
        "io.github.balsammist.HomeMusic".into()
    };
    let app = gtk::Application::builder()
        .application_id(&id)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    app.add_main_option(
        "background",
        0u8.into(),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Start HomeMusic in the system tray",
        None,
    );
    app.connect_activate(|app| ui::build(app, true));
    app.connect_command_line(|app, command| {
        if !command.options_dict().contains("background") {
            app.activate();
        } else if !command.is_remote() {
            ui::build(app, false);
        }
        glib::ExitCode::SUCCESS
    });
    app.run()
}
