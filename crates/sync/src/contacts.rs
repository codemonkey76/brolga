//! Keeping an account's personal contacts in step with the server.
//!
//! Contacts are few and small, and the main Contacts folder has no
//! documented way to ask only for what changed, so each folder is read
//! whole and replaces what was here for it ("List contacts", SOURCES.md).

use std::sync::Arc;

use brolga_graph::GraphClient;
use brolga_store::{AccountDb, ContactRow};
use jiff::Timestamp;
use serde_json::{Value, json};

use crate::SyncError;

/// How often contacts are read again, unless it is asked for.
const EVERY_MINUTES: i64 = 15;

pub struct ContactSync {
    graph: Arc<GraphClient>,
    db: Arc<AccountDb>,
}

impl ContactSync {
    pub fn new(graph: Arc<GraphClient>, db: Arc<AccountDb>) -> Self {
        Self { graph, db }
    }

    /// Read every folder of contacts, if it is time to or `now` is true.
    /// Gives how many contacts there are, or nothing if it was not time.
    pub async fn sync_all(&self, now: bool) -> Result<Option<u32>, SyncError> {
        let last = self
            .db
            .setting("contacts_synced_at")?
            .and_then(|at| at.parse::<Timestamp>().ok());
        if !now
            && last
                .is_some_and(|last| Timestamp::now().duration_since(last).as_mins() < EVERY_MINUTES)
        {
            return Ok(None);
        }

        let folders = self.graph.contact_folders().await?;
        // The main folder first, then each of the others; a folder is
        // replaced only once all of it has been read.
        let main = self.graph.contacts(None).await?;
        let mut others = Vec::new();
        for (id, _) in &folders {
            others.push((id.clone(), self.graph.contacts(Some(id)).await?));
        }

        let mut count = 0;
        let rows = |all: &[Value], folder: &str| -> Vec<ContactRow> {
            all.iter()
                .filter_map(|each| contact_row(each, folder))
                .collect()
        };
        self.db.set_contact_folders(&folders)?;
        let kept = rows(&main, "");
        count += kept.len();
        self.db.set_contacts("", &kept)?;
        for (id, all) in &others {
            let kept = rows(all, id);
            count += kept.len();
            self.db.set_contacts(id, &kept)?;
        }
        self.db
            .set_setting("contacts_synced_at", &Timestamp::now().to_string())?;

        Ok(Some(u32::try_from(count).unwrap_or(u32::MAX)))
    }
}

/// A contact as it is kept, from how Graph gives one ("contact resource
/// type", SOURCES.md). Nothing if it has no id.
pub fn contact_row(contact: &Value, folder_id: &str) -> Option<ContactRow> {
    let id = contact["id"].as_str()?.to_owned();
    let words = |name: &str| contact[name].as_str().unwrap_or_default().trim().to_owned();
    let list = |name: &str| -> Vec<String> {
        contact[name]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|each| {
                each.as_str()
                    .map(str::trim)
                    .filter(|each| !each.is_empty())
                    .map(str::to_owned)
            })
            .collect()
    };
    let emails: Vec<Value> = contact["emailAddresses"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|each| {
            let address = each["address"].as_str()?.trim();
            (!address.is_empty()).then(
                || json!({ "address": address, "name": each["name"].as_str().unwrap_or_default() }),
            )
        })
        .collect();
    let (given, surname, company) = (words("givenName"), words("surname"), words("companyName"));
    // Its name, or failing that whatever else it is known by.
    let name = [
        words("displayName"),
        format!("{given} {surname}").trim().to_owned(),
        company.clone(),
        emails
            .first()
            .and_then(|first| first["address"].as_str())
            .unwrap_or_default()
            .to_owned(),
    ]
    .into_iter()
    .find(|each| !each.is_empty())
    .unwrap_or_default();

    let data = json!({
        "id": id,
        "folder_id": folder_id,
        "name": name,
        "given_name": given,
        "surname": surname,
        "nickname": words("nickName"),
        "company": company,
        "job_title": words("jobTitle"),
        "department": words("department"),
        "office": words("officeLocation"),
        "emails": emails,
        "business_phones": list("businessPhones"),
        "home_phones": list("homePhones"),
        "mobile": words("mobilePhone"),
        "notes": words("personalNotes"),
        // The day alone: the time Graph gives with it means nothing.
        "birthday": contact["birthday"].as_str().and_then(|day| day.get(..10)),
        "categories": list("categories"),
    });
    let mut search = vec![
        name.clone(),
        given,
        surname,
        words("nickName"),
        words("companyName"),
        words("jobTitle"),
        words("department"),
    ];
    search.extend(
        emails
            .iter()
            .filter_map(|each| each["address"].as_str().map(str::to_owned)),
    );

    Some(ContactRow {
        id,
        folder_id: folder_id.to_owned(),
        display_name: name,
        search: search.join(" ").to_lowercase(),
        data: data.to_string(),
    })
}
