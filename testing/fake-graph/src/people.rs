//! Personal contacts and the organisation's people, as Graph serves them.

use std::collections::BTreeMap;

use serde_json::{Value, json};

#[derive(Default)]
pub struct People {
    /// The folders of contacts beside the main one: (id, name).
    folders: Vec<(String, String)>,
    /// Every contact: the folder it is in (empty for the main one), and itself.
    contacts: BTreeMap<String, (String, Value)>,
    /// The organisation: each a person as "List people" gives one.
    directory: Vec<Value>,
    next: u32,
}

/// What of a contact a request may set ("contact resource type").
const WRITABLE: [&str; 15] = [
    "displayName",
    "givenName",
    "surname",
    "nickName",
    "companyName",
    "jobTitle",
    "department",
    "officeLocation",
    "emailAddresses",
    "businessPhones",
    "homePhones",
    "mobilePhone",
    "personalNotes",
    "categories",
    "birthday",
];

impl People {
    pub fn add_folder(&mut self, name: &str) -> String {
        let id = format!("contact-folder-{}", self.folders.len() + 1);
        self.folders.push((id.clone(), name.to_owned()));
        id
    }

    pub fn has_folder(&self, id: &str) -> bool {
        self.folders.iter().any(|each| each.0 == id)
    }

    pub fn folders(&self) -> Value {
        let all: Vec<Value> = self
            .folders
            .iter()
            .map(|(id, name)| json!({ "id": id, "displayName": name }))
            .collect();
        json!({ "value": all })
    }

    /// "Create contact": the contact as made. Nothing if what was given is not a contact.
    pub fn create(&mut self, folder: &str, given: &Value) -> Option<Value> {
        let given = given.as_object()?;
        self.next += 1;
        let id = format!("contact-{:04}", self.next);
        let mut contact = json!({ "id": id, "parentFolderId": folder, "emailAddresses": [], "businessPhones": [], "homePhones": [], "categories": [] });
        for (name, value) in given
            .iter()
            .filter(|(name, _)| WRITABLE.contains(&name.as_str()))
        {
            contact[name.as_str()] = value.clone();
        }
        // "displayName": worked out from the names where it is not given.
        if contact["displayName"].as_str().is_none_or(str::is_empty) {
            let full = format!(
                "{} {}",
                contact["givenName"].as_str().unwrap_or_default(),
                contact["surname"].as_str().unwrap_or_default()
            );
            contact["displayName"] = json!(full.trim());
        }
        self.contacts
            .insert(id, (folder.to_owned(), contact.clone()));

        Some(contact)
    }

    /// "Update contact": only what is given is changed.
    pub fn update(&mut self, id: &str, given: &Value) -> Option<Value> {
        let (_, contact) = self.contacts.get_mut(id)?;
        for (name, value) in given
            .as_object()?
            .iter()
            .filter(|(name, _)| WRITABLE.contains(&name.as_str()))
        {
            contact[name.as_str()] = value.clone();
        }

        Some(contact.clone())
    }

    pub fn delete(&mut self, id: &str) -> bool {
        self.contacts.remove(id).is_some()
    }

    /// "List contacts", for a folder: empty for the main one.
    pub fn in_folder(&self, folder: &str) -> Value {
        let all: Vec<&Value> = self
            .contacts
            .values()
            .filter(|(of, _)| of == folder)
            .map(|(_, contact)| contact)
            .collect();
        json!({ "value": all })
    }

    pub fn add_person(&mut self, name: &str, address: &str, title: &str, department: &str) {
        let (given, surname) = name.split_once(' ').unwrap_or((name, ""));
        self.directory.push(json!({
            "id": format!("person-{}", self.directory.len() + 1),
            "displayName": name,
            "givenName": given,
            "surname": surname,
            "jobTitle": title,
            "department": department,
            "companyName": "Example Pty Ltd",
            "userPrincipalName": address,
            "mail": address,
            "scoredEmailAddresses": [{ "address": address, "relevanceScore": 8.0 }],
            "phones": [{ "type": "business", "number": "+61 7 5550 0100" }]
        }));
    }

    /// "List people" with `$search`: those with a word of their name, or
    /// their alias, beginning with what was typed.
    pub fn search(&self, typed: &str) -> Value {
        let typed = typed.trim_matches('"').to_lowercase();
        let all: Vec<&Value> = self
            .directory
            .iter()
            .filter(|each| {
                each["displayName"].as_str().is_some_and(|name| {
                    name.to_lowercase()
                        .split_whitespace()
                        .any(|word| word.starts_with(&typed))
                }) || each["mail"].as_str().is_some_and(|mail| {
                    mail.split('@')
                        .next()
                        .is_some_and(|alias| alias.to_lowercase().starts_with(&typed))
                })
            })
            .collect();
        json!({ "value": all })
    }

    /// "List users" with a `startswith` filter: the first quoted value in
    /// it is what was typed, and a name or address must begin with it.
    pub fn users(&self, filter: &str) -> Value {
        let typed = filter.split('\'').nth(1).unwrap_or_default().to_lowercase();
        let all: Vec<Value> = self
            .directory
            .iter()
            .filter(|each| {
                ["displayName", "surname", "mail"]
                    .iter()
                    .any(|name| each[*name].as_str().is_some_and(|value| value.to_lowercase().starts_with(&typed)))
            })
            .map(|each| {
                json!({
                    "id": each["id"], "displayName": each["displayName"], "givenName": each["givenName"],
                    "surname": each["surname"], "mail": each["mail"], "userPrincipalName": each["userPrincipalName"]
                })
            })
            .collect();
        json!({ "value": all })
    }
}
