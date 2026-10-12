//! Fetching a message's remote images, when the person has asked for them.
//!
//! The reading pane never fetches anything itself. Images come through here,
//! so that the request carries no cookies and no referrer, cannot be steered
//! at this computer or the network it sits on, and cannot bring back
//! anything but a picture. Each image is put into the message as a `data:`
//! URL; one that cannot be had is simply left out.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use reqwest::header::CONTENT_TYPE;

/// The most images one message will have asked for, had or not.
const MAX_IMAGES: usize = 40;
/// The largest single image.
const MAX_BYTES: usize = 3 * 1024 * 1024;
/// How long all of one message's images are given between them.
const ALL_WITHIN: Duration = Duration::from_secs(30);

pub(crate) struct RemoteImages {
    http: Option<reqwest::Client>,
    /// Only tests fetch from this machine or a private network.
    allow_private: bool,
}

impl RemoteImages {
    pub fn new(allow_private: bool) -> Self {
        let http = builder().build().ok();

        Self {
            http,
            allow_private,
        }
    }

    /// Replace each `<img src="https://…">` in cleaned HTML with the image
    /// itself. Returns the HTML and how many images could not be had.
    ///
    /// Only the `src` of an `<img>` is ever replaced or fetched. The same
    /// characters written out in the message's words are words.
    pub async fn inline(&self, html: &str) -> (String, u32) {
        self.inline_any(html, "https://").await
    }

    /// As `inline`, of images whose address opens as `open` does: tests
    /// have no `https` server to fetch from.
    async fn inline_any(&self, html: &str, open: &str) -> (String, u32) {
        let mut out = String::with_capacity(html.len());
        let mut from = 0;
        let (mut asked, mut missing) = (0usize, 0u32);
        let deadline = tokio::time::Instant::now() + ALL_WITHIN;
        // Each host is looked up once for the message, and asked through
        // the one client held to the addresses that were checked.
        let mut hosts = HashMap::new();

        for source in brolga_mail::image_sources(html) {
            if !source.address.starts_with(open) {
                continue;
            }
            out.push_str(&html[from..source.at.start]);
            from = source.at.end;

            // All of a message's pictures are had within the one stretch of
            // time, so that slow ones cannot hold the message up for long.
            // Those that fail are counted with those that are had: a
            // message of a thousand pictures that are not there would
            // otherwise have every one of them looked up and asked for.
            let image = if asked < MAX_IMAGES {
                asked += 1;
                tokio::time::timeout_at(deadline, self.fetch(&source.address, &mut hosts))
                    .await
                    .ok()
                    .flatten()
            } else {
                None
            };
            match image {
                Some(data_url) => {
                    out.push_str("src=\"");
                    out.push_str(&data_url);
                    out.push('"');
                }
                // No `src` at all, so the view has nothing to try for itself.
                None => missing += 1,
            }
        }
        out.push_str(&html[from..]);

        (out, missing)
    }

    /// One image as a `data:` URL. `hosts` holds, for each host already
    /// asked for this message, the client held to its addresses, or
    /// nothing for a host that may not be asked.
    async fn fetch(
        &self,
        url: &str,
        hosts: &mut HashMap<String, Option<reqwest::Client>>,
    ) -> Option<String> {
        let parsed = reqwest::Url::parse(url).ok()?;
        if parsed.scheme() != "https" && !(self.allow_private && parsed.scheme() == "http") {
            return None;
        }
        // The name is looked up once, here, and the request is held to the
        // addresses that were checked: a name that answers with a public
        // address now and a private one a moment later gets nowhere.
        let http = if self.allow_private {
            self.http.as_ref()?
        } else {
            let host = parsed.host_str()?.trim_matches(['[', ']']).to_owned();
            if !hosts.contains_key(&host) {
                let port = parsed.port_or_known_default().unwrap_or(443);
                hosts.insert(host.clone(), pinned(&host, port).await);
            }
            hosts.get(&host)?.as_ref()?
        };

        let mut response = http.get(parsed).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        // Only plain picture types. SVG is not one: it can carry scripts and fetch things.
        let kind = response
            .headers()
            .get(CONTENT_TYPE)?
            .to_str()
            .ok()?
            .split(';')
            .next()?
            .trim()
            .to_ascii_lowercase();
        if !matches!(
            kind.as_str(),
            "image/png" | "image/jpeg" | "image/gif" | "image/webp"
        ) {
            return None;
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_BYTES as u64)
        {
            return None;
        }
        // Read a piece at a time, and given up on as soon as it is too big:
        // a server need not say how much it is about to send.
        let mut bytes = Vec::new();
        while let Some(piece) = response.chunk().await.ok()? {
            if bytes.len() + piece.len() > MAX_BYTES {
                return None;
            }
            bytes.extend_from_slice(&piece);
        }

        Some(format!("data:{kind};base64,{}", STANDARD.encode(&bytes)))
    }
}

/// A client that asks a host at the addresses it has now, and no others,
/// if every one of them is public. Nothing for a host that has none, or
/// any that is not.
async fn pinned(host: &str, port: u16) -> Option<reqwest::Client> {
    let addresses: Vec<_> = tokio::net::lookup_host((host, port)).await.ok()?.collect();
    if addresses.is_empty() || !addresses.iter().all(|a| is_public_address(a.ip())) {
        return None;
    }

    builder().resolve_to_addrs(host, &addresses).build().ok()
}

/// Requests that carry nothing and follow nothing.
fn builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        // A redirect could lead off https or onto a private address, and
        // is not followed.
        .redirect(reqwest::redirect::Policy::none())
        .referer(false)
}

fn is_public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => {
            let [a, b, c, _] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_documentation()
                // Carrier-grade NAT, and the block reserved for benchmarking.
                || (a == 100 && (64..128).contains(&b))
                || (a == 198 && (18..20).contains(&b))
                // Set aside for protocols' own use.
                || (a == 192 && b == 0 && c == 0)
                || a == 0
                || a >= 240)
        }
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_public_address(IpAddr::V4(v4)),
            None => {
                let parts = v6.segments();
                let first = parts[0];
                !(v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.is_multicast()
                    // Unique local fc00::/7 and link-local fe80::/10.
                    || (first & 0xfe00) == 0xfc00
                    || (first & 0xffc0) == 0xfe80
                    // Site-local fec0::/10, long retired but still routed inside.
                    || (first & 0xffc0) == 0xfec0
                    // IPv4 carried inside IPv6, where the address within
                    // could be any at all: NAT64 64:ff9b::/96 and 6to4 2002::/16.
                    // 64:ff9b:1::/48, NAT64 within one network, is among the first.
                    || (first == 0x0064 && parts[1] == 0xff9b)
                    || first == 0x2002
                    // And two more of the kind: the retired "IPv4-compatible"
                    // ::/96, and Teredo 2001::/32, which carries a server's
                    // and a client's ("IANA IPv6 Special-Purpose Address
                    // Registry", SOURCES.md).
                    || parts[..6] == [0; 6]
                    || (first == 0x2001 && parts[1] == 0))
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_computer_and_private_networks_are_not_public() {
        for private in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "0.0.0.0",
            "100.64.0.1",
            "255.255.255.255",
            "224.0.0.1",
            "::1",
            "::",
            "fe80::1",
            "fd00::1",
            "fc00::1",
            "::ffff:127.0.0.1",
            "::ffff:192.168.0.1",
        ] {
            assert!(!is_public_address(private.parse().unwrap()), "{private}");
        }
        for public in [
            "8.8.8.8",
            "1.1.1.1",
            "2606:4700:4700::1111",
            "::ffff:8.8.8.8",
        ] {
            assert!(is_public_address(public.parse().unwrap()), "{public}");
        }
    }

    #[tokio::test]
    async fn an_image_on_this_computer_is_never_fetched() {
        let images = RemoteImages::new(false);

        let (html, missing) = images
            .inline("<p>x</p><img src=\"https://127.0.0.1:1/a.png\" alt=\"a\"><img src=\"https://localhost/b.png\">")
            .await;

        assert_eq!(missing, 2);
        assert!(!html.contains("src="), "{html}");
        assert!(html.contains("alt=\"a\""));
    }

    /// A web server of the test's own, to fetch from.
    async fn serve(routes: axum::Router) -> String {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, routes).await;
        });

        format!("http://{address}")
    }

    fn picture(kind: &'static str, bytes: Vec<u8>) -> axum::routing::MethodRouter {
        axum::routing::get(move |headers: axum::http::HeaderMap| {
            let bytes = bytes.clone();
            async move {
                // What a tracker would be told about the reader.
                assert!(headers.get("cookie").is_none() && headers.get("referer").is_none());
                ([(axum::http::header::CONTENT_TYPE, kind)], bytes)
            }
        })
    }

    #[tokio::test]
    async fn a_picture_is_fetched_and_anything_else_is_not() {
        let base = serve(
            axum::Router::new()
                .route("/ok.png", picture("image/png", vec![1, 2, 3]))
                .route(
                    "/page.png",
                    picture("text/html", b"<script>x</script>".to_vec()),
                )
                .route("/drawing.svg", picture("image/svg+xml", b"<svg/>".to_vec()))
                .route("/huge.png", picture("image/png", vec![0; MAX_BYTES + 1]))
                .route(
                    "/elsewhere.png",
                    axum::routing::get(|| async { axum::response::Redirect::temporary("/ok.png") }),
                ),
        )
        .await;
        let images = RemoteImages::new(true);

        assert_eq!(
            images
                .fetch(&format!("{base}/ok.png"), &mut HashMap::new())
                .await
                .as_deref(),
            Some("data:image/png;base64,AQID")
        );
        for refused in [
            "page.png",
            "drawing.svg",
            "huge.png",
            "elsewhere.png",
            "missing.png",
        ] {
            assert_eq!(
                images
                    .fetch(&format!("{base}/{refused}"), &mut HashMap::new())
                    .await,
                None,
                "{refused}"
            );
        }
        // And outside a test, nothing on this computer is fetched at all.
        assert_eq!(
            RemoteImages::new(false)
                .fetch(&format!("{base}/ok.png"), &mut HashMap::new())
                .await,
            None
        );
    }

    #[tokio::test]
    async fn only_so_many_images_are_fetched_for_one_message() {
        let images = RemoteImages::new(false);
        let html = "<img src=\"https://127.0.0.1:1/x.png\">".repeat(MAX_IMAGES + 5);

        let (out, missing) = images.inline(&html).await;

        assert_eq!(missing as usize, MAX_IMAGES + 5);
        assert!(!out.contains("src="));
    }

    #[tokio::test]
    async fn images_that_cannot_be_had_count_towards_the_most_asked_for() {
        let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = asked.clone();
        let base = serve(axum::Router::new().fallback(move || {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async { axum::http::StatusCode::NOT_FOUND }
        }))
        .await;
        let images = RemoteImages::new(true);
        let html = format!("<img src=\"{base}/x.png\">").repeat(MAX_IMAGES + 5);
        let (out, missing) = images.inline_any(&html, "http://").await;

        assert_eq!(missing as usize, MAX_IMAGES + 5);
        assert!(!out.contains("src="));
        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), MAX_IMAGES);
    }

    #[test]
    fn an_ipv4_address_wrapped_in_ipv6_is_not_taken_for_public() {
        for inside in [
            "64:ff9b::c0a8:1",
            "64:ff9b:1::c0a8:1",
            "2002:c0a8:1::1",
            "fec0::1",
            "192.0.0.8",
            "::c0a8:1",
            "::7f00:1",
            "::808:808",
            "2001::1",
            "2001:0:4136:e378:8000:63bf:3fff:fdd2",
        ] {
            assert!(
                !is_public_address(inside.parse().unwrap()),
                "{inside} was taken for public"
            );
        }
        for public in ["2606:4700::1111", "2001:4860:4860::8888"] {
            assert!(is_public_address(public.parse().unwrap()), "{public}");
        }
    }
}
