//! Writing an attachment to disk.
//!
//! The name of an attachment is chosen by whoever sent the message, so it is
//! never used as a path: it is cut down to a plain file name first. Nothing
//! already on disk is ever written over, and nothing saved is opened or run.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The longest file name written, in characters, extension included.
const LONGEST: usize = 120;

/// A plain file name made from what a sender called an attachment: no
/// folders, nothing hidden, nothing the file system gives a meaning to.
pub fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        // Marks that change the direction of text can make `exe.txt` read
        // as `txt.exe`, and ones that take no room can sit after `.exe`
        // unseen, so they are dropped.
        .filter(|c| {
            !matches!(c,
                '\u{061C}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
        })
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    // Windows drops trailing dots and spaces; a leading dot hides the file.
    let cleaned = cleaned
        .trim()
        .trim_end_matches(['.', ' '])
        .trim_start_matches('.')
        .trim();
    if cleaned.is_empty() {
        return "attachment".to_owned();
    }

    let (stem, extension) = split(cleaned);
    // Names Windows keeps for devices, whatever extension follows them.
    const DEVICES: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9", "COM¹",
        "COM²", "COM³", "LPT¹", "LPT²", "LPT³", "CONIN$", "CONOUT$",
    ];
    let first = stem.split('.').next().unwrap_or_default().trim();
    let stem = if DEVICES.iter().any(|d| d.eq_ignore_ascii_case(first)) {
        format!("_{stem}")
    } else {
        stem.to_owned()
    };

    let room = LONGEST.saturating_sub(extension.chars().count()).max(1);
    let stem: String = stem.chars().take(room).collect();

    format!("{stem}{extension}")
}

/// The kinds of file that run, or can run something, when they are opened:
/// programs and installers, scripts, shortcuts and the files Windows
/// follows like one, disk images, pages that carry script, and the Office
/// files that carry macros or fetch data.
const RUNS: &[&str] = &[
    "apk",
    "app",
    "appimage",
    "application",
    "appref-ms",
    "bat",
    "bin",
    "chm",
    "cmd",
    "com",
    "command",
    "cpl",
    "deb",
    "desktop",
    "diagcab",
    "dmg",
    "docm",
    "dotm",
    "exe",
    "hta",
    "htm",
    "html",
    "img",
    "inf",
    "iqy",
    "iso",
    "jar",
    "js",
    "jse",
    "library-ms",
    "lnk",
    "msc",
    "msi",
    "msix",
    "pif",
    "pkg",
    "ppam",
    "pptm",
    "ps1",
    "py",
    "pyw",
    "reg",
    "rpm",
    "run",
    "scf",
    "scr",
    "sct",
    "search-ms",
    "settingcontent-ms",
    "sh",
    "slk",
    "svg",
    "url",
    "vbe",
    "vbs",
    "vhd",
    "vhdx",
    "wsc",
    "wsf",
    "wsh",
    "xlam",
    "xll",
    "xlsb",
    "xlsm",
];

/// Whether a file of this name is one that runs, or can run something, when
/// it is opened. The name is the one [`safe_name`] gives, which is the one
/// written to disk: asking about the sender's own spelling would let
/// `invoice.exe.` through, which is saved as `invoice.exe`.
pub fn can_run(safe_name: &str) -> bool {
    safe_name
        .rsplit_once('.')
        .is_some_and(|(_, extension)| RUNS.contains(&extension.trim().to_lowercase().as_str()))
}

/// A name's stem and its extension with the dot, where it has a short one.
fn split(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(at) if at > 0 && name.len() - at <= 12 => name.split_at(at),
        _ => (name, ""),
    }
}

/// Write a file into a folder under a name that is free, and say where it
/// went: `report.pdf`, or `report (1).pdf` when that is taken.
pub fn save(folder: &Path, name: &str, content: &[u8]) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(folder)?;
    let name = safe_name(name);
    let (stem, extension) = split(&name);

    for attempt in 0..1000 {
        let candidate = match attempt {
            0 => name.clone(),
            n => format!("{stem} ({n}){extension}"),
        };
        let path = folder.join(candidate);
        // `create_new` fails if anything is there already, a link included,
        // so nothing is written over or through.
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        if let Err(error) = file.write_all(content).and_then(|()| file.sync_all()) {
            drop(file);
            let _ = std::fs::remove_file(&path);
            return Err(error);
        }
        drop(file);
        from_the_internet(&path);

        return Ok(path);
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "there are too many files of that name already",
    ))
}

/// Write a file at a place a person chose in a save dialog, replacing what
/// is there: the dialog has already asked about that.
pub fn save_as(path: &Path, content: &[u8]) -> std::io::Result<PathBuf> {
    if !path.is_absolute() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the place to save to must be a full path",
        ));
    }
    let mut file = std::fs::File::create(path)?;
    file.write_all(content)?;
    file.sync_all()?;
    drop(file);
    from_the_internet(path);

    Ok(path.to_path_buf())
}

/// Mark a saved file as having come from the internet, the way a browser
/// marks a download, so that Windows and macOS give their own warning
/// before it is run and Office opens it protected. Linux has no such mark.
/// A mark that cannot be made does not undo the save.
#[allow(unused_variables)]
fn from_the_internet(path: &Path) {
    // "Zone.Identifier" beside the file's contents; zone 3 is the internet.
    #[cfg(windows)]
    {
        let mut mark = path.as_os_str().to_owned();
        mark.push(":Zone.Identifier");
        let _ = std::fs::write(mark, "[ZoneTransfer]\r\nZoneId=3\r\n");
    }
    // The quarantine attribute, as the system's own tool writes it.
    #[cfg(target_os = "macos")]
    {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let _ = std::process::Command::new("/usr/bin/xattr")
            .arg("-w")
            .arg("com.apple.quarantine")
            .arg(format!("0081;{now:x};Brolga;"))
            .arg(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_cannot_leave_the_folder_it_is_saved_in() {
        assert_eq!(
            safe_name("../../.ssh/authorized_keys"),
            "_.._.ssh_authorized_keys"
        );
        assert_eq!(
            safe_name("C:\\Windows\\system32\\evil.dll"),
            "C__Windows_system32_evil.dll"
        );
        assert_eq!(safe_name("/etc/passwd"), "_etc_passwd");
        for name in ["../../x", "..", ".", "a/b", "a\\b", "\0x"] {
            let safe = safe_name(name);
            assert!(!safe.contains(['/', '\\', '\0']), "{safe}");
            assert!(safe != ".." && safe != ".", "{safe}");
        }
    }

    #[test]
    fn a_name_is_not_hidden_reserved_empty_or_disguised() {
        assert_eq!(safe_name(".bashrc"), "bashrc");
        assert_eq!(safe_name(""), "attachment");
        assert_eq!(safe_name(" .. "), "attachment");
        assert_eq!(safe_name("con.txt"), "_con.txt");
        assert_eq!(safe_name("NUL"), "_NUL");
        assert_eq!(safe_name("report.pdf. "), "report.pdf");
        assert_eq!(safe_name("line\nbreak.txt"), "line_break.txt");
        // Right-to-left override: shown as `invoice-exe.pdf`, really `.exe`.
        assert_eq!(safe_name("invoice-\u{202E}fdp.exe"), "invoice-fdp.exe");
    }

    #[test]
    fn a_file_that_runs_is_known_by_the_name_it_is_saved_under() {
        // Each is saved as `invoice.exe`, however the sender spelt it.
        for disguised in [
            "invoice.exe",
            "invoice.EXE",
            "invoice.exe.",
            "invoice.exe \u{202E}",
            "invoice.exe\u{200B}",
            "invoice.pdf .exe . ",
        ] {
            let name = safe_name(disguised);
            assert!(name.ends_with(".exe") || name.ends_with(".EXE"), "{name}");
            assert!(can_run(&name), "{disguised:?} was saved as {name}");
        }
        for runs in [
            "a.url",
            "a.scf",
            "a.library-ms",
            "a.settingcontent-ms",
            "a.appref-ms",
            "a.chm",
            "a.xll",
            "a.vhdx",
            "a.pyw",
            "a.html",
            "a.svg",
            "a.xlsb",
            "a.iqy",
            "a.docm",
            "a.lnk",
        ] {
            assert!(can_run(&safe_name(runs)), "{runs}");
        }
        for plain in ["report.pdf", "exe", "notes.exe.txt", "photo.jpeg", "a.docx"] {
            assert!(!can_run(&safe_name(plain)), "{plain}");
        }
    }

    #[test]
    fn a_long_name_is_cut_but_keeps_its_extension() {
        let safe = safe_name(&format!("{}.pdf", "é".repeat(400)));

        assert_eq!(safe.chars().count(), LONGEST);
        assert!(safe.ends_with(".pdf"));
    }

    #[test]
    fn saving_as_writes_where_it_is_told_and_only_to_a_full_path() {
        let folder = std::env::temp_dir().join(format!("brolga-saving-as-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("chosen.txt");

        save_as(&path, b"one").unwrap();
        save_as(&path, b"two").unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        assert!(save_as(Path::new("relative.txt"), b"x").is_err());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn saving_never_writes_over_what_is_there() {
        let folder = std::env::temp_dir().join(format!("brolga-saving-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);

        let first = save(&folder, "notes.txt", b"one").unwrap();
        let second = save(&folder, "notes.txt", b"two").unwrap();
        let outside = save(&folder, "../escaped.txt", b"three").unwrap();

        assert_eq!(first, folder.join("notes.txt"));
        assert_eq!(second, folder.join("notes (1).txt"));
        assert_eq!(std::fs::read(&first).unwrap(), b"one");
        assert_eq!(std::fs::read(&second).unwrap(), b"two");
        assert_eq!(outside.parent(), Some(folder.as_path()));

        let _ = std::fs::remove_dir_all(&folder);
    }
}
