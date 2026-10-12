use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::GraphError;

/// One entry in a delta page.
#[derive(Debug, Clone, PartialEq)]
pub enum DeltaItem<T> {
    /// New, or changed. A changed item may carry only some of its properties.
    Changed(T),
    /// Gone from this collection: deleted, or moved out of it.
    Removed { id: String },
}

/// Where a delta sync goes from here. The URL is used exactly as given:
/// Graph has encoded the original query into it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// More of this round to fetch.
    Page(String),
    /// The round is complete. Ask this for what changes from now on.
    Done(String),
}

impl Next {
    pub fn url(&self) -> &str {
        match self {
            Self::Page(url) | Self::Done(url) => url,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeltaPage<T> {
    pub items: Vec<DeltaItem<T>>,
    pub next: Next,
    /// How many of the items had a part that could not be read, and are
    /// here without that part.
    pub damaged: u32,
}

#[derive(Deserialize)]
struct RawPage {
    #[serde(default)]
    value: Vec<Value>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
    #[serde(rename = "@odata.deltaLink")]
    delta_link: Option<String>,
}

/// Read a delta page. "Use delta query to track changes in Microsoft Graph
/// data" (SOURCES.md): a page carries a nextLink or a deltaLink, never both,
/// and a removed item is its `id` and an `@removed` object.
pub(crate) fn read_page<T: DeserializeOwned>(body: &[u8]) -> Result<DeltaPage<T>, GraphError> {
    read(body, false)
}

/// Read a page of an ordinary list, which is not a delta: it has a nextLink
/// while there is more, and nothing when there is not. The end is given as
/// `Next::Done` with no link, there being nothing to ask next.
pub(crate) fn read_list_page<T: DeserializeOwned>(body: &[u8]) -> Result<DeltaPage<T>, GraphError> {
    read(body, true)
}

fn read<T: DeserializeOwned>(body: &[u8], list: bool) -> Result<DeltaPage<T>, GraphError> {
    let raw: RawPage =
        serde_json::from_slice(body).map_err(|e| GraphError::Decode(e.to_string()))?;

    let next = match (raw.next_link, raw.delta_link) {
        (None, None) if list => Next::Done(String::new()),
        (Some(next), None) => Next::Page(next),
        (None, Some(delta)) => Next::Done(delta),
        (None, None) => {
            return Err(GraphError::Decode(
                "a delta page with neither a nextLink nor a deltaLink".to_owned(),
            ));
        }
        (Some(_), Some(_)) => {
            return Err(GraphError::Decode(
                "a delta page with both a nextLink and a deltaLink".to_owned(),
            ));
        }
    };

    let mut damaged = 0;
    let mut items = Vec::with_capacity(raw.value.len());
    for entry in raw.value {
        if entry.get("@removed").is_some() {
            let id = entry
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| GraphError::Decode("a removed item with no id".to_owned()))?;
            items.push(DeltaItem::Removed { id: id.to_owned() });
        } else {
            let (item, whole) = changed(entry)?;
            damaged += u32::from(!whole);
            items.push(DeltaItem::Changed(item));
        }
    }

    Ok(DeltaPage {
        items,
        next,
        damaged,
    })
}

/// Read one new or changed item, and say whether all of it could be read.
///
/// An item with a property in a shape that cannot be read is not allowed to
/// make its whole page unreadable, which would stop the sync of everything
/// that shares the page, every time. Nor is it left out, because the place
/// the page leaves the sync at is past it, and it would not be offered
/// again. It is read without the properties that cannot be, which is how a
/// delta gives any item of which only a part has changed. Only an item
/// that cannot be told from any other, having no id that can be read, still
/// fails the page: there is nothing of it to keep, and to go on past it
/// would be to lose it without a word.
fn changed<T: DeserializeOwned>(entry: Value) -> Result<(T, bool), GraphError> {
    let error = match T::deserialize(&entry) {
        Ok(item) => return Ok((item, true)),
        Err(error) => GraphError::Decode(error.to_string()),
    };
    let Value::Object(properties) = entry else {
        return Err(error);
    };
    let Some(id) = properties.get("id").cloned() else {
        return Err(error);
    };

    // Each property is tried beside the id alone, and kept if that reads.
    let mut readable = serde_json::Map::new();
    for (name, value) in properties {
        if name == "id" {
            continue;
        }
        let mut alone = serde_json::Map::new();
        alone.insert("id".to_owned(), id.clone());
        alone.insert(name.clone(), value.clone());
        if T::deserialize(&Value::Object(alone)).is_ok() {
            readable.insert(name, value);
        }
    }
    readable.insert("id".to_owned(), id);

    match serde_json::from_value(Value::Object(readable)) {
        Ok(item) => Ok((item, false)),
        Err(_) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Thing {
        id: String,
        size: Option<u32>,
        name: Option<String>,
    }

    fn thing(id: &str) -> Thing {
        Thing {
            id: id.to_owned(),
            size: None,
            name: None,
        }
    }

    fn page(value: Value) -> Result<DeltaPage<Thing>, GraphError> {
        read_page(value.to_string().as_bytes())
    }

    #[test]
    fn changed_and_removed_items_are_told_apart() {
        let read = page(json!({
            "value": [{ "id": "a" }, { "id": "b", "@removed": { "reason": "deleted" } }],
            "@odata.nextLink": "https://example.test/next"
        }))
        .unwrap();

        assert_eq!(
            read.items,
            [
                DeltaItem::Changed(thing("a")),
                DeltaItem::Removed { id: "b".to_owned() }
            ]
        );
        assert_eq!(
            read.next,
            Next::Page("https://example.test/next".to_owned())
        );
    }

    #[test]
    fn an_empty_page_can_still_end_the_round() {
        let read =
            page(json!({ "value": [], "@odata.deltaLink": "https://example.test/delta" })).unwrap();

        assert!(read.items.is_empty());
        assert_eq!(
            read.next,
            Next::Done("https://example.test/delta".to_owned())
        );
    }

    #[test]
    fn a_page_that_does_not_say_where_to_go_next_is_refused() {
        assert!(page(json!({ "value": [] })).is_err());
        assert!(
            page(json!({ "value": [], "@odata.nextLink": "a", "@odata.deltaLink": "b" })).is_err()
        );
        assert!(page(json!({ "value": [{ "@removed": {} }], "@odata.deltaLink": "b" })).is_err());
    }

    #[test]
    fn an_item_with_a_part_that_cannot_be_read_is_kept_without_that_part() {
        let read = page(json!({
            "value": [
                { "id": "a", "size": "large", "name": "Ada" },
                { "id": "b", "size": 2 }
            ],
            "@odata.deltaLink": "https://example.test/delta"
        }))
        .unwrap();

        assert_eq!(
            read.items,
            [
                DeltaItem::Changed(Thing {
                    name: Some("Ada".to_owned()),
                    ..thing("a")
                }),
                DeltaItem::Changed(Thing {
                    size: Some(2),
                    ..thing("b")
                })
            ]
        );
        assert_eq!(read.damaged, 1);
    }

    #[test]
    fn an_item_with_no_id_to_keep_it_by_still_fails_its_page() {
        assert!(page(json!({ "value": [{ "name": "Ada" }], "@odata.deltaLink": "b" })).is_err());
        assert!(page(json!({ "value": ["a"], "@odata.deltaLink": "b" })).is_err());
        assert!(page(json!({ "value": [{ "id": 7 }], "@odata.deltaLink": "b" })).is_err());
    }
}
