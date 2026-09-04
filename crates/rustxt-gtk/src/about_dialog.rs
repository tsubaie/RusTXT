//! The About dialog: logo, version, a one-line pitch, where to find the
//! source, and a button that asks GitHub whether a newer release exists.
//! The logo carries the wordmark, so there is no separate name label, and
//! it is embedded so it shows even without an installed icon theme.
//!
//! Per-user installs can update in place from here; everyone else is told
//! where the update comes from. Nothing touches the network until the
//! button is pressed.

use adw::prelude::*;
use gtk::glib;
use rustxt_core::update::{self, Install, Release};
use std::{cell::RefCell, path::PathBuf, process::Command, rc::Rc};

pub fn present(parent: &impl IsA<gtk::Widget>) {
    let logo = gtk::Image::from_paintable(Some(&crate::theme::logo_texture()));
    logo.set_pixel_size(128);
    logo.set_margin_bottom(6);

    let version = gtk::Label::new(Some(&format!("Version {}", crate::VERSION)));
    version.add_css_class("dim-label");

    let pitch = gtk::Label::new(Some(
        "Dead simple, recoverable plain text editing. It opens, it saves, it stays out of your way.",
    ));
    pitch.set_wrap(true);
    pitch.set_justify(gtk::Justification::Center);
    pitch.set_max_width_chars(36);
    pitch.set_margin_top(12);

    let links = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    links.set_halign(gtk::Align::Center);
    links.set_margin_top(12);
    links.append(&gtk::LinkButton::with_label(
        update::REPOSITORY,
        "Source code",
    ));
    links.append(&gtk::LinkButton::with_label(
        &format!("{}/issues", update::REPOSITORY),
        "Report a problem",
    ));

    let built = gtk::Label::new(Some("Built with GTK 4, libadwaita and GtkSourceView."));
    built.add_css_class("dim-label");
    built.add_css_class("caption");
    built.set_margin_top(12);

    let license = gtk::Label::new(Some("MIT License · © 2026 Talal A. Alsubaie"));
    license.add_css_class("dim-label");
    license.add_css_class("caption");

    let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
    content.set_halign(gtk::Align::Center);
    content.set_margin_top(12);
    content.set_margin_bottom(24);
    content.set_margin_start(24);
    content.set_margin_end(24);
    for widget in [
        logo.upcast_ref::<gtk::Widget>(),
        version.upcast_ref(),
        Updater::new(parent.clone().upcast()).root.upcast_ref(),
        pitch.upcast_ref(),
        links.upcast_ref(),
        built.upcast_ref(),
        license.upcast_ref(),
    ] {
        content.append(widget);
    }

    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&view_wrap(&content)));

    let dialog = adw::Dialog::new();
    dialog.set_title("About RusTXT");
    dialog.set_content_width(380);
    dialog.set_child(Some(&view));
    dialog.present(Some(parent));
}

fn view_wrap(content: &gtk::Box) -> gtk::Box {
    let wrap = gtk::Box::new(gtk::Orientation::Vertical, 0);
    wrap.append(content);
    wrap
}

/// The update row: one button to check, then the verdict and, when the app
/// can do it, a button to install and one to restart.
struct Updater {
    root: gtk::Box,
    status: gtk::Label,
    check: gtk::Button,
    notes: gtk::LinkButton,
    install: gtk::Button,
    restart: gtk::Button,
    release: RefCell<Option<Release>>,
    window: gtk::Widget,
}

impl Updater {
    fn new(window: gtk::Widget) -> Rc<Self> {
        let status = gtk::Label::new(None);
        status.add_css_class("dim-label");
        status.set_wrap(true);
        status.set_justify(gtk::Justification::Center);
        status.set_max_width_chars(36);
        status.set_visible(false);

        let check = gtk::Button::with_label("Check for updates");
        let notes = gtk::LinkButton::with_label("", "Release notes");
        notes.set_visible(false);
        let install = gtk::Button::with_label("Update now");
        install.add_css_class("suggested-action");
        install.set_visible(false);
        let restart = gtk::Button::with_label("Restart now");
        restart.add_css_class("suggested-action");
        restart.set_visible(false);

        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        buttons.set_halign(gtk::Align::Center);
        for button in [
            check.upcast_ref::<gtk::Widget>(),
            notes.upcast_ref(),
            install.upcast_ref(),
            restart.upcast_ref(),
        ] {
            buttons.append(button);
        }

        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        root.set_margin_top(8);
        root.append(&status);
        root.append(&buttons);

        let this = Rc::new(Self {
            root,
            status,
            check,
            notes,
            install,
            restart,
            release: RefCell::new(None),
            window,
        });
        let updater = this.clone();
        this.check.connect_clicked(move |_| updater.check());
        let updater = this.clone();
        this.install.connect_clicked(move |_| updater.install());
        let updater = this.clone();
        this.restart.connect_clicked(move |_| updater.restart());
        this
    }

    fn say(&self, text: &str) {
        self.status.set_text(text);
        self.status.set_visible(true);
    }

    fn check(self: &Rc<Self>) {
        self.check.set_sensitive(false);
        self.say("Checking…");
        let this = self.clone();
        in_background(
            || update::fetch_latest(crate::VERSION),
            move |result| this.checked(result),
        );
    }

    fn checked(&self, result: Result<Release, String>) {
        self.check.set_sensitive(true);
        let release = match result {
            Ok(release) => release,
            Err(error) => return self.say(&error),
        };
        if !release.is_newer_than(crate::VERSION) {
            return self.say("You have the latest version.");
        }
        self.notes.set_uri(&release.url);
        self.notes.set_visible(true);
        let install = Install::detect();
        match install.hint() {
            None => {
                self.say(&format!("Version {} is available.", release.version()));
                self.check.set_visible(false);
                self.install.set_visible(true);
            }
            Some(hint) => self.say(&format!(
                "Version {} is available. {hint}",
                release.version()
            )),
        }
        *self.release.borrow_mut() = Some(release);
    }

    fn install(self: &Rc<Self>) {
        let Some(release) = self.release.borrow().clone() else {
            return;
        };
        let Some(target) = Install::detect().replaceable_binary().map(PathBuf::from) else {
            return;
        };
        self.install.set_sensitive(false);
        self.say(&format!("Downloading version {}…", release.version()));
        let this = self.clone();
        let version = release.version().to_string();
        in_background(
            move || update::install(&release, &target),
            move |result| match result {
                Ok(()) => {
                    this.say(&format!(
                        "Version {version} is installed. Restart RusTXT to start using it."
                    ));
                    this.install.set_visible(false);
                    this.restart.set_visible(true);
                }
                Err(error) => {
                    this.say(&format!("Update failed. {error}"));
                    this.install.set_sensitive(true);
                }
            },
        );
    }

    /// Hand off to the new binary once this process has gone, so the
    /// single-instance handshake finds nobody home.
    fn restart(&self) {
        if let Ok(exe) = std::env::current_exe() {
            let _ = Command::new("sh")
                .arg("-c")
                .arg("sleep 1; exec \"$0\"")
                .arg(exe)
                .spawn();
        }
        if let Some(app) = self
            .window
            .root()
            .and_downcast::<gtk::Window>()
            .and_then(|window| window.application())
        {
            app.quit();
        }
    }
}

/// Run blocking work on a thread and hand its result back on the UI thread.
fn in_background<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    then: impl FnOnce(T) + 'static,
) {
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = sender.send_blocking(work());
    });
    glib::spawn_future_local(async move {
        if let Ok(result) = receiver.recv().await {
            then(result);
        }
    });
}
