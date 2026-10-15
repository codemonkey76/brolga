//! What differs from one operating system to the next: where secrets are
//! kept, where files go, and how a link is handed to the browser.

use std::path::PathBuf;

use brolga_auth::{SecretError, SecretStore};

/// The service name every secret is filed under in the system's store.
const SERVICE: &str = "brolga";

/// The operating system's own secret store: the Secret Service on Linux,
/// Credential Manager on Windows, the Keychain on macOS.
pub struct SystemSecrets;

impl SystemSecrets {
    fn entry(key: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(SERVICE, key).map_err(unavailable)
    }
}

/// The longest secret kept in one entry. Windows Credential Manager holds
/// 2,560 bytes an entry, which as UTF-16 is 1,280 characters, and a
/// Microsoft 365 refresh token is longer than that. A longer secret is kept
/// as several entries, all of them in the system's store.
const LONGEST_ENTRY: usize = if cfg!(windows) { 1000 } else { usize::MAX };

/// How the first entry of a secret kept in several begins, followed by how
/// many there are. No token starts with a control character.
const IN_PARTS: &str = "\u{1}brolga-parts:";

/// One named place in a secret store.
trait Entries {
    fn read(&self, key: &str) -> Result<Option<String>, SecretError>;
    fn write(&self, key: &str, secret: &str) -> Result<(), SecretError>;
    fn remove(&self, key: &str) -> Result<(), SecretError>;
}

fn part(key: &str, number: usize) -> String {
    format!("{key}.part{number}")
}

/// How many parts the entry under this key says it is kept in.
fn parts_of(head: Option<&str>) -> usize {
    head.and_then(|h| h.strip_prefix(IN_PARTS))
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

fn get_whole(entries: &impl Entries, key: &str) -> Result<Option<String>, SecretError> {
    let Some(head) = entries.read(key)? else {
        return Ok(None);
    };
    if !head.starts_with(IN_PARTS) {
        return Ok(Some(head));
    }

    let mut whole = String::new();
    for number in 0..parts_of(Some(&head)) {
        match entries.read(&part(key, number))? {
            Some(piece) => whole.push_str(&piece),
            // Part of it is gone, so there is no secret: the person signs in again.
            None => return Ok(None),
        }
    }

    Ok(Some(whole))
}

fn set_whole(
    entries: &impl Entries,
    key: &str,
    secret: &str,
    longest: usize,
) -> Result<(), SecretError> {
    let before = parts_of(entries.read(key)?.as_deref());
    let characters: Vec<char> = secret.chars().collect();
    let pieces: Vec<String> = if characters.len() <= longest {
        Vec::new()
    } else {
        characters
            .chunks(longest.max(1))
            .map(|piece| piece.iter().collect())
            .collect()
    };

    // The parts first and the entry that counts them last, so that what
    // is read is always a whole secret: the old one or the new.
    for (number, piece) in pieces.iter().enumerate() {
        entries.write(&part(key, number), piece)?;
    }
    if pieces.is_empty() {
        entries.write(key, secret)?;
    } else {
        entries.write(key, &format!("{IN_PARTS}{}", pieces.len()))?;
    }
    for number in pieces.len()..before {
        entries.remove(&part(key, number))?;
    }

    Ok(())
}

fn delete_whole(entries: &impl Entries, key: &str) -> Result<(), SecretError> {
    let before = parts_of(entries.read(key)?.as_deref());
    entries.remove(key)?;
    for number in 0..before {
        entries.remove(&part(key, number))?;
    }

    Ok(())
}

impl Entries for SystemSecrets {
    fn read(&self, key: &str) -> Result<Option<String>, SecretError> {
        match Self::entry(key)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(unavailable(e)),
        }
    }

    fn write(&self, key: &str, secret: &str) -> Result<(), SecretError> {
        Self::entry(key)?.set_password(secret).map_err(unavailable)
    }

    fn remove(&self, key: &str) -> Result<(), SecretError> {
        match Self::entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(unavailable(e)),
        }
    }
}

impl SecretStore for SystemSecrets {
    fn get(&self, key: &str) -> Result<Option<String>, SecretError> {
        get_whole(self, key)
    }

    fn set(&self, key: &str, secret: &str) -> Result<(), SecretError> {
        set_whole(self, key, secret, LONGEST_ENTRY)
    }

    fn delete(&self, key: &str) -> Result<(), SecretError> {
        delete_whole(self, key)
    }
}

fn unavailable(error: keyring::Error) -> SecretError {
    SecretError::Unavailable(error.to_string())
}

/// Where Brolga keeps its data for this user: the XDG data directory on
/// Linux, and the usual application data folder on Windows and macOS.
pub fn data_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("io.github", "codemonkey76", "Brolga")
        .map(|dirs| dirs.data_dir().to_path_buf())
}

/// The computer's own name, as its owner would know it, where it can be
/// found: what a copy of Brolga is first called when other devices show it.
pub fn host_name() -> Option<String> {
    #[cfg(target_os = "linux")]
    let name = std::fs::read_to_string("/proc/sys/kernel/hostname").ok();
    #[cfg(target_os = "windows")]
    let name = std::env::var("COMPUTERNAME").ok();
    #[cfg(target_os = "macos")]
    let name = std::process::Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok());
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    let name: Option<String> = None;

    name.map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
}

/// Where this user's downloads go, for saved attachments.
pub fn downloads_dir() -> Option<PathBuf> {
    directories::UserDirs::new().and_then(|dirs| dirs.download_dir().map(PathBuf::from))
}

/// Hand a link to the system's default browser.
///
/// An AppImage runs with its own libraries and programs put ahead of the
/// system's, in the environment. A browser started with those looks for
/// its libraries in the AppImage and does not start at all, so the browser
/// is given the environment as it was outside.
///
/// Outside every AppImage, not only this one: a copy started by the copy
/// before it, as after an update, has that copy's places in its environment
/// behind its own, and the one before that's behind those.
pub fn open_in_browser(url: &str) -> std::io::Result<()> {
    let Some(inside) = std::env::var_os("APPDIR").filter(|dir| !dir.is_empty()) else {
        return open::that_detached(url);
    };
    let inside = inside.to_string_lossy().into_owned();
    let changes = outside_of(
        std::env::vars_os().map(|(name, value)| {
            (
                name.to_string_lossy().into_owned(),
                value.to_string_lossy().into_owned(),
            )
        }),
        &inside,
    );

    let mut last = std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "there is no program to open a link with",
    );
    for mut opener in open::commands(url) {
        for (name, value) in &changes {
            match value {
                Some(value) => opener.env(name, value),
                None => opener.env_remove(name),
            };
        }
        // Not left standing in the AppImage either, which is where an
        // AppImage's program starts: a browser that stayed there would
        // keep the AppImage from being put away when Brolga closes.
        opener.current_dir(elsewhere(&inside));
        opener
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        match opener.spawn() {
            Ok(mut started) => {
                // Waited for elsewhere, so that it is not left half-ended.
                std::thread::spawn(move || {
                    let _ = started.wait();
                });
                return Ok(());
            }
            Err(error) => last = error,
        }
    }

    Err(last)
}

/// A directory to start a program in that is not inside an AppImage: the
/// person's own, or failing that the top of the file system.
fn elsewhere(inside: &str) -> PathBuf {
    let within = mounted_like(inside);
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_absolute() && home.is_dir())
        .filter(|home| !within(&home.to_string_lossy()))
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Whether a place is inside an AppImage, given where this one is mounted.
///
/// An AppImage is mounted for as long as it runs at a directory named
/// `.mount_` and some letters, in the directory for temporary files. This
/// one's is `inside`, and any directory so named beside it is another's:
/// the copy this one was started from, most likely.
fn mounted_like(inside: &str) -> impl Fn(&str) -> bool {
    let inside = inside.trim_end_matches('/').to_owned();
    let beside = inside
        .rsplit_once('/')
        .filter(|(_, name)| name.starts_with(".mount_"))
        .map(|(parent, _)| format!("{parent}/.mount_"));

    move |place: &str| {
        place.starts_with(inside.as_str())
            || beside
                .as_deref()
                .is_some_and(|beside| place.starts_with(beside))
    }
}

/// What to change in an environment so that nothing in it points inside
/// an AppImage, this one being mounted at `inside`: for each variable that
/// does, its value with those places taken out of its list, or `None`
/// where nothing is left of it.
fn outside_of(
    environment: impl Iterator<Item = (String, String)>,
    inside: &str,
) -> Vec<(String, Option<String>)> {
    let within = mounted_like(inside);
    environment
        .filter(|(name, _)| name != "APPDIR")
        .filter_map(|(name, value)| {
            let kept: Vec<&str> = value.split(':').filter(|place| !within(place)).collect();
            if kept.len() == value.split(':').count() {
                return None;
            }
            let left = kept.join(":");
            let left = left.trim_matches(':');
            Some((name, (!left.is_empty()).then(|| left.to_owned())))
        })
        .chain([("APPDIR".to_owned(), None), ("APPIMAGE".to_owned(), None)])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_browser_is_started_with_nothing_of_the_appimage_in_its_environment() {
        let app = "/tmp/.mount_Brolga1";
        let inside = [
            (
                "PATH",
                "/tmp/.mount_Brolga1/usr/bin/:/tmp/.mount_Brolga1/bin/:/usr/local/bin:/usr/bin",
            ),
            (
                "LD_LIBRARY_PATH",
                "/tmp/.mount_Brolga1/usr/lib/:/tmp/.mount_Brolga1/lib/",
            ),
            ("XDG_DATA_DIRS", "/tmp/.mount_Brolga1/usr/share/:/usr/share"),
            ("GTK_EXE_PREFIX", "/tmp/.mount_Brolga1//usr"),
            (
                "GST_PLUGIN_SYSTEM_PATH",
                "/tmp/.mount_Brolga1/usr/lib/gstreamer:",
            ),
            ("HOME", "/home/robin"),
            ("APPDIR", "/tmp/.mount_Brolga1"),
        ];
        let mut changes = outside_of(
            inside
                .iter()
                .map(|(n, v)| ((*n).to_owned(), (*v).to_owned())),
            app,
        );
        changes.sort();

        let now = |name: &str| {
            changes
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(
            now("PATH"),
            Some(Some("/usr/local/bin:/usr/bin".to_owned()))
        );
        assert_eq!(now("XDG_DATA_DIRS"), Some(Some("/usr/share".to_owned())));
        for gone in [
            "LD_LIBRARY_PATH",
            "GTK_EXE_PREFIX",
            "GST_PLUGIN_SYSTEM_PATH",
            "APPDIR",
            "APPIMAGE",
        ] {
            assert_eq!(now(gone), Some(None), "{gone}");
        }
        // What never pointed inside is left alone.
        assert_eq!(now("HOME"), None);
    }

    #[test]
    fn a_browser_is_started_with_nothing_of_the_appimages_this_one_was_started_from() {
        // As it is after two updates: this copy's places first, then those
        // of the copy that started it, then those of the one before.
        let app = "/tmp/.mount_brolgaBMBoFJ";
        let inside = [
            (
                "PATH",
                "/tmp/.mount_brolgaBMBoFJ/usr/bin/:/tmp/.mount_brolgaOodGMl/usr/bin/:/tmp/.mount_brolgaMfCmCo/usr/bin/:/usr/local/bin:/usr/bin",
            ),
            (
                "LD_LIBRARY_PATH",
                "/tmp/.mount_brolgaBMBoFJ/usr/lib/:/tmp/.mount_brolgaOodGMl/usr/lib/:/tmp/.mount_brolgaMfCmCo/usr/lib/",
            ),
            (
                "XDG_DATA_DIRS",
                "/tmp/.mount_brolgaBMBoFJ/usr/share/:/usr/share:/tmp/.mount_brolgaOodGMl/usr/share/:/usr/share:",
            ),
            (
                "PYTHONPATH",
                "/tmp/.mount_brolgaOodGMl/usr/share/pyshared/:",
            ),
            ("PWD", "/tmp/.mount_brolgaOodGMl/usr"),
            // Temporary files of other programs are not an AppImage's.
            ("SOCKET", "/tmp/.mounting/socket:/tmp/other"),
            ("HOME", "/home/robin"),
            ("APPDIR", "/tmp/.mount_brolgaBMBoFJ"),
        ];
        let changes = outside_of(
            inside
                .iter()
                .map(|(n, v)| ((*n).to_owned(), (*v).to_owned())),
            app,
        );
        let now = |name: &str| {
            changes
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
        };

        assert_eq!(
            now("PATH"),
            Some(Some("/usr/local/bin:/usr/bin".to_owned()))
        );
        assert_eq!(
            now("XDG_DATA_DIRS"),
            Some(Some("/usr/share:/usr/share".to_owned()))
        );
        for gone in ["LD_LIBRARY_PATH", "PYTHONPATH", "PWD", "APPDIR", "APPIMAGE"] {
            assert_eq!(now(gone), Some(None), "{gone}");
        }
        assert_eq!(now("SOCKET"), None);
        assert_eq!(now("HOME"), None);
        // Nothing of any AppImage is left in anything.
        assert!(
            changes
                .iter()
                .all(|(_, value)| !value.as_deref().unwrap_or_default().contains(".mount_")),
            "{changes:?}"
        );
    }

    #[test]
    fn an_appimage_that_is_not_mounted_as_one_takes_out_only_its_own_places() {
        // Unpacked and run from a folder, there is no mount to know others by.
        let changes = outside_of(
            [
                (
                    "PATH",
                    "/opt/brolga/usr/bin:/tmp/.mount_other1/usr/bin:/usr/bin",
                ),
                ("APPDIR", "/opt/brolga"),
            ]
            .iter()
            .map(|(n, v)| ((*n).to_owned(), (*v).to_owned())),
            "/opt/brolga",
        );

        assert!(changes.contains(&(
            "PATH".to_owned(),
            Some("/tmp/.mount_other1/usr/bin:/usr/bin".to_owned())
        )));
    }

    /// A store that holds only so much in an entry, as Windows does.
    struct Small(
        std::sync::Mutex<std::collections::BTreeMap<String, String>>,
        usize,
    );

    impl Entries for Small {
        fn read(&self, key: &str) -> Result<Option<String>, SecretError> {
            Ok(self.0.lock().unwrap().get(key).cloned())
        }

        fn write(&self, key: &str, secret: &str) -> Result<(), SecretError> {
            if secret.encode_utf16().count() > self.1 {
                return Err(SecretError::Unavailable(
                    "longer than the platform limit".to_owned(),
                ));
            }
            self.0
                .lock()
                .unwrap()
                .insert(key.to_owned(), secret.to_owned());
            Ok(())
        }

        fn remove(&self, key: &str) -> Result<(), SecretError> {
            self.0.lock().unwrap().remove(key);
            Ok(())
        }
    }

    #[test]
    fn a_secret_longer_than_an_entry_holds_is_kept_in_parts_and_comes_back_whole() {
        let store = Small(Default::default(), 1280);
        let token: String = (0..5000)
            .map(|n| char::from(b'a' + (n % 26) as u8))
            .collect();

        // As it failed on Windows: too long for one entry.
        assert!(store.write("account", &token).is_err());

        set_whole(&store, "account", &token, 1000).unwrap();
        assert_eq!(
            get_whole(&store, "account").unwrap().as_deref(),
            Some(token.as_str())
        );
        assert_eq!(store.0.lock().unwrap().len(), 6);

        // A shorter one after it leaves no parts of the longer behind.
        set_whole(&store, "account", &token[..1500], 1000).unwrap();
        assert_eq!(
            get_whole(&store, "account").unwrap().as_deref(),
            Some(&token[..1500])
        );
        assert_eq!(store.0.lock().unwrap().len(), 3);
        set_whole(&store, "account", "short", 1000).unwrap();
        assert_eq!(
            get_whole(&store, "account").unwrap().as_deref(),
            Some("short")
        );
        assert_eq!(store.0.lock().unwrap().len(), 1);

        // Forgotten, parts and all; and a secret missing a part is no secret.
        set_whole(&store, "account", &token, 1000).unwrap();
        store.remove("account.part2").unwrap();
        assert_eq!(get_whole(&store, "account").unwrap(), None);
        delete_whole(&store, "account").unwrap();
        assert!(store.0.lock().unwrap().is_empty());
        assert_eq!(get_whole(&store, "account").unwrap(), None);
    }

    /// Against the real secret store of whoever runs it, so it is not part
    /// of the ordinary run: `cargo test -p brolga-platform -- --ignored`.
    #[test]
    #[ignore = "writes to, and cleans up after itself in, this user's real secret store"]
    fn a_secret_goes_into_the_system_store_and_comes_back_out() {
        let key = format!("test.{}", std::process::id());
        let store = SystemSecrets;

        assert_eq!(store.get(&key).unwrap(), None);
        store.set(&key, "first").unwrap();
        store.set(&key, "second").unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("second"));
        store.delete(&key).unwrap();
        assert_eq!(store.get(&key).unwrap(), None);
        // Forgetting what is not there is not an error.
        store.delete(&key).unwrap();

        // As long as a Microsoft 365 refresh token, which is longer than
        // Windows keeps in one entry.
        let long: String = (0..5000)
            .map(|n| char::from(b'a' + (n % 26) as u8))
            .collect();
        store.set(&key, &long).unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some(long.as_str()));
        store.set(&key, "short again").unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("short again"));
        store.delete(&key).unwrap();
        assert_eq!(store.get(&key).unwrap(), None);
    }
}
