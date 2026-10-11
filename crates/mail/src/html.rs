use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use ammonia::{Builder, UrlRelative};

/// The largest body that will be cleaned. A message longer than this is
/// shown as plain text; nothing legitimate is this long, and a parser is not
/// to be handed unbounded input.
const MAX_HTML_BYTES: usize = 8 * 1024 * 1024;

/// Whether a message may fetch things from the internet when it is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteContent {
    /// Nothing is fetched. A remote image tells the sender the message was
    /// opened, when, and from where, so this is the default.
    Blocked,
    /// Remote images are left in place for the view to load.
    Allowed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sanitised {
    /// HTML that is safe to put in the reading pane.
    pub html: String,
    /// How many remote images were taken out. More than none means the
    /// message has something to offer to show.
    pub blocked_remote: u32,
}

/// Make a message's HTML safe to show.
///
/// This is an allowlist: only the tags and attributes named here survive,
/// so anything not thought of is removed, not let through. Scripts, event
/// handlers, forms, frames, objects, embeds and meta refreshes all go, with
/// their contents where the contents are code. Styling goes too, both
/// `<style>` and `style=`, because CSS can fetch from the internet and can
/// draw over the rest of the window; messages are shown plainer for it.
/// Links keep only web and mail addresses.
pub fn sanitise_html(html: &str, remote: RemoteContent) -> Sanitised {
    if html.len() > MAX_HTML_BYTES {
        return Sanitised {
            html: text_to_html(&html_to_text_unbounded(
                &html[..floor_boundary(html, MAX_HTML_BYTES)],
            )),
            blocked_remote: 0,
        };
    }

    let blocked = Arc::new(AtomicU32::new(0));
    let counter = blocked.clone();
    let clean = builder()
        .attribute_filter(move |element, attribute, value| {
            match (element, attribute) {
                ("img", "src") => image_source(value, remote, &counter),
                // Anything else that holds an address may hold a web or mail
                // address and nothing more: no `data:`, nothing relative.
                (_, "href" | "src" | "cite") => {
                    let lower = value.trim_start().to_ascii_lowercase();
                    ["https://", "http://", "mailto:"]
                        .iter()
                        .any(|scheme| lower.starts_with(scheme))
                        .then_some(Cow::Borrowed(value))
                }
                _ => Some(Cow::Borrowed(value)),
            }
        })
        .clean(html)
        .to_string();

    Sanitised {
        html: clean,
        blocked_remote: blocked.load(Ordering::Relaxed),
    }
}

/// An image may be one carried inside the message itself, as a `data:` URL
/// of a plain picture type. One on the web stays only if remote content is
/// allowed. Anything else, `cid:` included until attachments are built, goes.
fn image_source<'a>(
    value: &'a str,
    remote: RemoteContent,
    blocked: &AtomicU32,
) -> Option<Cow<'a, str>> {
    let lower = value.trim_start().to_ascii_lowercase();

    if [
        "data:image/png;",
        "data:image/jpeg;",
        "data:image/gif;",
        "data:image/webp;",
    ]
    .iter()
    .any(|p| lower.starts_with(p))
    {
        return Some(Cow::Borrowed(value));
    }
    if lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("//") {
        return match remote {
            RemoteContent::Allowed if lower.starts_with("https://") => Some(Cow::Borrowed(value)),
            _ => {
                blocked.fetch_add(1, Ordering::Relaxed);
                None
            }
        };
    }

    None
}

fn builder() -> Builder<'static> {
    let tags: HashSet<&str> = [
        "a",
        "abbr",
        "b",
        "blockquote",
        "br",
        "caption",
        "center",
        "cite",
        "code",
        "col",
        "colgroup",
        "dd",
        "del",
        "div",
        "dl",
        "dt",
        "em",
        "font",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "hr",
        "i",
        "img",
        "ins",
        "li",
        "ol",
        "p",
        "pre",
        "q",
        "s",
        "small",
        "span",
        "strike",
        "strong",
        "sub",
        "sup",
        "table",
        "tbody",
        "td",
        "tfoot",
        "th",
        "thead",
        "tr",
        "u",
        "ul",
    ]
    .into();

    let mut builder = Builder::default();
    builder
        .tags(tags)
        // Their contents are code or are not for showing, so the contents go with the tag.
        .clean_content_tags(
            [
                "script", "style", "title", "head", "noscript", "template", "iframe", "object",
                "embed", "svg", "math",
            ]
            .into(),
        )
        .generic_attributes(["dir", "lang", "title"].into())
        .tag_attributes(
            [
                ("a", ["href"].into()),
                ("img", ["src", "alt", "width", "height"].into()),
                ("font", ["color", "face", "size"].into()),
                (
                    "table",
                    [
                        "width",
                        "align",
                        "border",
                        "cellpadding",
                        "cellspacing",
                        "bgcolor",
                    ]
                    .into(),
                ),
                (
                    "td",
                    [
                        "width", "height", "align", "valign", "colspan", "rowspan", "bgcolor",
                    ]
                    .into(),
                ),
                (
                    "th",
                    [
                        "width", "height", "align", "valign", "colspan", "rowspan", "bgcolor",
                    ]
                    .into(),
                ),
                ("tr", ["align", "valign", "bgcolor"].into()),
                ("col", ["width", "span"].into()),
                ("ol", ["start", "type"].into()),
                ("blockquote", ["cite"].into()),
            ]
            .into(),
        )
        .url_schemes(["http", "https", "mailto", "data"].into())
        // Addresses are checked one by one in the attribute filter, which
        // has to see the relative ones too in order to count blocked images.
        .url_relative(UrlRelative::PassThrough)
        .link_rel(Some("noopener noreferrer"))
        .strip_comments(true);

    builder
}

/// A message's words, for search and for agents: no tags, no scripts, no
/// styles, with the usual entities read back into characters.
pub fn html_to_text(html: &str) -> String {
    html_to_text_unbounded(&html[..floor_boundary(html, MAX_HTML_BYTES)])
}

fn html_to_text_unbounded(html: &str) -> String {
    // Line-making tags become line breaks before the tags are taken away.
    let mut spaced = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find('<') {
        spaced.push_str(&rest[..at]);
        let tag: String = rest[at + 1..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '/')
            .collect();
        let closing = tag.starts_with('/');
        match tag.trim_start_matches('/').to_ascii_lowercase().as_str() {
            // A new line where one of these opens...
            "br" | "li" | "tr" if !closing => spaced.push('\n'),
            // ...and a blank line either side of a block.
            "p" | "div" | "table" | "blockquote" | "ul" | "ol" | "hr" | "h1" | "h2" | "h3"
            | "h4" | "h5" | "h6" => {
                spaced.push_str("\n\n");
            }
            _ => {}
        }
        spaced.push('<');
        rest = &rest[at + 1..];
    }
    spaced.push_str(rest);

    let mut none = Builder::default();
    none.tags(HashSet::new()).clean_content_tags(
        [
            "script", "style", "title", "head", "noscript", "template", "iframe", "object",
            "embed", "svg", "math",
        ]
        .into(),
    );
    let escaped = none.clean(&spaced).to_string();

    let text = escaped
        .replace("&nbsp;", " ")
        .replace('\u{a0}', " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");

    // Tidy: no trailing spaces, no more than one blank line running.
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
    {
        if line.is_empty() {
            blank += 1;
            if blank > 1 || out.is_empty() {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(&line);
        out.push('\n');
    }

    out.trim_end().to_owned()
}

/// A plain-text body, as HTML that shows it as written.
pub fn text_to_html(text: &str) -> String {
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");

    format!("<pre>{escaped}</pre>")
}

/// The largest index no greater than `at` that does not split a character.
fn floor_boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }

    at
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(html: &str) -> String {
        sanitise_html(html, RemoteContent::Blocked).html
    }

    #[test]
    fn ordinary_formatting_survives() {
        let html = "<p>Hello <strong>Robin</strong>, see <a href=\"https://example.test/a?b=1\">this</a>.</p><ul><li>One</li></ul>";

        let cleaned = clean(html);

        assert!(cleaned.contains("<strong>Robin</strong>"));
        assert!(cleaned.contains("href=\"https://example.test/a?b=1\""));
        assert!(cleaned.contains("rel=\"noopener noreferrer\""));
        assert!(cleaned.contains("<li>One</li>"));
    }

    #[test]
    fn nothing_that_can_run_gets_through() {
        let attacks = [
            "<script>alert(1)</script>",
            "<img src=x onerror=alert(1)>",
            "<a href=\"javascript:alert(1)\">x</a>",
            "<a href=\"JaVaScRiPt:alert(1)\">x</a>",
            "<a href=\"vbscript:msgbox(1)\">x</a>",
            "<iframe src=\"https://evil.test\"></iframe>",
            "<object data=\"https://evil.test/x.swf\"></object>",
            "<embed src=\"https://evil.test/x.swf\">",
            "<form action=\"https://evil.test\"><input name=password><button>Sign in</button></form>",
            "<meta http-equiv=\"refresh\" content=\"0;url=https://evil.test\">",
            "<svg onload=alert(1)><script>alert(1)</script></svg>",
            "<math><mtext><script>alert(1)</script></mtext></math>",
            "<body onload=alert(1)>",
            "<div onclick=\"alert(1)\" onmouseover=\"alert(1)\">x</div>",
            "<link rel=stylesheet href=\"https://evil.test/x.css\">",
            "<base href=\"https://evil.test/\">",
            "<a href=\"data:text/html,<script>alert(1)</script>\">x</a>",
            "<img src=\"data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==\">",
            "<img src=\"data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=\">",
            "<noscript><p title=\"</noscript><img src=x onerror=alert(1)>\">",
        ];

        for attack in attacks {
            let cleaned = clean(attack).to_ascii_lowercase();
            for bad in [
                "<script",
                "onerror",
                "onload",
                "onclick",
                "onmouseover",
                "javascript:",
                "vbscript:",
                "<iframe",
                "<object",
                "<embed",
                "<form",
                "<input",
                "<button",
                "<meta",
                "<link",
                "<base",
                "<svg",
                "<math",
                "data:text",
                "svg+xml",
                "alert(",
            ] {
                assert!(
                    !cleaned.contains(bad),
                    "`{bad}` survived in `{cleaned}` from `{attack}`"
                );
            }
        }
    }

    #[test]
    fn styling_is_removed_because_css_can_fetch_and_can_cover_the_window() {
        let html = "<style>body{background:url(https://evil.test/track)}</style>\
                    <div style=\"position:fixed;top:0;left:0;width:100%;height:100%\">Your session expired</div>";

        let cleaned = clean(html);

        assert!(!cleaned.contains("style"), "{cleaned}");
        assert!(!cleaned.contains("evil.test"), "{cleaned}");
        assert!(cleaned.contains("Your session expired"));
    }

    #[test]
    fn remote_images_are_blocked_and_counted() {
        let html = "<p>Hi</p><img src=\"https://tracker.test/open.gif?id=1\" alt=\"\"><img src=\"http://tracker.test/2.png\">\
                    <img src=\"//tracker.test/3.png\"><img src=\"data:image/png;base64,iVBORw0KGgo=\" alt=\"Logo\">";

        let blocked = sanitise_html(html, RemoteContent::Blocked);

        assert_eq!(blocked.blocked_remote, 3);
        assert!(!blocked.html.contains("tracker.test"));
        assert!(
            blocked.html.contains("data:image/png;base64,iVBORw0KGgo="),
            "an image inside the message was lost"
        );
    }

    #[test]
    fn allowing_remote_content_lets_secure_images_through_and_nothing_else() {
        let html = "<img src=\"https://cdn.test/a.png\"><img src=\"http://cdn.test/b.png\"><script src=\"https://cdn.test/x.js\"></script>";

        let allowed = sanitise_html(html, RemoteContent::Allowed);

        assert!(allowed.html.contains("https://cdn.test/a.png"));
        assert!(
            !allowed.html.contains("http://cdn.test/b.png"),
            "an image over plain http was let through"
        );
        assert!(!allowed.html.contains("script"));
        assert_eq!(allowed.blocked_remote, 1);
    }

    #[test]
    fn broken_and_hostile_markup_does_not_panic() {
        for html in [
            "",
            "<",
            "<<<<>>>>",
            "<p",
            "</p></p></p>",
            "<a href=\"",
            "&#xFFFFFFFF;",
            "\u{0}<p>\u{202e}x</p>",
            &"<div>".repeat(5000),
        ] {
            let _ = sanitise_html(html, RemoteContent::Blocked);
            let _ = html_to_text(html);
        }
    }

    #[test]
    fn text_is_the_words_and_not_the_code() {
        let html = "<html><head><title>T</title><style>p{color:red}</style></head><body>\
                    <p>Hello&nbsp;Robin &amp; Sam,</p><script>steal()</script><div>Line two<br>Line three</div>\
                    <ul><li>First</li><li>Second</li></ul></body></html>";

        assert_eq!(
            html_to_text(html),
            "Hello Robin & Sam,\n\nLine two\nLine three\n\nFirst\nSecond"
        );
    }

    #[test]
    fn plain_text_is_shown_as_written_and_cannot_become_markup() {
        assert_eq!(
            text_to_html("1 < 2 & <script>x</script>"),
            "<pre>1 &lt; 2 &amp; &lt;script&gt;x&lt;/script&gt;</pre>"
        );
    }
}
