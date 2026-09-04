//! The About dialog: logo, version, a one-line pitch and where to find the
//! source. The logo carries the wordmark, so there is no separate name label.
//! It is embedded so it shows even without an installed icon theme.

use adw::prelude::*;

const REPOSITORY: &str = "https://github.com/tsubaie/RusTXT";

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
    links.append(&gtk::LinkButton::with_label(REPOSITORY, "Source code"));
    links.append(&gtk::LinkButton::with_label(
        &format!("{REPOSITORY}/issues"),
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
        pitch.upcast_ref(),
        links.upcast_ref(),
        built.upcast_ref(),
        license.upcast_ref(),
    ] {
        content.append(widget);
    }

    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&content));

    let dialog = adw::Dialog::new();
    dialog.set_title("About RusTXT");
    dialog.set_content_width(380);
    dialog.set_child(Some(&view));
    dialog.present(Some(parent));
}
