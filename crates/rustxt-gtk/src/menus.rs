//! Menu models for the menu bar and the tab context menu.
//! Every item targets a `win.*` action defined in `window.rs`.

use gtk::gio;
use gtk::prelude::*;

pub struct Menus {
    pub bar: gio::Menu,
    /// Rebuilt whenever the list of closed tabs changes.
    pub recently_closed: gio::Menu,
    pub tab_context: gio::Menu,
}

fn section(items: &[(&str, &str)]) -> gio::Menu {
    let menu = gio::Menu::new();
    for (label, action) in items {
        menu.append(Some(label), Some(action));
    }
    menu
}

pub fn build() -> Menus {
    let recently_closed = gio::Menu::new();

    let file = gio::Menu::new();
    let open_section = section(&[("_New tab", "win.new-tab"), ("_Open…", "win.open")]);
    open_section.append_submenu(Some("Recently _closed"), &recently_closed);
    file.append_section(None, &open_section);
    file.append_section(
        None,
        &section(&[
            ("_Save", "win.save"),
            ("Save _as…", "win.save-as"),
            ("Save a_ll", "win.save-all"),
        ]),
    );
    file.append_section(None, &section(&[("_Print…", "win.print")]));
    file.append_section(
        None,
        &section(&[
            ("Close _tab", "win.close-tab"),
            ("_Discard changes and close", "win.discard"),
            ("_Reopen closed tab", "win.reopen-last"),
        ]),
    );
    file.append_section(None, &section(&[("E_xit", "win.exit")]));

    let edit = gio::Menu::new();
    edit.append_section(
        None,
        &section(&[("_Undo", "win.undo"), ("_Redo", "win.redo")]),
    );
    edit.append_section(
        None,
        &section(&[
            ("Cu_t", "win.cut"),
            ("_Copy", "win.copy"),
            ("_Paste", "win.paste"),
            ("De_lete", "win.delete"),
        ]),
    );
    edit.append_section(
        None,
        &section(&[
            ("_Find…", "win.find"),
            ("Find _next", "win.find-next"),
            ("Find pre_vious", "win.find-previous"),
            ("R_eplace…", "win.replace"),
            ("_Go to…", "win.goto"),
        ]),
    );
    edit.append_section(
        None,
        &section(&[
            ("Select _all", "win.select-all"),
            ("Time/_Date", "win.time-date"),
        ]),
    );

    let view = gio::Menu::new();
    let zoom = section(&[
        ("Zoom _in", "win.zoom-in"),
        ("Zoom _out", "win.zoom-out"),
        ("_Restore default zoom", "win.zoom-reset"),
    ]);
    let zoom_section = gio::Menu::new();
    zoom_section.append_submenu(Some("_Zoom"), &zoom);
    view.append_section(None, &zoom_section);
    view.append_section(
        None,
        &section(&[
            ("_Status bar", "win.status-bar"),
            ("_Word wrap", "win.word-wrap"),
        ]),
    );

    let settings = gio::Menu::new();
    settings.append_section(None, &section(&[("_Settings…", "win.settings")]));
    settings.append_section(None, &section(&[("_About RusTXT", "win.about")]));

    let bar = gio::Menu::new();
    bar.append_submenu(Some("_File"), &file);
    bar.append_submenu(Some("_Edit"), &edit);
    bar.append_submenu(Some("_View"), &view);
    bar.append_submenu(Some("_Settings"), &settings);

    let tab_context = gio::Menu::new();
    tab_context.append_section(
        None,
        &section(&[
            ("Close tab", "win.tab-close"),
            ("Close other tabs", "win.tab-close-others"),
            ("Discard changes and close", "win.tab-discard"),
        ]),
    );
    tab_context.append_section(
        None,
        &section(&[("Save", "win.tab-save"), ("Save as…", "win.tab-save-as")]),
    );

    Menus {
        bar,
        recently_closed,
        tab_context,
    }
}

/// Keyboard shortcuts. Text-editing accelerators are also bound by the text
/// view itself, which handles them first while it has focus; these make the
/// menu items show their shortcuts and work when focus is elsewhere.
pub fn install_accelerators(app: &impl IsA<gtk::Application>) {
    let accels: &[(&str, &[&str])] = &[
        ("win.new-tab", &["<Control>n", "<Control>t"]),
        ("win.open", &["<Control>o"]),
        ("win.save", &["<Control>s"]),
        ("win.save-as", &["<Control><Shift>s"]),
        ("win.save-all", &["<Control><Alt>s"]),
        ("win.print", &["<Control>p"]),
        ("win.close-tab", &["<Control>w"]),
        ("win.reopen-last", &["<Control><Shift>t"]),
        ("win.settings", &["<Control>comma"]),
        ("win.exit", &["<Control><Shift>w", "<Control>q"]),
        ("win.undo", &["<Control>z"]),
        ("win.redo", &["<Control>y", "<Control><Shift>z"]),
        ("win.cut", &["<Control>x"]),
        ("win.copy", &["<Control>c"]),
        ("win.paste", &["<Control>v"]),
        ("win.delete", &["Delete"]),
        ("win.find", &["<Control>f"]),
        ("win.find-next", &["F3"]),
        ("win.find-previous", &["<Shift>F3"]),
        ("win.replace", &["<Control>h"]),
        ("win.goto", &["<Control>g"]),
        ("win.select-all", &["<Control>a"]),
        ("win.time-date", &["F5"]),
        (
            "win.zoom-in",
            &["<Control>plus", "<Control>equal", "<Control>KP_Add"],
        ),
        ("win.zoom-out", &["<Control>minus", "<Control>KP_Subtract"]),
        ("win.zoom-reset", &["<Control>0", "<Control>KP_0"]),
    ];
    for (action, keys) in accels {
        app.set_accels_for_action(action, keys);
    }
}
