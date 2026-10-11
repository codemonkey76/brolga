//! One account's personal contacts.
//!
//! A contact is kept whole as the JSON the commands give out, beside the
//! few things it is listed and searched by. Contacts are few and small, so
//! a folder of them is replaced whole each time it is read.

use rusqlite::{OptionalExtension, params};

use crate::{AccountDb, StoreError};

/// A contact as it is kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactRow {
    pub id: String,
    /// The folder it is in. Empty for the main Contacts folder.
    pub folder_id: String,
    pub display_name: String,
    /// Everything it is found by, in lower case: names, addresses, company.
    pub search: String,
    /// The contact whole, as JSON.
    pub data: String,
}

impl AccountDb {
    /// The folders of contacts beside the main one, as `(id, name)`. The
    /// contacts of a folder that is no longer there go with it.
    pub fn set_contact_folders(&self, folders: &[(String, String)]) -> Result<(), StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        transaction.execute("DELETE FROM contact_folders", [])?;
        for (id, name) in folders {
            transaction.execute(
                "INSERT OR REPLACE INTO contact_folders (id, name) VALUES (?1, ?2)",
                params![id, name],
            )?;
        }
        transaction.execute(
            "DELETE FROM contacts WHERE folder_id <> '' AND folder_id NOT IN (SELECT id FROM contact_folders)",
            [],
        )?;
        transaction.commit()?;

        Ok(())
    }

    pub fn contact_folders(&self) -> Result<Vec<(String, String)>, StoreError> {
        let connection = self.connection();
        let mut statement = connection
            .prepare("SELECT id, name FROM contact_folders ORDER BY name COLLATE NOCASE")?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;

        Ok(rows)
    }

    /// Replace everything in a folder with what it now holds.
    pub fn set_contacts(&self, folder_id: &str, contacts: &[ContactRow]) -> Result<(), StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        transaction.execute("DELETE FROM contacts WHERE folder_id = ?1", [folder_id])?;
        for contact in contacts {
            transaction.execute(
                "INSERT OR REPLACE INTO contacts (id, folder_id, display_name, search, data) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![contact.id, folder_id, contact.display_name, contact.search, contact.data],
            )?;
        }
        transaction.commit()?;

        Ok(())
    }

    /// Put in one contact as the server has just given it.
    pub fn put_contact(&self, contact: &ContactRow) -> Result<(), StoreError> {
        self.connection().execute(
            "INSERT OR REPLACE INTO contacts (id, folder_id, display_name, search, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![contact.id, contact.folder_id, contact.display_name, contact.search, contact.data],
        )?;

        Ok(())
    }

    pub fn forget_contact(&self, id: &str) -> Result<(), StoreError> {
        self.connection()
            .execute("DELETE FROM contacts WHERE id = ?1", [id])?;

        Ok(())
    }

    /// One contact, whole, as JSON.
    pub fn contact(&self, id: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .connection()
            .query_row("SELECT data FROM contacts WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()?)
    }

    /// Contacts by name, each whole as JSON: all of them, or those with
    /// every word of `query` somewhere in what they are found by; in one
    /// folder (empty for the main one) or with none named in all.
    pub fn contacts(
        &self,
        query: &str,
        folder_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<String>, StoreError> {
        let words: Vec<String> = query
            .to_lowercase()
            .split_whitespace()
            // `%` and `_` typed by a person are letters, not wildcards.
            .map(|word| {
                format!(
                    "%{}%",
                    word.replace('\\', "\\\\")
                        .replace('%', "\\%")
                        .replace('_', "\\_")
                )
            })
            .collect();
        let mut sql = "SELECT data FROM contacts WHERE (?1 IS NULL OR folder_id = ?1)".to_owned();
        for at in 0..words.len() {
            sql.push_str(&format!(" AND search LIKE ?{} ESCAPE '\\'", at + 3));
        }
        sql.push_str(" ORDER BY display_name COLLATE NOCASE, id LIMIT ?2");

        let connection = self.connection();
        let mut statement = connection.prepare(&sql)?;
        let mut given: Vec<Box<dyn rusqlite::ToSql>> =
            vec![Box::new(folder_id.map(str::to_owned)), Box::new(limit)];
        for word in words {
            given.push(Box::new(word));
        }
        let rows = statement
            .query_map(
                rusqlite::params_from_iter(given.iter().map(AsRef::as_ref)),
                |row| row.get(0),
            )?
            .collect::<Result<_, _>>()?;

        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact(id: &str, folder: &str, name: &str, search: &str) -> ContactRow {
        ContactRow {
            id: id.to_owned(),
            folder_id: folder.to_owned(),
            display_name: name.to_owned(),
            search: search.to_owned(),
            data: format!("{{\"id\":\"{id}\"}}"),
        }
    }

    #[test]
    fn contacts_are_kept_by_folder_and_found_by_every_word() {
        let db = AccountDb::in_memory().unwrap();
        db.set_contact_folders(&[("f1".into(), "Suppliers".into())])
            .unwrap();
        db.set_contacts(
            "",
            &[
                contact(
                    "a",
                    "",
                    "Sam Taylor",
                    "sam taylor sam@example.test northwind",
                ),
                contact("b", "", "alex chen", "alex chen 100%_sure"),
            ],
        )
        .unwrap();
        db.set_contacts(
            "f1",
            &[contact(
                "c",
                "f1",
                "Jo Nguyen",
                "jo nguyen jo@supplier.test",
            )],
        )
        .unwrap();
        let ids = |query: &str, folder: Option<&str>| db.contacts(query, folder, 50).unwrap();

        // By name, whatever the case of it.
        assert_eq!(
            ids("", None),
            ["{\"id\":\"b\"}", "{\"id\":\"c\"}", "{\"id\":\"a\"}"]
        );
        assert_eq!(ids("TAYLOR north", None), ["{\"id\":\"a\"}"]);
        assert_eq!(ids("taylor chen", None), Vec::<String>::new());
        assert_eq!(ids("", Some("f1")), ["{\"id\":\"c\"}"]);
        assert_eq!(ids("", Some("")).len(), 2);
        // What looks like a wildcard is a letter.
        assert_eq!(ids("%", None), ["{\"id\":\"b\"}"]);
        assert_eq!(ids("_", None), ["{\"id\":\"b\"}"]);

        // A folder read again holds only what it now has; a folder gone takes its contacts.
        db.set_contacts("", &[contact("a", "", "Sam Taylor", "sam")])
            .unwrap();
        assert_eq!(ids("", Some("")).len(), 1);
        db.set_contact_folders(&[]).unwrap();
        assert_eq!(ids("", None), ["{\"id\":\"a\"}"]);

        db.put_contact(&contact("d", "", "Dee", "dee")).unwrap();
        assert!(db.contact("d").unwrap().is_some());
        db.forget_contact("d").unwrap();
        assert!(db.contact("d").unwrap().is_none());
    }
}
