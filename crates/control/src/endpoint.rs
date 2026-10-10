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
}

/// The endpoint a client started by this user listens on.
///
/// On Linux and macOS it is a socket in a directory only this user can enter,
/// which is what keeps other users of the machine out.
#[cfg(unix)]
pub fn default_endpoint() -> io::Result<Endpoint> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    // XDG_RUNTIME_DIR on Linux and TMPDIR on macOS are each already private
    // to the user. Without either, fall back to the home directory.
    let base = ["XDG_RUNTIME_DIR", "TMPDIR"]
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .find(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "no runtime or home directory to put the socket in",
            )
        })?;

    let dir = base.join("brolga");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)?;
    // The directory may be left from an older run with looser permissions.
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;

    Ok(Endpoint::Path(dir.join("control.sock")))
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
