//! The RustPad -> RusTXT rename must carry existing settings and recovery data over.
use std::fs;

#[test]
fn old_rustpad_dirs_are_adopted_when_new_ones_are_missing() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let data = root.path().join("data");
    fs::create_dir_all(config.join("rustpad/themes")).unwrap();
    fs::create_dir_all(data.join("rustpad")).unwrap();
    fs::write(config.join("rustpad/config.toml"), "theme = \"x\"\n").unwrap();
    fs::write(data.join("rustpad/session.db"), b"db").unwrap();
    std::env::set_var("XDG_CONFIG_HOME", &config);
    std::env::set_var("XDG_DATA_HOME", &data);
    std::env::set_var("XDG_CACHE_HOME", root.path().join("cache"));

    let paths = rustxt_core::config::Paths::discover();

    assert_eq!(paths.config_dir, config.join("rustxt"));
    assert_eq!(
        fs::read_to_string(paths.config_file()).unwrap(),
        "theme = \"x\"\n"
    );
    assert!(paths.themes_dir().is_dir());
    assert_eq!(fs::read(paths.session_db()).unwrap(), b"db");
    assert!(!config.join("rustpad").exists());
    assert!(!data.join("rustpad").exists());
}
