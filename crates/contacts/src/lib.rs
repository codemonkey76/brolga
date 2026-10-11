//! Contacts as vCards: reading the files other programs write, and writing
//! ones they can read (RFC 6350, SOURCES.md).
//!
//! A contact here is the JSON the rest of the client keeps: `name`,
//! `given_name`, `surname`, `nickname`, `company`, `department`,
//! `job_title`, `emails` (each an `address`), `business_phones`,
//! `home_phones`, `mobile`, `notes`, `birthday` (`YYYY-MM-DD`) and
//! `categories`. A vCard is whatever someone chose to put in a file, so
//! nothing in one is trusted: it is read into those fields and no others.

use calcard::vcard::{
    VCard, VCardEntry, VCardParameter, VCardParameterName, VCardParameterValue, VCardProperty,
    VCardType, VCardValue, VCardVersion,
};
use calcard::{Entry, Parser};
use serde_json::{Value, json};

/// The most contacts read from one file. A file with more is cut off
/// there, and says so.
pub const MOST: usize = 2000;

/// The longest any one field is kept, in characters.
const LONGEST: usize = 4000;

/// What reading a file of vCards came to.
#[derive(Debug, Default, PartialEq)]
pub struct Read {
    pub contacts: Vec<Value>,
    /// Cards with nothing to know a person by, and stretches that were not cards.
    pub skipped: usize,
    /// There were more than [`MOST`], and the rest were left.
    pub cut_short: bool,
}

fn text(value: &VCardValue) -> Option<String> {
    let text: String = value.as_text()?.trim().chars().take(LONGEST).collect();

    (!text.is_empty()).then_some(text)
}

/// Every part of a property that is words, in order, with the gaps kept:
/// a name's family, given and other parts are told apart by where they are.
fn parts(entry: &VCardEntry) -> Vec<String> {
    entry
        .values
        .iter()
        .flat_map(|value| match value {
            VCardValue::Component(all) => all.clone(),
            other => vec![other.as_text().unwrap_or_default().to_owned()],
        })
        .map(|part| part.trim().chars().take(LONGEST).collect())
        .collect()
}

fn kinds(entry: &VCardEntry) -> Vec<&VCardType> {
    entry
        .params
        .iter()
        .filter(|param| param.name == VCardParameterName::Type)
        .filter_map(|param| match &param.value {
            VCardParameterValue::Type(kind) => Some(kind),
            _ => None,
        })
        .collect()
}

/// One card as a contact, or nothing if it names nobody.
fn contact(card: &VCard) -> Option<Value> {
    let first = |property: VCardProperty| {
        card.property(&property)
            .and_then(|entry| entry.values.first())
            .and_then(text)
            .unwrap_or_default()
    };
    let name = card
        .property(&VCardProperty::N)
        .map(parts)
        .unwrap_or_default();
    let part = |at: usize| name.get(at).cloned().unwrap_or_default();
    let organisation = card
        .property(&VCardProperty::Org)
        .map(parts)
        .unwrap_or_default();

    let emails: Vec<Value> = card
        .properties(&VCardProperty::Email)
        .filter_map(|entry| entry.values.first().and_then(text))
        // An address is one word with an @ in it, or it is not kept.
        .filter(|address| address.contains('@') && !address.contains(char::is_whitespace))
        .map(|address| json!({ "address": address }))
        .collect();

    let (mut business, mut home, mut mobile) = (Vec::new(), Vec::new(), String::new());
    for entry in card.properties(&VCardProperty::Tel) {
        let Some(number) = entry.values.first().and_then(text) else {
            continue;
        };
        // A number as a link (`tel:+61…`) is the number.
        let number = number.trim_start_matches("tel:").to_owned();
        let kinds = kinds(entry);
        if kinds.contains(&&VCardType::Fax) || kinds.contains(&&VCardType::Pager) {
            continue;
        }
        if kinds.contains(&&VCardType::Cell) && mobile.is_empty() {
            mobile = number;
        } else if kinds.contains(&&VCardType::Home) {
            home.push(number);
        } else {
            business.push(number);
        }
    }

    let birthday = card
        .property(&VCardProperty::Bday)
        .and_then(|entry| entry.values.first())
        .and_then(|value| match value {
            VCardValue::PartialDateTime(when) => Some(format!(
                "{:04}-{:02}-{:02}",
                when.year?, when.month?, when.day?
            )),
            _ => None,
        });
    let categories: Vec<String> = card
        .properties(&VCardProperty::Categories)
        .flat_map(|entry| entry.values.iter().filter_map(text))
        .collect();

    let contact = json!({
        "name": first(VCardProperty::Fn),
        "given_name": part(1),
        "surname": part(0),
        "nickname": first(VCardProperty::Nickname),
        "company": organisation.first().cloned().unwrap_or_default(),
        "department": organisation.get(1).cloned().unwrap_or_default(),
        "job_title": first(VCardProperty::Title),
        "emails": emails,
        "business_phones": business,
        "home_phones": home,
        "mobile": mobile,
        "notes": first(VCardProperty::Note),
        "birthday": birthday,
        "categories": categories,
    });
    // Something to know them by: a name, a company or an address.
    let known = ["name", "given_name", "surname", "company"]
        .iter()
        .any(|field| {
            contact[*field]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        })
        || !emails.is_empty();

    known.then_some(contact)
}

/// Read every card in a file. What is not a card is passed over and
/// counted; nothing in the file can make this fail.
pub fn read(file: &str) -> Read {
    let mut read = Read::default();
    let mut parser = Parser::new(file);
    loop {
        match parser.entry() {
            Entry::Eof => return read,
            Entry::VCard(_) if read.contacts.len() == MOST => {
                read.cut_short = true;
                return read;
            }
            Entry::VCard(card) => match contact(&card) {
                Some(contact) => read.contacts.push(contact),
                None => read.skipped += 1,
            },
            _ => read.skipped += 1,
        }
    }
}

fn entry(property: VCardProperty, values: Vec<String>) -> VCardEntry {
    VCardEntry::new(property).with_values(values.into_iter().map(VCardValue::Text).collect())
}

fn words(contact: &Value, field: &str) -> String {
    contact[field].as_str().unwrap_or_default().to_owned()
}

fn list(contact: &Value, field: &str) -> Vec<String> {
    contact[field]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|each| each.as_str().map(str::to_owned))
        .collect()
}

/// One contact as a card.
fn card(contact: &Value) -> VCard {
    let mut entries = Vec::new();
    let (given, surname) = (words(contact, "given_name"), words(contact, "surname"));
    entries.push(entry(VCardProperty::Fn, vec![words(contact, "name")]));
    entries.push(
        VCardEntry::new(VCardProperty::N).with_value(VCardValue::Component(vec![
            surname,
            given,
            String::new(),
            String::new(),
            String::new(),
        ])),
    );
    for (property, field) in [
        (VCardProperty::Nickname, "nickname"),
        (VCardProperty::Title, "job_title"),
        (VCardProperty::Note, "notes"),
    ] {
        let value = words(contact, field);
        if !value.is_empty() {
            entries.push(entry(property, vec![value]));
        }
    }
    let (company, department) = (words(contact, "company"), words(contact, "department"));
    if !company.is_empty() || !department.is_empty() {
        let mut organisation = vec![company];
        if !department.is_empty() {
            organisation.push(department);
        }
        entries.push(entry(VCardProperty::Org, organisation));
    }
    for email in contact["emails"].as_array().into_iter().flatten() {
        if let Some(address) = email["address"].as_str() {
            entries.push(entry(VCardProperty::Email, vec![address.to_owned()]));
        }
    }
    let mobile = words(contact, "mobile");
    let phones = list(contact, "business_phones")
        .into_iter()
        .map(|number| (number, VCardType::Work))
        .chain(
            list(contact, "home_phones")
                .into_iter()
                .map(|number| (number, VCardType::Home)),
        )
        .chain((!mobile.is_empty()).then_some((mobile, VCardType::Cell)));
    for (number, kind) in phones {
        entries.push(entry(VCardProperty::Tel, vec![number]).with_param(VCardParameter::typ(kind)));
    }
    // The day as the older form has it, which every reader takes.
    if let Some(day) = contact["birthday"].as_str().filter(|day| day.len() == 10) {
        entries.push(entry(VCardProperty::Bday, vec![day.replace('-', "")]));
    }
    let categories = list(contact, "categories");
    if !categories.is_empty() {
        entries.push(entry(VCardProperty::Categories, categories));
    }

    VCard { entries }
}

/// Contacts as one file of cards, in vCard 3.0, which is what the most
/// programs read.
pub fn write(contacts: &[Value]) -> String {
    let mut file = String::new();
    for contact in contacts {
        // Writing into a string cannot fail.
        let _ = card(contact).write_to(&mut file, VCardVersion::V3_0);
    }

    file
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_from_another_program_is_read_into_the_fields_kept() {
        let file = "BEGIN:VCARD\r\nVERSION:3.0\r\nN:Taylor;Sam;;;\r\nFN:Sam Taylor\r\nNICKNAME:Sammy\r\n\
                    ORG:Harbour Dental;Front desk\r\nTITLE:Practice manager\r\n\
                    EMAIL;TYPE=INTERNET,WORK:sam@example.test\r\nEMAIL;TYPE=HOME:sam.home@example.test\r\n\
                    TEL;TYPE=WORK,VOICE:+61 7 5550 0100\r\nTEL;TYPE=CELL:0400 000 000\r\nTEL;TYPE=HOME:07 5550 0199\r\n\
                    TEL;TYPE=WORK,FAX:07 5550 0101\r\nBDAY:1984-03-09\r\nNOTE:Prefers a call\\, not email.\r\n\
                    CATEGORIES:Clients,Dental\r\nEND:VCARD\r\n";

        let read = read(file);

        assert_eq!(read.skipped, 0);
        assert_eq!(
            read.contacts,
            [json!({
                "name": "Sam Taylor", "given_name": "Sam", "surname": "Taylor", "nickname": "Sammy",
                "company": "Harbour Dental", "department": "Front desk", "job_title": "Practice manager",
                "emails": [{ "address": "sam@example.test" }, { "address": "sam.home@example.test" }],
                "business_phones": ["+61 7 5550 0100"], "home_phones": ["07 5550 0199"], "mobile": "0400 000 000",
                "notes": "Prefers a call, not email.", "birthday": "1984-03-09", "categories": ["Clients", "Dental"]
            })]
        );
    }

    #[test]
    fn the_older_and_the_newer_forms_are_both_read() {
        let old = "BEGIN:VCARD\nVERSION:2.1\nN:Chen;Alex\nFN:Alex Chen\nTEL;CELL:0411 111 111\n\
                   EMAIL;PREF;INTERNET:alex@example.test\nEND:VCARD\n";
        let new = "BEGIN:VCARD\nVERSION:4.0\nFN:Jo Nguyen\nN:Nguyen;Jo;;;\nTEL;VALUE=uri;TYPE=\"voice,cell\":tel:+61422222222\n\
                   EMAIL:jo@example.test\nBDAY:19900521\nEND:VCARD\n";

        let read = read(&format!("{old}{new}"));

        assert_eq!(read.contacts.len(), 2);
        assert_eq!(read.contacts[0]["mobile"], "0411 111 111");
        assert_eq!(
            read.contacts[0]["emails"][0]["address"],
            "alex@example.test"
        );
        assert_eq!(read.contacts[1]["mobile"], "+61422222222");
        assert_eq!(read.contacts[1]["birthday"], "1990-05-21");
    }

    #[test]
    fn what_is_not_a_card_or_names_nobody_is_passed_over_and_counted() {
        let file = "not a card at all\nBEGIN:VCARD\nVERSION:3.0\nTEL:123\nEND:VCARD\n\
                    BEGIN:VCARD\nVERSION:3.0\nEMAIL:not an address\nFN:\nEND:VCARD\n\
                    BEGIN:VCARD\nVERSION:3.0\nFN:Kept\nEND:VCARD\nBEGIN:VCARD\nFN:Never ended\n";

        let read = read(file);

        // A card cut off before its end is kept for what it holds.
        assert_eq!(read.contacts.len(), 2);
        assert_eq!(read.contacts[0]["name"], "Kept");
        assert_eq!(read.contacts[1]["name"], "Never ended");
        assert!(read.skipped >= 3, "{}", read.skipped);
        assert_eq!(super::read("").contacts.len(), 0);
    }

    #[test]
    fn a_file_with_too_many_cards_is_cut_off_and_says_so() {
        let file = "BEGIN:VCARD\nVERSION:3.0\nFN:One of many\nEND:VCARD\n".repeat(MOST + 5);

        let read = read(&file);

        assert_eq!(read.contacts.len(), MOST);
        assert!(read.cut_short);
    }

    #[test]
    fn what_is_written_is_read_back_the_same() {
        let contact = json!({
            "name": "Sam Taylor; Esq.", "given_name": "Sam", "surname": "Taylor", "nickname": "",
            "company": "Harbour, Dental", "department": "Front desk", "job_title": "Practice manager",
            "emails": [{ "address": "sam@example.test" }],
            "business_phones": ["+61 7 5550 0100"], "home_phones": ["07 5550 0199"], "mobile": "0400 000 000",
            "notes": "Two lines:\nthe second; with, marks.", "birthday": "1984-03-09", "categories": ["Clients"]
        });

        let file = write(std::slice::from_ref(&contact));

        assert!(file.starts_with("BEGIN:VCARD\r\nVERSION:3.0\r\n"), "{file}");
        assert_eq!(read(&file).contacts, [contact]);
    }
}
