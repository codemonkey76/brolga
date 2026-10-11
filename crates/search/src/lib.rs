//! Turning what somebody types in the search box into a query.
//!
//! Words are matched anywhere in a message. A word may be limited to one
//! part of it (`from:sam`, `to:robin`, `subject:invoice`), and a search may
//! be narrowed with `has:attachment`, `is:unread`, `is:read`,
//! `after:2026-01-31` and `before:2026-03-01`.
//!
//! Nothing typed is ever passed through as query syntax. Every word is
//! quoted, so a stray quote or operator cannot change what the query means
//! or make it fail.

/// A search, ready for the store.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    /// A full-text expression in SQLite FTS5 syntax. Empty when the search
    /// is filters only.
    pub fts: String,
    pub has_attachment: Option<bool>,
    pub is_read: Option<bool>,
    /// Received on or after this date, `YYYY-MM-DD`.
    pub after: Option<String>,
    /// Received before this date, `YYYY-MM-DD`.
    pub before: Option<String>,
}

impl Query {
    /// True when the search asks for nothing at all.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

pub fn parse(input: &str) -> Query {
    let mut query = Query::default();
    let mut terms = Vec::new();

    for word in words(input) {
        let (field, value) = match word.split_once(':') {
            Some((field, value)) if !value.is_empty() => (field.to_ascii_lowercase(), value),
            _ => (String::new(), word.as_str()),
        };

        match (field.as_str(), value.to_ascii_lowercase().as_str()) {
            ("has", "attachment" | "attachments") => query.has_attachment = Some(true),
            ("is", "unread") => query.is_read = Some(false),
            ("is", "read") => query.is_read = Some(true),
            ("after" | "since", date) if is_date(date) => query.after = Some(date.to_owned()),
            ("before", date) if is_date(date) => query.before = Some(date.to_owned()),
            ("from", _) => terms.extend(term(value).map(|t| format!("sender : {t}"))),
            ("to", _) => terms.extend(term(value).map(|t| format!("recipients : {t}"))),
            ("subject", _) => terms.extend(term(value).map(|t| format!("subject : {t}"))),
            // Not a filter we know: the whole word is something to look for.
            _ => terms.extend(term(&word)),
        }
    }

    query.fts = terms.join(" ");
    query
}

/// Split on spaces, keeping a "quoted phrase" together, with or without a
/// field in front of it (`subject:"board meeting"`).
fn words(input: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;

    for c in input.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            c => word.push(c),
        }
    }
    if !word.is_empty() {
        words.push(word);
    }

    words
}

/// One thing to look for, as an FTS5 string. It matches words that start
/// with what was typed, so a search works while it is still being typed.
/// Nothing if there is no letter or digit in it to look for.
fn term(text: &str) -> Option<String> {
    let text = text.trim();
    if !text.chars().any(char::is_alphanumeric) {
        return None;
    }

    // Inside an FTS5 string, a quote is written twice. Nothing else is special.
    Some(format!("\"{}\" *", text.replace('"', "\"\"")))
}

fn is_date(text: &str) -> bool {
    let bytes = text.as_bytes();

    bytes.len() == 10
        && bytes.iter().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                *b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_looked_for_anywhere_and_all_must_match() {
        assert_eq!(parse("budget review").fts, "\"budget\" * \"review\" *");
    }

    #[test]
    fn a_word_can_be_limited_to_one_part_of_the_message() {
        let query = parse("from:sam to:robin subject:invoice march");

        assert_eq!(
            query.fts,
            "sender : \"sam\" * recipients : \"robin\" * subject : \"invoice\" * \"march\" *"
        );
    }

    #[test]
    fn a_phrase_in_quotes_stays_together() {
        assert_eq!(
            parse("\"board meeting\" notes").fts,
            "\"board meeting\" * \"notes\" *"
        );
        assert_eq!(
            parse("subject:\"board meeting\"").fts,
            "subject : \"board meeting\" *"
        );
    }

    #[test]
    fn filters_narrow_the_search_without_being_looked_for_as_words() {
        let query = parse("has:attachment is:unread after:2026-01-31 before:2026-03-01 invoice");

        assert_eq!(query.fts, "\"invoice\" *");
        assert_eq!(query.has_attachment, Some(true));
        assert_eq!(query.is_read, Some(false));
        assert_eq!(query.after.as_deref(), Some("2026-01-31"));
        assert_eq!(query.before.as_deref(), Some("2026-03-01"));
    }

    #[test]
    fn something_that_only_looks_like_a_filter_is_just_a_word() {
        assert_eq!(parse("after:tuesday").fts, "\"after:tuesday\" *");
        assert_eq!(parse("re: hello").fts, "\"re:\" * \"hello\" *");
        assert_eq!(
            parse("http://example.test").fts,
            "\"http://example.test\" *"
        );
    }

    #[test]
    fn query_syntax_typed_in_is_looked_for_not_obeyed() {
        for hostile in [
            "\"", "a\"b", "AND", "NOT x", "x OR y", "(", "x)", "col:", "*", "^x", "a NEAR b", "-x",
            "{a b}: c",
        ] {
            let fts = parse(hostile).fts;
            // Every term is one quoted string, an optional column in front, and a star behind.
            let mut rest = fts.as_str();
            while !rest.is_empty() {
                for column in ["sender : ", "recipients : ", "subject : "] {
                    rest = rest.strip_prefix(column).unwrap_or(rest);
                }
                rest = rest
                    .strip_prefix('"')
                    .unwrap_or_else(|| panic!("`{hostile}` gave `{fts}`"));
                let mut end = None;
                let mut chars = rest.char_indices().peekable();
                while let Some((i, c)) = chars.next() {
                    if c == '"' {
                        if matches!(chars.peek(), Some((_, '"'))) {
                            chars.next();
                        } else {
                            end = Some(i);
                            break;
                        }
                    }
                }
                rest = &rest[end.unwrap_or_else(|| panic!("`{hostile}` gave `{fts}`")) + 1..];
                rest = rest
                    .strip_prefix(" *")
                    .unwrap_or_else(|| panic!("`{hostile}` gave `{fts}`"))
                    .trim_start();
            }
        }
    }

    #[test]
    fn nothing_to_look_for_is_an_empty_search() {
        assert!(parse("").is_empty());
        assert!(parse("   \"\"  ").is_empty());
        assert!(parse("* - ( )").is_empty());
        assert!(!parse("is:unread").is_empty());
    }
}
