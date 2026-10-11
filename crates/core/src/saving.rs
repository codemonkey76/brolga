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
        // as `txt.exe`, so they are dropped.
        .filter(|c| !matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'))
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
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
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

    Ok(path.to_path_buf())
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
