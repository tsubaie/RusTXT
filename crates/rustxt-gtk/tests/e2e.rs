//! End-to-end tests of the real application.
//!
//! Each scenario runs the actual window in a child process: this test binary
//! re-executes itself with `RUSTXT_E2E` set, inside a sandbox of scratch XDG
//! directories and with single-instance registration off, so nothing here
//! can touch the copy of RusTXT the developer may have open. Where a crash
//! is simulated the parent kills the child with SIGKILL and checks what the
//! next launch shows, which is the promise the README makes.
//!
//! A display is required: a Wayland or X11 session locally, Xvfb in CI.

use rustxt_core::storage::Storage;
use std::{
    env, fs,
    panic::{self, AssertUnwindSafe},
    path::PathBuf,
    process::{Child, Command},
    time::{Duration, Instant},
};

const TEXT: &str = "Rewrite the intro\nthen call the bank\n";
const FIRST: &str = "first note";
const SECOND: &str = "second note";
const CRLF_FILE: &str = "hello\r\nworld\r\n";
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);
/// Separates fields in a child's report; never appears in test text.
const SEP: char = '\u{1f}';

type Scenario = fn(&Sandbox);

fn main() {
    if let Ok(scenario) = env::var("RUSTXT_E2E") {
        child::run(&scenario);
    }
    let tests: [(&str, Scenario); 8] = [
        ("crash_recovery", crash_recovery),
        ("many_tabs_survive_a_crash", many_tabs_survive_a_crash),
        ("close_and_reopen", close_and_reopen),
        ("save_keeps_line_endings", save_keeps_line_endings),
        (
            "file_changed_on_disk_then_deleted",
            file_changed_on_disk_then_deleted,
        ),
        (
            "undo_redo_time_date_and_delete",
            undo_redo_time_date_and_delete,
        ),
        ("zoom_persists_across_exit", zoom_persists_across_exit),
        (
            "empty_and_whitespace_tabs_are_dropped",
            empty_and_whitespace_tabs_are_dropped,
        ),
    ];
    let mut failed = 0;
    println!("\nrunning {} end-to-end tests", tests.len());
    for (name, test) in tests {
        let sandbox = Sandbox::new();
        match panic::catch_unwind(AssertUnwindSafe(|| test(&sandbox))) {
            Ok(()) => println!("test {name} ... ok"),
            Err(_) => {
                failed += 1;
                println!("test {name} ... FAILED");
            }
        }
    }
    println!(
        "\ne2e result: {}. {} passed; {failed} failed\n",
        if failed == 0 { "ok" } else { "FAILED" },
        tests.len() - failed
    );
    if failed > 0 {
        std::process::exit(1);
    }
}

/// Type, get killed without warning, and come back with everything intact.
fn crash_recovery(sandbox: &Sandbox) {
    let mut child = sandbox.spawn("type-then-hang");
    sandbox.wait_for_ready(&mut child);
    child.kill().expect("SIGKILL the editor");
    child.wait().expect("reap the killed editor");

    let session = sandbox
        .storage()
        .restore_session()
        .expect("read the session");
    assert_eq!(session.documents.len(), 1, "one tab survives the crash");
    assert_eq!(session.documents[0].content, TEXT);
    assert!(
        session.documents[0].dirty,
        "unsaved text is flagged as such"
    );

    let shown = sandbox.dump();
    assert_eq!(shown.tabs, [TEXT], "the relaunched window shows the text");
    assert_eq!(
        shown.cursor,
        TEXT.chars().count() as i32,
        "cursor stays at the end"
    );
}

/// Three tabs, the middle one active with the cursor moved, all come back.
fn many_tabs_survive_a_crash(sandbox: &Sandbox) {
    let mut child = sandbox.spawn("three-tabs-then-hang");
    sandbox.wait_for_ready(&mut child);
    child.kill().expect("SIGKILL the editor");
    child.wait().expect("reap the killed editor");

    let shown = sandbox.dump();
    assert_eq!(shown.tabs, ["alpha", "beta", "gamma"]);
    assert_eq!(shown.selected, 1, "the active tab is restored");
    assert_eq!(
        shown.cursor, 2,
        "the cursor position in the active tab is restored"
    );
}

/// Close a tab with text, see it listed by its first line, bring it back.
fn close_and_reopen(sandbox: &Sandbox) {
    let report = sandbox.run("close-reopen");
    assert_eq!(
        report,
        ["1", "2", SECOND, SECOND],
        "tabs after close, tabs after reopen, closed-list label, reopened text"
    );
    let storage = sandbox.storage();
    let session = storage.restore_session().expect("read the session");
    let contents: Vec<&str> = session
        .documents
        .iter()
        .map(|d| d.content.as_str())
        .collect();
    assert_eq!(contents, [FIRST, SECOND]);
    assert!(
        storage
            .closed_documents()
            .expect("list closed tabs")
            .is_empty(),
        "the reopened tab left the closed list"
    );
}

/// Open a CRLF file, edit, save: the file keeps CRLF and the database keeps
/// no copy of a file that is safely on disk. Opening it again is a no-op.
fn save_keeps_line_endings(sandbox: &Sandbox) {
    let file = sandbox.root.join("notes.txt");
    fs::write(&file, CRLF_FILE).unwrap();
    let report = sandbox.run("edit-file");
    assert_eq!(
        report,
        ["2", "2", "Dear hello\nworld\n"],
        "tabs, tabs after reopening, text"
    );
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "Dear hello\r\nworld\r\n"
    );

    let session = sandbox
        .storage()
        .restore_session()
        .expect("read the session");
    let saved = session
        .documents
        .iter()
        .find(|d| d.file_path.is_some())
        .expect("the file's tab is in the session");
    assert!(!saved.dirty);
    assert_eq!(saved.content, "", "a clean saved file stores no text");
}

/// A file edited elsewhere is reloaded; a file that vanished is let go.
fn file_changed_on_disk_then_deleted(sandbox: &Sandbox) {
    let file = sandbox.root.join("notes.txt");
    fs::write(&file, "original\n").unwrap();
    let mut child = sandbox.spawn("open-file-then-hang");
    sandbox.wait_for_ready(&mut child);
    child.kill().expect("SIGKILL the editor");
    child.wait().expect("reap the killed editor");

    fs::write(&file, "changed elsewhere\n").unwrap();
    let shown = sandbox.dump();
    assert_eq!(
        shown.tabs,
        ["", "changed elsewhere\n"],
        "the untitled tab and the reloaded file"
    );

    fs::remove_file(&file).unwrap();
    let shown = sandbox.dump();
    assert_eq!(
        shown.tabs,
        [""],
        "the missing file's tab is gone, the untitled tab stays"
    );
}

/// Editing actions through the menu actions.
fn undo_redo_time_date_and_delete(sandbox: &Sandbox) {
    let report = sandbox.run("edit-history");
    assert_eq!(report[0], "", "undo empties the tab");
    assert_eq!(report[1], "draft", "redo brings the text back");
    assert!(
        report[2].starts_with("draft") && report[2].len() > "draft".len() + 8,
        "time/date is inserted at the cursor: {:?}",
        report[2]
    );
    assert_eq!(report[3], "", "select all then delete empties the tab");
}

/// Zoom changes are written to the config file and survive a clean exit.
fn zoom_persists_across_exit(sandbox: &Sandbox) {
    sandbox.run("zoom-then-exit");
    let config = fs::read_to_string(sandbox.root.join("config/rustxt/config.toml"))
        .expect("the config file was written");
    let zoom: u32 = config
        .lines()
        .find_map(|line| line.trim().strip_prefix("zoom = "))
        .expect("a zoom line")
        .parse()
        .expect("zoom is a number");
    assert!(
        zoom > 100,
        "two zoom-in steps raise the zoom above 100, got {zoom}"
    );
}

/// Closing an empty or whitespace-only untitled tab leaves nothing behind.
fn empty_and_whitespace_tabs_are_dropped(sandbox: &Sandbox) {
    let report = sandbox.run("close-blank-tabs");
    assert_eq!(report, ["1"], "only the tab with text is left open");
    let storage = sandbox.storage();
    assert!(
        storage.closed_documents().unwrap().is_empty(),
        "nothing to reopen"
    );
    let session = storage.restore_session().unwrap();
    assert_eq!(session.documents.len(), 1);
    assert_eq!(session.documents[0].content, FIRST);
}

/// What a `dump` launch reports about the window it restored.
struct Dump {
    tabs: Vec<String>,
    selected: usize,
    cursor: i32,
}

/// Scratch XDG directories and the files the child reports through.
struct Sandbox {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("create a sandbox");
        let root = dir.path().to_path_buf();
        for sub in ["config", "data", "cache", "state"] {
            fs::create_dir_all(root.join(sub)).expect("create sandbox directories");
        }
        Self { _dir: dir, root }
    }

    fn storage(&self) -> Storage {
        Storage::open(&self.root.join("data/rustxt/session.db"))
            .expect("open the recovery database")
    }

    fn ready(&self) -> PathBuf {
        self.root.join("ready")
    }

    fn out(&self) -> PathBuf {
        self.root.join("out.txt")
    }

    fn spawn(&self, scenario: &str) -> Child {
        let _ = fs::remove_file(self.ready());
        let _ = fs::remove_file(self.out());
        Command::new(env::current_exe().expect("own path"))
            .env("RUSTXT_E2E", scenario)
            .env("RUSTXT_E2E_READY", self.ready())
            .env("RUSTXT_E2E_OUT", self.out())
            .env("RUSTXT_E2E_FILE", self.root.join("notes.txt"))
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env("XDG_CACHE_HOME", self.root.join("cache"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("GSETTINGS_BACKEND", "memory")
            .spawn()
            .expect("launch the editor")
    }

    /// Run a scenario to completion and return the fields it reported.
    fn run(&self, scenario: &str) -> Vec<String> {
        let mut child = self.spawn(scenario);
        let deadline = Instant::now() + LAUNCH_TIMEOUT;
        loop {
            if let Some(status) = child.try_wait().expect("poll the editor") {
                assert!(status.success(), "scenario {scenario} failed: {status}");
                break;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                panic!("scenario {scenario} did not finish within {LAUNCH_TIMEOUT:?}");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        fs::read_to_string(self.out())
            .expect("the scenario wrote its report")
            .split(SEP)
            .map(str::to_string)
            .collect()
    }

    /// Launch, let the session restore, and read back what the window shows.
    fn dump(&self) -> Dump {
        let fields = self.run("dump");
        let selected = fields[0].parse().expect("selected tab index");
        let cursor = fields[1].parse().expect("cursor offset");
        Dump {
            tabs: fields[2..].to_vec(),
            selected,
            cursor,
        }
    }

    fn wait_for_ready(&self, child: &mut Child) {
        let deadline = Instant::now() + LAUNCH_TIMEOUT;
        while !self.ready().exists() {
            if let Some(status) = child.try_wait().expect("poll the editor") {
                panic!("editor exited before it was ready: {status}");
            }
            assert!(Instant::now() < deadline, "editor never became ready");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// The child side: the real application plus a scripted scenario. A panic
/// here aborts the process, which the parent sees as a failed launch.
mod child {
    use super::{FIRST, SECOND, SEP, TEXT};
    use adw::prelude::*;
    use gtk::{gio, glib};
    use rustxt::window::RustxtWindow;
    use rustxt_core::{config::Paths, storage::Storage};
    use std::{
        env, fs,
        path::Path,
        rc::Rc,
        time::{Duration, Instant},
    };

    pub fn run(scenario: &str) -> ! {
        let app = rustxt::application(true);
        let scenario = scenario.to_string();
        app.connect_activate(move |app| {
            let window = RustxtWindow::obtain(app);
            match scenario.as_str() {
                "type-then-hang" => type_then_hang(&window),
                "three-tabs-then-hang" => three_tabs_then_hang(&window),
                "open-file-then-hang" => open_file_then_hang(&window),
                "dump" => dump(app, &window),
                "close-reopen" => close_reopen(app, &window),
                "edit-file" => edit_file(app, &window),
                "edit-history" => edit_history(app, &window),
                "zoom-then-exit" => zoom_then_exit(&window),
                "close-blank-tabs" => close_blank_tabs(app, &window),
                other => panic!("unknown scenario {other}"),
            }
        });
        app.run_with_args::<&str>(&[]);
        std::process::exit(0);
    }

    fn type_then_hang(window: &Rc<RustxtWindow>) {
        type_text(window, TEXT);
        ready();
    }

    fn three_tabs_then_hang(window: &Rc<RustxtWindow>) {
        type_text(window, "alpha");
        act(window, "new-tab");
        settle(200);
        type_text(window, "beta");
        act(window, "new-tab");
        settle(200);
        type_text(window, "gamma");
        let tabs = tabs(window);
        tabs.set_selected_page(&tabs.nth_page(1));
        settle(200);
        let buffer = active_view(window).buffer();
        buffer.place_cursor(&buffer.iter_at_offset(2));
        // Cursor moves are flushed on the same delay as typing.
        settle(700);
        ready();
    }

    fn open_file_then_hang(window: &Rc<RustxtWindow>) {
        window.open_path(Path::new(&env::var("RUSTXT_E2E_FILE").unwrap()));
        ready();
    }

    fn dump(app: &adw::Application, window: &Rc<RustxtWindow>) {
        settle(300);
        let tabs = tabs(window);
        let selected = tabs.selected_page().expect("a selected tab");
        let mut fields = vec![
            tabs.page_position(&selected).to_string(),
            active_view(window).buffer().cursor_position().to_string(),
        ];
        for index in 0..tabs.n_pages() {
            fields.push(view_in(&tabs.nth_page(index)).buffer().text_all());
        }
        report(&fields);
        app.quit();
    }

    fn close_reopen(app: &adw::Application, window: &Rc<RustxtWindow>) {
        type_text(window, FIRST);
        act(window, "new-tab");
        settle(200);
        type_text(window, SECOND);
        act(window, "close-tab");
        settle(300);
        let after_close = tabs(window).n_pages().to_string();
        let closed_label = Storage::open(&Paths::discover().session_db())
            .unwrap()
            .closed_documents()
            .unwrap()
            .first()
            .map(|doc| doc.title.clone())
            .unwrap_or_default();
        act(window, "reopen-last");
        settle(300);
        let after_reopen = tabs(window).n_pages().to_string();
        let reopened = active_view(window).buffer().text_all();
        report(&[after_close, after_reopen, closed_label, reopened]);
        app.quit();
    }

    fn edit_file(app: &adw::Application, window: &Rc<RustxtWindow>) {
        let file = env::var("RUSTXT_E2E_FILE").unwrap();
        window.open_path(Path::new(&file));
        settle(300);
        let tabs_after_open = tabs(window).n_pages().to_string();
        window.open_path(Path::new(&file));
        settle(300);
        let tabs_after_reopen = tabs(window).n_pages().to_string();
        type_text(window, "Dear ");
        act(window, "save");
        settle(600);
        let text = active_view(window).buffer().text_all();
        report(&[tabs_after_open, tabs_after_reopen, text]);
        app.quit();
    }

    fn edit_history(app: &adw::Application, window: &Rc<RustxtWindow>) {
        type_text(window, "draft");
        act(window, "undo");
        settle(200);
        let after_undo = active_view(window).buffer().text_all();
        act(window, "redo");
        settle(200);
        let after_redo = active_view(window).buffer().text_all();
        act(window, "time-date");
        settle(200);
        let stamped = active_view(window).buffer().text_all();
        act(window, "select-all");
        settle(200);
        act(window, "delete");
        settle(600);
        let emptied = active_view(window).buffer().text_all();
        report(&[after_undo, after_redo, stamped, emptied]);
        app.quit();
    }

    fn zoom_then_exit(window: &Rc<RustxtWindow>) {
        act(window, "zoom-in");
        act(window, "zoom-in");
        settle(600);
        report(&[]);
        act(window, "exit");
    }

    fn close_blank_tabs(app: &adw::Application, window: &Rc<RustxtWindow>) {
        type_text(window, FIRST);
        act(window, "new-tab");
        settle(200);
        act(window, "close-tab");
        settle(300);
        act(window, "new-tab");
        settle(200);
        type_text(window, " \n\t\n");
        act(window, "close-tab");
        settle(300);
        report(&[tabs(window).n_pages().to_string()]);
        app.quit();
    }

    /// Insert at the cursor and wait past the snapshot delay so it is on disk.
    fn type_text(window: &Rc<RustxtWindow>, text: &str) {
        active_view(window).buffer().insert_at_cursor(text);
        settle(700);
    }

    fn act(window: &Rc<RustxtWindow>, action: &str) {
        gio::prelude::ActionGroupExt::activate_action(&window.window, action, None);
    }

    fn ready() {
        fs::write(env::var("RUSTXT_E2E_READY").unwrap(), "").unwrap();
    }

    fn report(fields: &[String]) {
        fs::write(
            env::var("RUSTXT_E2E_OUT").unwrap(),
            fields.join(&SEP.to_string()),
        )
        .unwrap();
    }

    /// Keep the GTK main loop turning for a while so timers and idle
    /// handlers, including the snapshot flush, get to run.
    fn settle(ms: u64) {
        let context = glib::MainContext::default();
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            while context.iteration(false) {}
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn tabs(window: &Rc<RustxtWindow>) -> adw::TabView {
        find::<adw::TabView>(window.window.upcast_ref()).expect("the tab strip")
    }

    fn active_view(window: &Rc<RustxtWindow>) -> sourceview5::View {
        view_in(&tabs(window).selected_page().expect("a selected tab"))
    }

    fn view_in(page: &adw::TabPage) -> sourceview5::View {
        find::<sourceview5::View>(&page.child()).expect("the editor of the tab")
    }

    fn find<T: IsA<gtk::Widget>>(root: &gtk::Widget) -> Option<T> {
        if let Ok(hit) = root.clone().downcast::<T>() {
            return Some(hit);
        }
        let mut child = root.first_child();
        while let Some(widget) = child {
            if let Some(hit) = find::<T>(&widget) {
                return Some(hit);
            }
            child = widget.next_sibling();
        }
        None
    }

    trait TextAll {
        fn text_all(&self) -> String;
    }

    impl TextAll for gtk::TextBuffer {
        fn text_all(&self) -> String {
            self.text(&self.start_iter(), &self.end_iter(), true)
                .to_string()
        }
    }
}
