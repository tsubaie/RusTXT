//! Checking GitHub for a newer release and, for per-user installs, putting
//! it in place. Network, checksum and archive work go through curl,
//! sha256sum and tar, which every Linux install already has, so the app
//! carries no HTTP stack of its own. Nothing here runs unless the user asks.

use serde::Deserialize;
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

pub const REPOSITORY: &str = "https://github.com/tsubaie/RusTXT";
const LATEST_RELEASE_API: &str = "https://api.github.com/repos/tsubaie/RusTXT/releases/latest";
const CHECKSUMS: &str = "SHA256SUMS";

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    #[serde(rename = "browser_download_url")]
    pub url: String,
}

/// What GitHub says about the latest release.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Release {
    #[serde(rename = "tag_name")]
    pub tag: String,
    /// The release page, with the notes.
    #[serde(rename = "html_url")]
    pub url: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

impl Release {
    /// The tag without its leading `v`.
    pub fn version(&self) -> &str {
        self.tag.trim().trim_start_matches('v')
    }

    pub fn is_newer_than(&self, current: &str) -> bool {
        let Ok(candidate) = semver::Version::parse(self.version()) else {
            return false;
        };
        semver::Version::parse(current).is_ok_and(|current| candidate > current)
    }

    /// The name of the tarball built for this machine.
    pub fn tarball_name(&self) -> String {
        format!(
            "rustxt-{}-{}-{}.tar.gz",
            self.version(),
            env::consts::ARCH,
            env::consts::OS
        )
    }

    fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|asset| asset.name == name)
    }
}

pub fn parse_release(json: &str) -> Result<Release, String> {
    serde_json::from_str(json).map_err(|error| format!("Unexpected reply from GitHub: {error}"))
}

/// Ask GitHub for the latest release. Blocks; call it off the UI thread.
pub fn fetch_latest(current_version: &str) -> Result<Release, String> {
    let reply = curl(&[
        "-H",
        "Accept: application/vnd.github+json",
        "-A",
        &format!("rustxt/{current_version}"),
        LATEST_RELEASE_API,
    ])?;
    parse_release(&reply)
}

/// Where the running binary came from, which decides how it can be updated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Install {
    /// In the user's own bin directory: the tarball or the quick installer
    /// put it there, and the app can replace it itself.
    PerUser(PathBuf),
    /// Under /usr or /opt: a package manager owns it and delivers updates.
    Packaged(PathBuf),
    /// Under ~/.cargo/bin: `cargo install` built it.
    Cargo(PathBuf),
    /// Anywhere else, such as a source checkout.
    Other(PathBuf),
}

impl Install {
    pub fn detect() -> Self {
        let exe = env::current_exe()
            .and_then(fs::canonicalize)
            .unwrap_or_default();
        let home = env::var_os("HOME").map(PathBuf::from);
        Self::classify(&exe, home.as_deref())
    }

    pub fn classify(exe: &Path, home: Option<&Path>) -> Self {
        let path = exe.to_path_buf();
        if let Some(home) = home {
            if exe.starts_with(home.join(".cargo/bin")) {
                return Self::Cargo(path);
            }
            if exe.starts_with(home.join(".local/bin")) || exe.starts_with(home.join("bin")) {
                return Self::PerUser(path);
            }
        }
        if exe.starts_with("/usr") || exe.starts_with("/opt") {
            return Self::Packaged(path);
        }
        Self::Other(path)
    }

    /// The binary the app may replace itself, if any.
    pub fn replaceable_binary(&self) -> Option<&Path> {
        match self {
            Self::PerUser(path) => Some(path),
            _ => None,
        }
    }

    /// How to get the update when the app cannot install it itself.
    pub fn hint(&self) -> Option<&'static str> {
        match self {
            Self::PerUser(_) => None,
            Self::Packaged(_) => {
                Some("Your package manager installed RusTXT and will deliver the update.")
            }
            Self::Cargo(_) => Some("This build came from cargo install. Run it again to update."),
            Self::Other(_) => Some("Download the release, or rebuild from source."),
        }
    }
}

/// Download the tarball built for this machine, check it against the
/// release's checksums, and swap the binary at `target` for the new one.
/// The running process keeps its old executable until it exits.
pub fn install(release: &Release, target: &Path) -> Result<(), String> {
    let name = release.tarball_name();
    let tarball = release.asset(&name).ok_or_else(|| {
        format!(
            "Version {} has no build for {} {}.",
            release.version(),
            env::consts::ARCH,
            env::consts::OS
        )
    })?;
    let sums = release
        .asset(CHECKSUMS)
        .ok_or_else(|| format!("Version {} ships no checksums.", release.version()))?;
    let expected_prefix = format!(
        "https://github.com/tsubaie/RusTXT/releases/download/{}/",
        release.tag
    );
    let trusted =
        |url: &str| url.starts_with(&expected_prefix) || (cfg!(test) && url.starts_with("file://"));
    if !trusted(&tarball.url) || !trusted(&sums.url) {
        return Err("GitHub returned an unexpected release download location.".into());
    }

    let work = tempfile::tempdir().map_err(text)?;
    let archive = work.path().join(&name);
    curl(&["-o", &archive.to_string_lossy(), &tarball.url])?;
    verify_checksum(&archive, &curl(&[&sums.url])?, &name)?;

    let member = format!("rustxt-{}/rustxt", release.version());
    // Extract only the expected member to stdout. This prevents an otherwise
    // valid archive from using absolute or `..` paths to write outside `work`.
    let output = Command::new("tar")
        .arg("-xOzf")
        .arg(&archive)
        .arg(&member)
        .output()
        .map_err(|error| format!("tar is needed to unpack the update: {error}"))?;
    if !output.status.success() {
        return Err("The download could not be unpacked.".into());
    }
    if output.stdout.is_empty() {
        return Err("The download did not contain the rustxt binary.".into());
    }
    let binary = work.path().join("rustxt");
    fs::write(&binary, output.stdout).map_err(text)?;
    replace_binary(&binary, target)
}

fn verify_checksum(file: &Path, sums: &str, name: &str) -> Result<(), String> {
    let expected = sums
        .lines()
        .filter_map(|line| line.split_once("  "))
        .find(|(_, entry)| entry.trim() == name)
        .map(|(hash, _)| hash.trim().to_ascii_lowercase())
        .ok_or_else(|| format!("{CHECKSUMS} has no entry for {name}."))?;
    let output = Command::new("sha256sum")
        .arg(file)
        .output()
        .map_err(|error| format!("sha256sum is needed to verify the update: {error}"))?;
    let actual = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !output.status.success()
        || expected.len() != 64
        || !expected.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!(
            "{CHECKSUMS} contains an invalid checksum for {name}."
        ));
    }
    if actual != expected {
        return Err("The download did not match its checksum, so it was not installed.".into());
    }
    Ok(())
}

/// Copy next to the target, then rename over it, so the swap is atomic and
/// a running copy of the old binary is unaffected.
fn replace_binary(new: &Path, target: &Path) -> Result<(), String> {
    let staged = target.with_extension("new");
    fs::copy(new, &staged)
        .map_err(|error| format!("Could not write next to {}: {error}", target.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o755)).map_err(text)?;
    }
    fs::rename(&staged, target)
        .map_err(|error| format!("Could not replace {}: {error}", target.display()))
}

fn curl(args: &[&str]) -> Result<String, String> {
    let output = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--max-time",
            "60",
        ])
        .args(args)
        .output()
        .map_err(|error| format!("curl is needed to reach GitHub: {error}"))?;
    if !output.status.success() {
        let reason = String::from_utf8_lossy(&output.stderr);
        let reason = reason.trim().trim_start_matches("curl: ");
        return Err(format!("Could not reach GitHub: {reason}"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn text(error: impl ToString) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "tag_name": "v0.4.0",
        "html_url": "https://github.com/tsubaie/RusTXT/releases/tag/v0.4.0",
        "body": "notes",
        "assets": [
            {"name": "SHA256SUMS", "browser_download_url": "https://example.test/SHA256SUMS", "size": 1},
            {"name": "rustxt-0.4.0-x86_64-linux.tar.gz", "browser_download_url": "https://example.test/t.tar.gz"}
        ]
    }"#;

    #[test]
    fn parses_what_github_sends_and_ignores_the_rest() {
        let release = parse_release(SAMPLE).unwrap();
        assert_eq!(release.version(), "0.4.0");
        assert_eq!(
            release.url,
            "https://github.com/tsubaie/RusTXT/releases/tag/v0.4.0"
        );
        assert_eq!(release.assets.len(), 2);
        assert!(parse_release("not json").is_err());
    }

    #[test]
    fn newer_means_numerically_newer() {
        let release = |tag: &str| Release {
            tag: tag.into(),
            url: String::new(),
            assets: vec![],
        };
        assert!(release("v0.4.0").is_newer_than("0.3.0"));
        assert!(release("0.10.0").is_newer_than("0.9.9"));
        assert!(!release("v0.3.0").is_newer_than("0.3.0"));
        assert!(!release("v0.2.9").is_newer_than("0.3.0"));
        assert!(release("v1.0.0").is_newer_than("0.99.0"));
        assert!(!release("v1.0.0-beta.1").is_newer_than("1.0.0"));
        assert!(!release("not-a-version").is_newer_than("1.0.0"));
    }

    #[test]
    fn install_kind_follows_the_binary_location() {
        let home = Path::new("/home/me");
        let classify = |path: &str| Install::classify(Path::new(path), Some(home));
        assert!(matches!(
            classify("/home/me/.local/bin/rustxt"),
            Install::PerUser(_)
        ));
        assert!(matches!(
            classify("/home/me/bin/rustxt"),
            Install::PerUser(_)
        ));
        assert!(matches!(
            classify("/home/me/.cargo/bin/rustxt"),
            Install::Cargo(_)
        ));
        assert!(matches!(classify("/usr/bin/rustxt"), Install::Packaged(_)));
        assert!(matches!(
            classify("/usr/local/bin/rustxt"),
            Install::Packaged(_)
        ));
        assert!(matches!(
            classify("/opt/rustxt/rustxt"),
            Install::Packaged(_)
        ));
        assert!(matches!(
            classify("/home/me/src/target/release/rustxt"),
            Install::Other(_)
        ));
        assert!(classify("/home/me/.local/bin/rustxt")
            .replaceable_binary()
            .is_some());
        assert!(classify("/usr/bin/rustxt").hint().is_some());
    }

    /// A release served from local files, fetched through curl like the real
    /// thing, unpacked and swapped into place.
    fn fake_release(dir: &Path, version: &str, tamper: bool) -> Release {
        let tag = format!("v{version}");
        let release = Release {
            tag: tag.clone(),
            url: String::new(),
            assets: vec![],
        };
        let name = release.tarball_name();
        let payload = dir.join(format!("rustxt-{version}"));
        fs::create_dir_all(&payload).unwrap();
        fs::write(
            payload.join("rustxt"),
            format!("#!/bin/sh\necho {version}\n"),
        )
        .unwrap();
        assert!(Command::new("tar")
            .args(["-czf", &name, &format!("rustxt-{version}")])
            .current_dir(dir)
            .status()
            .unwrap()
            .success());
        let sum = Command::new("sha256sum")
            .arg(&name)
            .current_dir(dir)
            .output()
            .unwrap();
        let mut sums = String::from_utf8(sum.stdout).unwrap();
        if tamper {
            sums = sums
                .replacen(char::is_alphanumeric, "0", 1)
                .replacen('0', "1", 1);
        }
        fs::write(dir.join(CHECKSUMS), format!("deadbeef  other.deb\n{sums}")).unwrap();
        let file_url = |file: &str| format!("file://{}", dir.join(file).display());
        Release {
            tag,
            url: String::new(),
            assets: vec![
                Asset {
                    name: CHECKSUMS.into(),
                    url: file_url(CHECKSUMS),
                },
                Asset {
                    name: name.clone(),
                    url: file_url(&name),
                },
            ],
        }
    }

    #[test]
    fn installs_a_verified_tarball_over_the_old_binary() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("bin/rustxt");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, "old").unwrap();
        let release = fake_release(&dir.path().join("release"), "9.9.9", false);

        install(&release, &target).unwrap();

        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "#!/bin/sh\necho 9.9.9\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&target).unwrap().permissions().mode() & 0o111,
                0o111
            );
        }
        assert!(
            !target.with_extension("new").exists(),
            "no staging file left behind"
        );
    }

    #[test]
    fn a_bad_checksum_leaves_the_old_binary_alone() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("rustxt");
        fs::write(&target, "old").unwrap();
        let release = fake_release(&dir.path().join("release"), "9.9.9", true);

        let error = install(&release, &target).unwrap_err();

        assert!(error.contains("checksum"), "{error}");
        assert_eq!(fs::read_to_string(&target).unwrap(), "old");
    }

    #[test]
    fn a_release_without_a_build_for_this_machine_is_refused() {
        let release = Release {
            tag: "v9.9.9".into(),
            url: String::new(),
            assets: vec![],
        };
        let error = install(&release, Path::new("/nonexistent/rustxt")).unwrap_err();
        assert!(error.contains("no build for"), "{error}");
    }
}
