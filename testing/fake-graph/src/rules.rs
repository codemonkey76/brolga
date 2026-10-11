//! Inbox rules and the mailbox's list of categories, as Graph serves them.

use serde_json::{Value, json};

/// What a rule may ask of a message ("messageRulePredicates resource type").
const PREDICATES: [&str; 29] = [
    "bodyContains",
    "bodyOrSubjectContains",
    "categories",
    "fromAddresses",
    "hasAttachments",
    "headerContains",
    "importance",
    "isApprovalRequest",
    "isAutomaticForward",
    "isAutomaticReply",
    "isEncrypted",
    "isMeetingRequest",
    "isMeetingResponse",
    "isNonDeliveryReport",
    "isPermissionControlled",
    "isReadReceipt",
    "isSigned",
    "isVoicemail",
    "messageActionFlag",
    "notSentToMe",
    "recipientContains",
    "senderContains",
    "sensitivity",
    "sentCcMe",
    "sentOnlyToMe",
    "sentToAddresses",
    "sentToMe",
    "sentToOrCcMe",
    "subjectContains",
];

/// What a rule may do ("messageRuleActions resource type").
const ACTIONS: [&str; 11] = [
    "assignCategories",
    "copyToFolder",
    "delete",
    "forwardAsAttachmentTo",
    "forwardTo",
    "markAsRead",
    "markImportance",
    "moveToFolder",
    "permanentDelete",
    "redirectTo",
    "stopProcessingRules",
];

/// Why a request about a rule was not carried out.
pub enum Refusal {
    Missing,
    Invalid(&'static str),
    ReadOnly,
}

#[derive(Default)]
pub struct Rules {
    rules: Vec<Value>,
    next: u32,
}

/// The parts of a rule are checked as the server would: a name it does not
/// know is refused, not ignored.
fn checked(given: &Value, has_folder: &dyn Fn(&str) -> bool) -> Result<(), Refusal> {
    for (part, known) in [
        ("conditions", &PREDICATES[..]),
        ("exceptions", &PREDICATES[..]),
        ("actions", &ACTIONS[..]),
    ] {
        let Some(part) = given.get(part).filter(|part| !part.is_null()) else {
            continue;
        };
        let Some(part) = part.as_object() else {
            return Err(Refusal::Invalid("A rule's parts are objects."));
        };
        if part
            .get("withinSizeRange")
            .is_some_and(|size| !size.is_object())
        {
            return Err(Refusal::Invalid("A size range is an object."));
        }
        if part
            .keys()
            .any(|name| name != "withinSizeRange" && !known.contains(&name.as_str()))
        {
            return Err(Refusal::Invalid(
                "The rule names something a rule cannot hold.",
            ));
        }
        for name in ["moveToFolder", "copyToFolder"] {
            let folder = part.get(name).and_then(Value::as_str);
            if folder.is_some_and(|folder| !has_folder(folder)) {
                return Err(Refusal::Invalid("The rule names a folder there is not."));
            }
        }
    }

    Ok(())
}

impl Rules {
    /// Put a rule in the mailbox as it is given, read-only ones included. Gives its id.
    pub fn add(&mut self, mut rule: Value) -> String {
        self.next += 1;
        let id = format!("rule-{:04}", self.next);
        rule["id"] = json!(id);
        for (name, value) in [
            ("sequence", json!(self.rules.len() + 1)),
            ("isEnabled", json!(true)),
            ("hasError", json!(false)),
            ("isReadOnly", json!(false)),
        ] {
            if rule.get(name).is_none() {
                rule[name] = value;
            }
        }
        self.rules.push(rule);
        id
    }

    /// "List rules": in the order they run.
    pub fn list(&self) -> Value {
        let mut all = self.rules.clone();
        all.sort_by_key(|rule| rule["sequence"].as_i64().unwrap_or_default());
        json!({ "value": all })
    }

    pub fn get(&self, id: &str) -> Option<Value> {
        self.rules.iter().find(|rule| rule["id"] == id).cloned()
    }

    /// "Create rule": a name and something to do are required.
    pub fn create(
        &mut self,
        given: &Value,
        has_folder: &dyn Fn(&str) -> bool,
    ) -> Result<Value, Refusal> {
        if given["displayName"].as_str().is_none_or(str::is_empty) {
            return Err(Refusal::Invalid("A rule has a name."));
        }
        if given["actions"].as_object().is_none_or(|a| a.is_empty()) {
            return Err(Refusal::Invalid("A rule does something."));
        }
        checked(given, has_folder)?;
        let mut rule = json!({});
        for name in [
            "displayName",
            "sequence",
            "isEnabled",
            "conditions",
            "exceptions",
            "actions",
        ] {
            if let Some(value) = given.get(name) {
                rule[name] = value.clone();
            }
        }
        let id = self.add(rule);

        self.get(&id).ok_or(Refusal::Missing)
    }

    /// "Update rule": what is given is changed, each part as a whole.
    pub fn update(
        &mut self,
        id: &str,
        given: &Value,
        has_folder: &dyn Fn(&str) -> bool,
    ) -> Result<Value, Refusal> {
        checked(given, has_folder)?;
        let rule = self
            .rules
            .iter_mut()
            .find(|rule| rule["id"] == id)
            .ok_or(Refusal::Missing)?;
        if rule["isReadOnly"] == true {
            return Err(Refusal::ReadOnly);
        }
        for name in [
            "displayName",
            "sequence",
            "isEnabled",
            "conditions",
            "exceptions",
            "actions",
        ] {
            if let Some(value) = given.get(name) {
                rule[name] = value.clone();
            }
        }

        Ok(rule.clone())
    }

    pub fn delete(&mut self, id: &str) -> Result<(), Refusal> {
        let rule = self.get(id).ok_or(Refusal::Missing)?;
        if rule["isReadOnly"] == true {
            return Err(Refusal::ReadOnly);
        }
        self.rules.retain(|rule| rule["id"] != id);

        Ok(())
    }
}

/// The mailbox's categories: each an id, a name and a colour.
pub struct Categories {
    all: Vec<(String, String, String)>,
    next: u32,
}

impl Default for Categories {
    fn default() -> Self {
        Self {
            all: [
                ("1", "Red category", "preset0"),
                ("2", "Clients", "preset7"),
                ("3", "Plain", "none"),
            ]
            .map(|(id, name, colour)| (id.to_owned(), name.to_owned(), colour.to_owned()))
            .to_vec(),
            next: 3,
        }
    }
}

fn colour(given: &Value) -> Option<String> {
    let colour = given.as_str()?.to_ascii_lowercase();
    let known = colour == "none"
        || colour
            .strip_prefix("preset")
            .and_then(|n| n.parse::<u8>().ok())
            .is_some_and(|n| n < 25);

    known.then_some(colour)
}

fn shown((id, name, colour): &(String, String, String)) -> Value {
    json!({ "id": id, "displayName": name, "color": colour })
}

impl Categories {
    pub fn list(&self) -> Value {
        json!({ "value": self.all.iter().map(shown).collect::<Vec<_>>() })
    }

    /// "Create Outlook category": a name no other has, and a colour.
    pub fn create(&mut self, given: &Value) -> Result<Value, Refusal> {
        let name = given["displayName"]
            .as_str()
            .filter(|name| !name.is_empty())
            .ok_or(Refusal::Invalid("A category has a name."))?;
        if self
            .all
            .iter()
            .any(|each| each.1.eq_ignore_ascii_case(name))
        {
            return Err(Refusal::Invalid("There is a category of that name."));
        }
        let colour = match given.get("color") {
            Some(given) => {
                colour(given).ok_or(Refusal::Invalid("That is not a category colour."))?
            }
            None => "none".to_owned(),
        };
        self.next += 1;
        let made = (self.next.to_string(), name.to_owned(), colour);
        self.all.push(made.clone());

        Ok(shown(&made))
    }

    /// "Update outlookCategory": the colour, and nothing else.
    pub fn update(&mut self, id: &str, given: &Value) -> Result<Value, Refusal> {
        if given.get("displayName").is_some() {
            return Err(Refusal::Invalid("A category's name cannot be changed."));
        }
        let colour =
            colour(&given["color"]).ok_or(Refusal::Invalid("That is not a category colour."))?;
        let category = self
            .all
            .iter_mut()
            .find(|each| each.0 == id)
            .ok_or(Refusal::Missing)?;
        category.2 = colour;

        Ok(shown(category))
    }

    pub fn delete(&mut self, id: &str) -> Result<(), Refusal> {
        let before = self.all.len();
        self.all.retain(|each| each.0 != id);

        (self.all.len() < before)
            .then_some(())
            .ok_or(Refusal::Missing)
    }
}
