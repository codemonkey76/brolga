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
    let raw: RawPage =
        serde_json::from_slice(body).map_err(|e| GraphError::Decode(e.to_string()))?;

    let next = match (raw.next_link, raw.delta_link) {
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

    let items =
        raw.value
            .into_iter()
            .map(|entry| {
                if entry.get("@removed").is_some() {
                    let id = entry.get("id").and_then(Value::as_str).ok_or_else(|| {
                        GraphError::Decode("a removed item with no id".to_owned())
                    })?;
                    Ok(DeltaItem::Removed { id: id.to_owned() })
                } else {
                    serde_json::from_value(entry)
                        .map(DeltaItem::Changed)
                        .map_err(|e| GraphError::Decode(e.to_string()))
                }
            })
            .collect::<Result<_, _>>()?;

    Ok(DeltaPage { items, next })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Thing {
        id: String,
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
                DeltaItem::Changed(Thing { id: "a".to_owned() }),
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
}
