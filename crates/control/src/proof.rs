//! How a caller tells that what answers on the channel is this user's own
//! client, where the place the channel listens cannot itself be kept from
//! other users.
//!
//! A named pipe on Windows is one: its name is in a namespace every user
//! of the machine shares, so another user can take the name first and
//! answer on it. The client that is listening therefore keeps a key in a
//! file only its user can read. A caller sends a challenge of its own
//! making, and the client answers with a hash of the key and the challenge,
//! which nothing without the key can give.

use std::io;
use std::path::Path;

use sha2::{Digest, Sha256};

/// The bytes in a key, and in a challenge.
pub(crate) const LENGTH: usize = 32;

pub(crate) type Key = [u8; LENGTH];

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Exactly `LENGTH` bytes written as hex, or nothing.
fn unhex(text: &str) -> Option<[u8; LENGTH]> {
    let text = text.trim();
    if text.len() != LENGTH * 2 || !text.is_ascii() {
        return None;
    }
    let mut bytes = [0; LENGTH];
    for (at, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[at * 2..at * 2 + 2], 16).ok()?;
    }

    Some(bytes)
}

fn random() -> io::Result<[u8; LENGTH]> {
    let mut bytes = [0; LENGTH];
    getrandom::fill(&mut bytes).map_err(io::Error::other)?;

    Ok(bytes)
}

/// A challenge of this caller's own making, as it is sent.
pub(crate) fn challenge() -> io::Result<String> {
    random().map(|bytes| hex(&bytes))
}

/// The answer to a challenge. The challenge must be one of the length
/// [`challenge`] makes: the key and it are then always the same length, so
/// an answer to one challenge says nothing about the answer to another.
pub(crate) fn answer(key: &Key, challenge: &str) -> Option<String> {
    let challenge = unhex(challenge)?;
    let mut hash = Sha256::new();
    hash.update(b"brolga control channel proof\n");
    hash.update(key);
    hash.update(challenge);

    Some(hex(&hash.finalize()))
}

/// Make a new key and keep it in `file`, for whoever may read that file to
/// call with. It is written whole beside the file and moved into place, so
/// a caller never reads half of one.
pub(crate) fn new_key(file: &Path) -> io::Result<Key> {
    let key = random()?;
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let beside = file.with_extension(format!("{}.new", std::process::id()));
    std::fs::write(&beside, hex(&key))?;
    if let Err(error) = std::fs::rename(&beside, file) {
        let _ = std::fs::remove_file(&beside);
        return Err(error);
    }

    Ok(key)
}

/// The key the client that is listening keeps in `file`.
pub(crate) fn read_key(file: &Path) -> io::Result<Key> {
    unhex(&std::fs::read_to_string(file)?).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} is not a control channel key", file.display()),
        )
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_is_the_keys_and_the_challenges_alone() {
        let (one, other) = ([1; LENGTH], [2; LENGTH]);
        let asked = challenge().unwrap();

        assert_eq!(answer(&one, &asked), answer(&one, &asked));
        assert_ne!(answer(&one, &asked), answer(&other, &asked));
        assert_ne!(answer(&one, &asked), answer(&one, &challenge().unwrap()));
        assert_eq!(answer(&one, &asked).unwrap().len(), 64);
    }

    #[test]
    fn a_challenge_of_another_length_is_not_answered() {
        let key = [1; LENGTH];

        for wrong in [
            "",
            "abcd",
            &"0".repeat(63),
            &"0".repeat(66),
            &"z".repeat(64),
        ] {
            assert_eq!(answer(&key, wrong), None, "{wrong}");
        }
    }

    #[test]
    fn a_key_is_read_back_as_it_was_kept_and_a_new_one_is_another() {
        let file = std::env::temp_dir().join(format!("brolga-proof-{}.key", std::process::id()));

        let first = new_key(&file).unwrap();
        assert_eq!(read_key(&file).unwrap(), first);
        let second = new_key(&file).unwrap();
        assert_ne!(first, second);
        assert_eq!(read_key(&file).unwrap(), second);

        std::fs::write(&file, "not a key").unwrap();
        assert_eq!(
            read_key(&file).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let _ = std::fs::remove_file(&file);
    }
}
