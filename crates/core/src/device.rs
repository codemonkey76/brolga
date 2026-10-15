//! This copy of Brolga, as other devices signed in to the same mailbox see
//! it: a random id made the first time it runs and kept on this computer,
//! and a name the person can change (docs/spec-mail-instructions.md, "The
//! processing device").

use brolga_store::GlobalDb;
use serde::Serialize;

const ID: &str = "device_id";
const NAME: &str = "device_name";

/// The name a device goes by when the computer's own name is not known.
const UNNAMED: &str = "This computer";

/// The longest a device's name may be.
pub(crate) const NAME_LIMIT: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Device {
    pub id: String,
    pub name: String,
}

/// Give this install its id and name, the first time it runs. The name is
/// the computer's own, where it is known.
pub(crate) fn settle(global: &GlobalDb, host_name: Option<&str>) -> Result<(), String> {
    if global.setting(ID).map_err(|e| e.to_string())?.is_none() {
        global
            .set_setting(ID, &random_id(16)?)
            .map_err(|e| e.to_string())?;
    }
    if global.setting(NAME).map_err(|e| e.to_string())?.is_none() {
        let name = host_name
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(|name| name.chars().take(NAME_LIMIT).collect::<String>())
            .unwrap_or_else(|| UNNAMED.to_owned());
        global.set_setting(NAME, &name).map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// This device, as settled when Brolga started.
pub(crate) fn this(global: &GlobalDb) -> Result<Device, String> {
    settle(global, None)?;
    let read = |key| {
        global
            .setting(key)
            .map_err(|e| e.to_string())
            .map(Option::unwrap_or_default)
    };

    Ok(Device {
        id: read(ID)?,
        name: read(NAME)?,
    })
}

/// Give this device another name.
pub(crate) fn rename(global: &GlobalDb, name: &str) -> Result<Device, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("A device needs a name.".to_owned());
    }
    if name.chars().count() > NAME_LIMIT {
        return Err(format!(
            "A device's name can be at most {NAME_LIMIT} characters."
        ));
    }
    settle(global, None)?;
    global.set_setting(NAME, name).map_err(|e| e.to_string())?;

    this(global)
}

/// So many random bytes, written in hex.
pub(crate) fn random_id(bytes: usize) -> Result<String, String> {
    let mut buffer = vec![0u8; bytes];
    getrandom::fill(&mut buffer).map_err(|e| format!("no random number could be had: {e}"))?;

    Ok(buffer.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_is_given_an_id_once_and_named_after_the_computer() {
        let global = GlobalDb::in_memory().unwrap();
        settle(&global, Some("titan")).unwrap();
        let first = this(&global).unwrap();
        assert_eq!(first.name, "titan");
        assert_eq!(first.id.len(), 32);

        // Started again, it keeps both, whatever the computer is called now.
        settle(&global, Some("giga")).unwrap();
        assert_eq!(this(&global).unwrap(), first);
    }

    #[test]
    fn a_device_with_no_computer_name_has_one_anyway_and_can_be_renamed() {
        let global = GlobalDb::in_memory().unwrap();
        settle(&global, Some("  ")).unwrap();
        assert_eq!(this(&global).unwrap().name, UNNAMED);

        assert_eq!(
            rename(&global, " Work laptop ").unwrap().name,
            "Work laptop"
        );
        assert!(rename(&global, "").is_err());
        assert!(rename(&global, &"x".repeat(NAME_LIMIT + 1)).is_err());
    }
}
