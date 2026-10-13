use std::io;
use std::path::PathBuf;

use interprocess::local_socket::Name;

/// Where the control channel listens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// A Unix socket file. Linux and macOS.
    Path(PathBuf),
    /// A named pipe. Windows.
    Pipe(String),
}

impl Endpoint {
    pub(crate) fn name(&self) -> io::Result<Name<'_>> {
        use interprocess::local_socket::{GenericFilePath, GenericNamespaced, ToFsName, ToNsName};

        match self {
            Self::Path(path) => path.as_path().to_fs_name::<GenericFilePath>(),
            Self::Pipe(name) => name.as_str().to_ns_name::<GenericNamespaced>(),
        }
    }

    /// The file the client listening here keeps its key in, where a caller
    /// must be shown that what answers is that client (see `proof`).
    ///
    /// A socket file needs none: the directory it is in is its user's own.
    /// A pipe's name is anyone's to take, so its key is kept in the user's
    /// own application data, which other users of the machine cannot read.
    pub(crate) fn key_file(&self) -> io::Result<Option<PathBuf>> {
        match self {
            Self::Path(_) => Ok(None),
            Self::Pipe(name) => std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|data| Some(data.join("Brolga").join(format!("{name}.key"))))
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        "LOCALAPPDATA is not set, so there is nowhere to keep the control channel's key",
                    )
                }),
        }
    }
}

/// The endpoint a client started by this user listens on.
///
/// On Linux and macOS it is a socket in a directory only this user can enter,
/// which is what keeps other users of the machine out.
#[cfg(unix)]
pub fn default_endpoint() -> io::Result<Endpoint> {
    let var = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    // XDG_RUNTIME_DIR on Linux and TMPDIR on macOS are each already private
    // to the user. TMPDIR anywhere else is `/tmp`, which everybody shares,
    // so without a runtime directory Linux falls back to the user's own
    // home directory.
    let base = var("XDG_RUNTIME_DIR")
        .or_else(|| {
            if cfg!(target_os = "macos") {
                var("TMPDIR")
            } else {
                None
            }
        })
        .or_else(|| var("HOME").map(|home| home.join(".cache")))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "no runtime or home directory to put the socket in",
            )
        })?;

    // Inside a Flatpak each start of the app has a runtime directory of its
    // own, and a second copy would never find the first. The one place they
    // share, and that the system keeps to this app, is `app/<its id>`.
    let base = match std::env::var_os("FLATPAK_ID") {
        Some(id) if !id.is_empty() => base.join("app").join(id),
        _ => base,
    };

    Ok(Endpoint::Path(
        private_directory(&base, current_uid()?)?.join("control.sock"),
    ))
}

/// The `brolga` directory under `base`, made if it is not there, and held
/// to being this user's own: a real directory, not a link to one, owned by
/// the user and closed to everyone else. Anything else is refused, since a
/// directory someone else made first is theirs to listen in.
#[cfg(unix)]
fn private_directory(base: &std::path::Path, user: u32) -> io::Result<PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

    std::fs::create_dir_all(base)?;
    let dir = base.join("brolga");
    let refused = |why: &str| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "{} {why}, so the control channel will not be put there",
                dir.display()
            ),
        )
    };

    match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    // Looked at without following a link, so that a link put there by
    // someone else is seen for what it is.
    let found = std::fs::symlink_metadata(&dir)?;
    if !found.file_type().is_dir() {
        return Err(refused("is not a real directory"));
    }
    if found.uid() != user {
        return Err(refused("belongs to another user"));
    }
    if found.mode() & 0o777 != 0o700 {
        // Ours, left from an older run with looser permissions.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        if std::fs::symlink_metadata(&dir)?.mode() & 0o777 != 0o700 {
            return Err(refused("could not be closed to other users"));
        }
    }

    Ok(dir)
}

/// The user this process runs as. The standard library does not say, and
/// this workspace has no unsafe code to ask the system with, so a file is
/// made and its owner read: what a process makes belongs to the user it
/// runs as.
#[cfg(unix)]
fn current_uid() -> io::Result<u32> {
    use std::os::unix::fs::MetadataExt;

    static UID: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    if let Some(uid) = UID.get() {
        return Ok(*uid);
    }
    // `/proc/self` is owned by the user the process runs as, where there
    // is a `/proc`.
    if cfg!(target_os = "linux")
        && let Ok(own) = std::fs::metadata("/proc/self")
    {
        return Ok(*UID.get_or_init(|| own.uid()));
    }

    let probe = std::env::temp_dir().join(format!(
        "brolga-uid-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.subsec_nanos())
    ));
    // `create_new` fails on anything already there, a link included, so
    // the file read is one this process made.
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)?;
    let uid = file.metadata().map(|made| made.uid());
    drop(file);
    let _ = std::fs::remove_file(&probe);
    let uid = uid?;

    Ok(*UID.get_or_init(|| uid))
}

#[cfg(windows)]
pub fn default_endpoint() -> io::Result<Endpoint> {
    let user = std::env::var("USERNAME").map_err(|_| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "USERNAME is not set, so the pipe cannot be named",
        )
    })?;

    Ok(Endpoint::Pipe(format!("brolga-control-{user}")))
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let base =
            std::env::temp_dir().join(format!("brolga-endpoint-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn the_sockets_directory_is_made_for_this_user_alone() {
        let base = scratch("made");

        let dir = private_directory(&base, current_uid().unwrap()).unwrap();

        let found = std::fs::symlink_metadata(&dir).unwrap();
        assert!(found.is_dir());
        assert_eq!(found.mode() & 0o777, 0o700);
        assert_eq!(found.uid(), current_uid().unwrap());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_directory_left_open_is_closed_and_a_link_is_refused() {
        let base = scratch("open");
        std::fs::create_dir(base.join("brolga")).unwrap();
        std::fs::set_permissions(base.join("brolga"), std::fs::Permissions::from_mode(0o777))
            .unwrap();

        let me = current_uid().unwrap();
        let dir = private_directory(&base, me).unwrap();
        assert_eq!(std::fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);

        // Someone got there first and pointed the name at a directory of theirs.
        let base = scratch("link");
        let theirs = scratch("theirs");
        std::os::unix::fs::symlink(&theirs, base.join("brolga")).unwrap();

        let refused = private_directory(&base, me).unwrap_err();

        assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);
        assert!(
            refused.to_string().contains("not a real directory"),
            "{refused}"
        );
        for each in [base, theirs, scratch("open")] {
            let _ = std::fs::remove_dir_all(each);
        }
    }

    #[test]
    fn a_directory_that_is_someone_elses_is_refused() {
        let base = scratch("theirs-first");
        std::fs::create_dir(base.join("brolga")).unwrap();
        // The directory is this test's, so the user it is held to is another.
        let someone_else = current_uid().unwrap() + 1;

        let refused = private_directory(&base, someone_else).unwrap_err();

        assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);
        assert!(
            refused.to_string().contains("belongs to another user"),
            "{refused}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_user_is_the_one_who_owns_what_this_process_makes() {
        let made = scratch("mine");

        assert_eq!(
            current_uid().unwrap(),
            std::fs::metadata(&made).unwrap().uid()
        );
        let _ = std::fs::remove_dir_all(&made);
    }
}
