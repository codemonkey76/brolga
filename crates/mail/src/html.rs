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
    /// The message's own style sheets, made safe: for the frame a message
    /// is read in and nowhere else. It is CSS, never HTML, and must not be
    /// put anywhere it would style more than the message.
    pub styles: String,
}

/// Make a message's HTML safe to show.
///
/// This is an allowlist: only the tags and attributes named here survive,
/// so anything not thought of is removed, not let through. Scripts, event
/// handlers, forms, frames, objects, embeds and meta refreshes all go, with
/// their contents where the contents are code. A message keeps how it
/// looks where that is said on the element itself, in `style=`, and is
/// about that element alone: colours, type, edges and size. `<style>`
/// sheets go, and so does anything in a `style=` that could fetch from the
/// internet or put one thing over another.
/// Links keep only web and mail addresses.
pub fn sanitise_html(html: &str, remote: RemoteContent) -> Sanitised {
    if html.len() > MAX_HTML_BYTES {
        return Sanitised {
            html: text_to_html(&html_to_text_unbounded(
                &html[..floor_boundary(html, MAX_HTML_BYTES)],
            )),
            blocked_remote: 0,
            styles: String::new(),
        };
    }

    let blocked = Arc::new(AtomicU32::new(0));
    let counter = blocked.clone();
    let clean = builder()
        .attribute_filter(move |element, attribute, value| {
            match (element, attribute) {
                ("img", "src") => image_source(value, remote, &counter),
                (_, "style") => safe_style(value).map(Cow::Owned),
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
        styles: safe_sheets(html),
    }
}

/// An image may be one carried inside the message itself, as a `data:` URL
/// of a plain picture type. One on the web stays only if remote content is
/// allowed. One attached to the message (`cid:`) is kept by its name: the
/// name fetches nothing, and the core puts the picture in its place.
/// Anything else goes.
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
    if lower.starts_with("cid:") {
        return Some(Cow::Borrowed(value));
    }
    if lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("//") {
        // Written back in one plain form, `https://` and no space before it,
        // so that whatever fetches it afterwards finds every one of them.
        let address = value.trim();
        let plain = !address.chars().any(|c| c.is_whitespace() || c.is_control());
        return match remote {
            RemoteContent::Allowed if lower.starts_with("https://") && plain => {
                Some(Cow::Owned(format!("https://{}", &address[8..])))
            }
            _ => {
                blocked.fetch_add(1, Ordering::Relaxed);
                None
            }
        };
    }

    None
}

/// What a message may say about how it looks. Each is about the element it
/// is on: its colours, its type, its edges and its size. Nothing here places
/// an element somewhere else on the page (`position`, `float`, `transform`,
/// `z-index`), hides it from view while leaving it there, or fetches
/// anything.
const STYLE_PROPERTIES: &[&str] = &[
    "background",
    "background-color",
    "border",
    "border-bottom",
    "border-bottom-color",
    "border-bottom-left-radius",
    "border-bottom-right-radius",
    "border-bottom-style",
    "border-bottom-width",
    "border-collapse",
    "border-color",
    "border-left",
    "border-left-color",
    "border-left-style",
    "border-left-width",
    "border-radius",
    "border-right",
    "border-right-color",
    "border-right-style",
    "border-right-width",
    "border-spacing",
    "border-style",
    "border-top",
    "border-top-color",
    "border-top-left-radius",
    "border-top-right-radius",
    "border-top-style",
    "border-top-width",
    "border-width",
    "box-shadow",
    "box-sizing",
    "color",
    "direction",
    "display",
    "font",
    "font-family",
    "font-size",
    "font-style",
    "font-variant",
    "font-variant-numeric",
    "font-weight",
    "height",
    "letter-spacing",
    "line-height",
    "list-style",
    "list-style-type",
    "margin",
    "margin-bottom",
    "margin-left",
    "margin-right",
    "margin-top",
    "max-height",
    "max-width",
    "min-height",
    "min-width",
    "opacity",
    "overflow",
    "padding",
    "padding-bottom",
    "padding-left",
    "padding-right",
    "padding-top",
    "table-layout",
    "text-align",
    "text-decoration",
    "text-indent",
    "text-transform",
    "vertical-align",
    "white-space",
    "width",
    "word-break",
    "word-spacing",
    "overflow-wrap",
    "word-wrap",
];

/// The most of a `style` that is read, and the most declarations kept.
const MAX_STYLE_BYTES: usize = 4096;
const MAX_DECLARATIONS: usize = 60;

/// The declarations of a `style` attribute that are safe to keep, or
/// nothing if none are.
///
/// A declaration stays only if its property is one of [`STYLE_PROPERTIES`]
/// and its value is plain: the only functions are colours, so there is no
/// `url()` to fetch with and no `expression()` or `var()` to hide one in;
/// there are no escapes, which is how a banned word is spelt past a filter;
/// and nothing is pulled off the page with a negative margin or indent.
fn safe_style(style: &str) -> Option<String> {
    let style = &style[..floor_boundary(style, MAX_STYLE_BYTES)];
    let kept: Vec<String> = style
        .split(';')
        .filter_map(|declaration| {
            let (property, value) = declaration.split_once(':')?;
            let property = property.trim().to_ascii_lowercase();
            let value = value.trim().trim_end_matches("!important").trim();
            (STYLE_PROPERTIES.contains(&property.as_str()) && plain_value(&property, value))
                .then(|| format!("{property}:{value}"))
        })
        .take(MAX_DECLARATIONS)
        .collect();

    (!kept.is_empty()).then(|| kept.join(";"))
}

fn plain_value(property: &str, value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if value.is_empty()
        || value.len() > 300
        || lower
            .chars()
            .any(|c| c.is_control() || "\\<>{}@!".contains(c))
        || lower.contains("/*")
    {
        return false;
    }
    // Every function named is a colour, and is closed.
    let mut rest = lower.as_str();
    while let Some(at) = rest.find('(') {
        let name = rest[..at]
            .rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .next()
            .unwrap_or_default();
        let Some(close) = rest[at..].find(')') else {
            return false;
        };
        let inside = &rest[at + 1..at + close];
        if !["rgb", "rgba", "hsl", "hsla"].contains(&name)
            || !inside
                .chars()
                .all(|c| c.is_ascii_digit() || " ,.%/".contains(c))
        {
            return false;
        }
        rest = &rest[at + close + 1..];
    }
    if rest.contains(')') {
        return false;
    }
    // Quotes are for the names of typefaces and nowhere else.
    if (lower.contains('"') || lower.contains('\'')) && !property.starts_with("font") {
        return false;
    }
    // Nothing is moved off the page, or shown and yet not there to see.
    if (property.starts_with("margin") || property == "text-indent") && lower.contains('-') {
        return false;
    }
    if property == "overflow" && !["visible", "hidden", "auto"].contains(&lower.as_str()) {
        return false;
    }
    if property == "display"
        && ![
            "block",
            "inline",
            "inline-block",
            "table",
            "table-row",
            "table-cell",
            "none",
        ]
        .contains(&lower.as_str())
    {
        return false;
    }

    true
}

/// The most style sheet read from one message, and the most rules kept.
const MAX_SHEET_BYTES: usize = 256 * 1024;
const MAX_RULES: usize = 2000;

/// The rules of a message's `<style>` sheets that are safe to keep, written
/// out afresh. A rule keeps the declarations [`safe_style`] would keep on an
/// element. `@media` blocks are read into; every other `@` rule goes, which
/// is `@import` and `@font-face` and whatever else can fetch.
fn safe_sheets(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut css = String::new();
    let mut from = 0;
    while let Some(open) = lower[from..].find("<style") {
        let Some(starts) = lower[from + open..].find('>') else {
            break;
        };
        let start = from + open + starts + 1;
        let end = lower[start..]
            .find("</style")
            .map_or(html.len(), |at| start + at);
        if css.len() + (end - start) > MAX_SHEET_BYTES {
            break;
        }
        css.push_str(&html[start..end]);
        css.push('\n');
        from = end;
    }

    let mut out = String::new();
    let mut rules = 0;
    rules_of(&without_comments(&css), &mut out, &mut rules, 0);

    out
}

fn without_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(at) = rest.find("/*") {
        out.push_str(&rest[..at]);
        out.push(' ');
        rest = rest[at + 2..]
            .find("*/")
            .map_or("", |end| &rest[at + 2 + end + 2..]);
    }
    out.push_str(rest);
    // Sheets were once wrapped in an HTML comment, for browsers long gone.
    out.replace("<!--", " ").replace("-->", " ")
}

/// The end of the block opening at `open`, which is a `{`.
fn block_end(css: &str, open: usize) -> usize {
    let mut depth = 0usize;
    for (at, c) in css[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return open + at;
                }
            }
            _ => {}
        }
    }

    css.len()
}

fn plain_selector(selector: &str) -> bool {
    !selector.is_empty()
        && selector.len() <= 500
        && selector.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || c.is_ascii_whitespace()
                || "_-.,#>+~*:()[]=\"'^$|".contains(c)
        })
}

fn rules_of(css: &str, out: &mut String, rules: &mut usize, depth: usize) {
    let mut at = 0;
    while at < css.len() && *rules < MAX_RULES {
        let rest = &css[at..];
        let Some(open) = rest.find('{') else {
            break;
        };
        // `@import "x";` and the like: a statement with no block.
        if let Some(stop) = rest.find(';').filter(|stop| *stop < open)
            && rest.trim_start().starts_with('@')
        {
            at += stop + 1;
            continue;
        }
        let prelude = rest[..open].trim();
        let prelude = prelude.rsplit(['}', ';']).next().unwrap_or(prelude).trim();
        let end = block_end(css, at + open);
        let inside = &css[(at + open + 1).min(end)..end];
        at = (end + 1).min(css.len());

        if let Some(query) = prelude.strip_prefix("@media") {
            let plain = query.len() <= 300
                && query.chars().all(|c| {
                    c.is_ascii_alphanumeric() || c.is_ascii_whitespace() || "-:(),.".contains(c)
                });
            if plain && depth < 2 {
                let mut within = String::new();
                rules_of(inside, &mut within, rules, depth + 1);
                if !within.is_empty() {
                    out.push_str(&format!("@media {}{{{within}}}\n", query.trim()));
                }
            }
        } else if !prelude.starts_with('@')
            && !inside.contains('{')
            && plain_selector(prelude)
            && let Some(kept) = safe_style(inside)
        {
            *rules += 1;
            out.push_str(&format!("{prelude}{{{kept}}}\n"));
        }
    }
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
        .generic_attributes(["dir", "lang", "title", "style", "class"].into())
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
        .url_schemes(["http", "https", "mailto", "data", "cid"].into())
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

/// Words written by a person, as the HTML to send them as: a paragraph for
/// each, line breaks kept, and nothing typed taken as markup. Unlike
/// `text_to_html`, which shows a received message exactly as it came, this
/// is for writing, and reads as ordinary text in any mail program.
pub fn text_to_paragraphs(text: &str) -> String {
    let mut out = String::new();
    for paragraph in text.replace("\r\n", "\n").split("\n\n") {
        if paragraph.trim().is_empty() {
            continue;
        }
        let escaped = paragraph
            .trim_matches('\n')
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;");
        out.push_str("<p>");
        out.push_str(&escaped.replace('\n', "<br>"));
        out.push_str("</p>");
    }

    out
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

    #[test]
    fn a_message_keeps_how_it_looks_and_nothing_that_fetches_or_moves() {
        let shown = |style: &str| {
            let html = format!(r#"<td style="{style}">x</td>"#);
            sanitise_html(
                &format!("<table><tr>{html}</tr></table>"),
                RemoteContent::Blocked,
            )
            .html
        };

        let kept = shown(
            "padding: 12px 10px; text-align:right; color:#e5e7eb; border-bottom:1px solid #374151; font-family:'Segoe UI', Arial, sans-serif; background-color: rgb(55, 48, 107) !important",
        );
        for each in [
            "padding:12px 10px",
            "text-align:right",
            "color:#e5e7eb",
            "border-bottom:1px solid #374151",
            "font-family:'Segoe UI', Arial, sans-serif",
            "background-color:rgb(55, 48, 107)",
        ] {
            assert!(kept.contains(each), "{each} is missing from {kept}");
        }

        for gone in [
            "background:url(https://tracker.test/a.png)",
            "background-image:url('https://tracker.test/a.png')",
            "background:u\\72l(https://tracker.test/a.png)",
            "position:fixed;top:0;left:0",
            "z-index:9999",
            "width:expression(alert(1))",
            "color:var(--x)",
            "margin-left:-9999px",
            "text-indent:-9999px",
            "transform:translate(10px)",
            "display:flex",
            "color:red} body{color:blue",
            "color:red/* x */",
            "behavior:url(x.htc)",
            "-moz-binding:url(x)",
            "float:left",
        ] {
            let html = shown(gone);
            assert!(!html.contains("style="), "{gone} survived as {html}");
        }
        // The safe part of a mixed one is kept without the rest.
        let mixed = shown("color:red;position:absolute;background:url(x)");
        assert!(mixed.contains(r#"style="color:red""#), "{mixed}");
    }

    #[test]
    fn a_messages_style_sheet_is_kept_rule_by_rule_and_only_what_is_safe() {
        let html = r#"<html><head><style>
            @import url("https://tracker.test/a.css");
            @font-face { font-family: x; src: url(https://tracker.test/f.woff) }
            /* a comment with } and { in it */
            * { box-sizing: border-box; margin: 0; padding: 0; }
            body { font-family: -apple-system, 'Segoe UI', Arial, sans-serif; background: #f3f4f6; color: #374151; }
            .wrapper { max-width: 900px; margin: 32px auto; border-radius: 10px; overflow: hidden; box-shadow: 0 2px 8px rgba(0,0,0,.10); }
            .header h1 { color: #fff; letter-spacing: -.3px; }
            thead th.r { text-align: right; }
            tbody tr:last-child td { border-bottom: none; }
            .cover { position: fixed; top: 0; left: 0; z-index: 99; background: url(https://tracker.test/b.png) }
            .bait:hover { background-image: url(https://tracker.test/hover.png); color: red }
            @media (max-width: 600px) { .wrapper { margin: 0 } .cover { position: fixed } }
            a[href^="x"]{color:blue}
            </style><style>.second{color:green}</style></head>
            <body><div class="wrapper"><h1 id="top">Hi</h1><script>alert(1)</script></div></body></html>"#;

        let clean = sanitise_html(html, RemoteContent::Blocked);

        for kept in [
            "*{box-sizing:border-box;margin:0;padding:0}",
            "body{font-family:-apple-system, 'Segoe UI', Arial, sans-serif;background:#f3f4f6;color:#374151}",
            ".wrapper{max-width:900px;margin:32px auto;border-radius:10px;overflow:hidden;box-shadow:0 2px 8px rgba(0,0,0,.10)}",
            ".header h1{color:#fff;letter-spacing:-.3px}",
            "thead th.r{text-align:right}",
            "tbody tr:last-child td{border-bottom:none}",
            ".bait:hover{color:red}",
            "@media (max-width: 600px){.wrapper{margin:0}\n}",
            "a[href^=\"x\"]{color:blue}",
            ".second{color:green}",
        ] {
            assert!(
                clean.styles.contains(kept),
                "{kept} is missing from:\n{}",
                clean.styles
            );
        }
        for gone in [
            "tracker.test",
            "url",
            "@import",
            "@font-face",
            "position",
            "z-index",
            ".cover",
            "<",
            "comment",
        ] {
            assert!(
                !clean.styles.contains(gone),
                "{gone} survived in:\n{}",
                clean.styles
            );
        }
        // The sheet is apart from the HTML, which has none of it; classes stay to hang it on.
        assert!(!clean.html.contains("style"), "{}", clean.html);
        assert!(
            clean
                .html
                .contains(r#"<div class="wrapper"><h1>Hi</h1></div>"#),
            "{}",
            clean.html
        );
    }

    #[test]
    fn a_style_sheet_cannot_close_itself_or_run_away() {
        for sheet in [
            "</style><script>alert(1)</script>",
            "a{color:red}</style><img src=x onerror=alert(1)>",
            "a{color:red;}} b{{{{",
            "@media screen{@media screen{@media screen{@media screen{a{color:red}}}}}",
            "a{color:red",
            "{}{}{};;;@;@{",
            "a<b{color:red}",
        ] {
            let styles = sanitise_html(
                &format!("<style>{sheet}</style><p>x</p>"),
                RemoteContent::Blocked,
            )
            .styles;
            assert!(
                !styles.contains('<') && !styles.contains("script"),
                "{sheet} gave {styles}"
            );
        }
    }

    #[test]
    fn a_remote_picture_is_written_one_way_however_it_was_spelt() {
        for spelt in [
            r#"<img src="HTTPS://tracker.test/a.png">"#,
            r#"<img src=" https://tracker.test/a.png">"#,
            "<img src=\"\thttps://tracker.test/a.png \">",
        ] {
            let shown = sanitise_html(spelt, RemoteContent::Allowed);
            assert!(
                shown.html.contains(r#"src="https://tracker.test/a.png""#),
                "{spelt} became {}",
                shown.html
            );
            let blocked = sanitise_html(spelt, RemoteContent::Blocked);
            assert!(!blocked.html.to_lowercase().contains("tracker.test"));
            assert_eq!(blocked.blocked_remote, 1);
        }
        // An address with a gap in it is not one to go and fetch.
        let odd = sanitise_html(
            r#"<img src="https://a.test/x y.png">"#,
            RemoteContent::Allowed,
        );
        assert!(!odd.html.contains("a.test"), "{}", odd.html);
    }

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
    fn styling_that_can_fetch_or_cover_the_window_is_removed() {
        let html = "<style>body{background:url(https://evil.test/track)}</style>\
                    <div style=\"position:fixed;top:0;left:0;width:100%;height:100%\">Your session expired</div>";

        let cleaned = clean(html);

        assert!(!cleaned.contains("<style"), "{cleaned}");
        assert!(!cleaned.contains("position"), "{cleaned}");
        assert!(!cleaned.contains("top:"), "{cleaned}");
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

    #[test]
    fn a_picture_attached_to_the_message_keeps_its_name_and_nothing_else_may_use_one() {
        let clean = sanitise_html(
            "<img src=\"cid:logo@example\"><a href=\"cid:logo@example\">x</a>",
            RemoteContent::Blocked,
        );

        assert!(
            clean.html.contains("<img src=\"cid:logo@example\">"),
            "{}",
            clean.html
        );
        assert!(!clean.html.contains("href=\"cid:"), "{}", clean.html);
        assert_eq!(clean.blocked_remote, 0);
    }

    #[test]
    fn written_words_become_paragraphs_and_nothing_typed_becomes_markup() {
        assert_eq!(
            text_to_paragraphs("Hi Sam,\n\nTwo lines\r\nhere <b>&</b>.\n\n\n"),
            "<p>Hi Sam,</p><p>Two lines<br>here &lt;b&gt;&amp;&lt;/b&gt;.</p>"
        );
        assert_eq!(text_to_paragraphs(" \n "), "");
    }
}
