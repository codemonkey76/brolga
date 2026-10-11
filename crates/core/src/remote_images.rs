//! Fetching a message's remote images, when the person has asked for them.
//!
//! The reading pane never fetches anything itself. Images come through here,
//! so that the request carries no cookies and no referrer, cannot be steered
//! at this computer or the network it sits on, and cannot bring back
//! anything but a picture. Each image is put into the message as a `data:`
//! URL; one that cannot be had is simply left out.

use std::net::IpAddr;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use reqwest::header::CONTENT_TYPE;

/// The most images one message will have fetched for it.
const MAX_IMAGES: usize = 40;
/// The largest single image.
const MAX_BYTES: usize = 3 * 1024 * 1024;

pub(crate) struct RemoteImages {
    http: Option<reqwest::Client>,
    /// Only tests fetch from this machine or a private network.
    allow_private: bool,
}

impl RemoteImages {
    pub fn new(allow_private: bool) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            // A redirect could lead off https or onto a private address, and
            // is not followed.
            .redirect(reqwest::redirect::Policy::none())
            .referer(false)
            .build()
            .ok();

        Self {
            http,
            allow_private,
        }
    }

    /// Replace each `<img src="https://…">` in cleaned HTML with the image
    /// itself. Returns the HTML and how many images could not be had.
    pub async fn inline(&self, html: &str) -> (String, u32) {
        const OPEN: &str = "src=\"https://";
        let mut out = String::with_capacity(html.len());
        let mut rest = html;
        let (mut fetched, mut missing) = (0usize, 0u32);

        while let Some(at) = rest.find(OPEN) {
            let start = at + "src=\"".len();
            let Some(length) = rest[start..].find('"') else {
                break;
            };
            // The cleaner writes attribute values with `&` as `&amp;`.
            let url = rest[start..start + length].replace("&amp;", "&");
            out.push_str(&rest[..at]);

            let image = if fetched < MAX_IMAGES {
                self.fetch(&url).await
            } else {
                None
            };
            match image {
                Some(data_url) => {
                    fetched += 1;
                    out.push_str("src=\"");
                    out.push_str(&data_url);
                    out.push('"');
                }
                // No `src` at all, so the view has nothing to try for itself.
                None => missing += 1,
            }
            rest = &rest[start + length + 1..];
        }
        out.push_str(rest);

        (out, missing)
    }

    async fn fetch(&self, url: &str) -> Option<String> {
        let http = self.http.as_ref()?;
        let parsed = reqwest::Url::parse(url).ok()?;
        if parsed.scheme() != "https" && !(self.allow_private && parsed.scheme() == "http") {
            return None;
        }
        if !self.allow_private && !self.is_public(&parsed).await {
            return None;
        }

        let response = http.get(parsed).send().await.ok()?;
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
        let bytes = response.bytes().await.ok()?;
        if bytes.len() > MAX_BYTES {
            return None;
        }

        Some(format!("data:{kind};base64,{}", STANDARD.encode(&bytes)))
    }

    /// True if every address the host resolves to is out on the internet.
    /// A tracking image must not be a way to make this computer call itself,
    /// its router, or a server on the office network.
    async fn is_public(&self, url: &reqwest::Url) -> bool {
        let Some(host) = url.host_str() else {
            return false;
        };
        let port = url.port_or_known_default().unwrap_or(443);

        match tokio::net::lookup_host((host.trim_matches(['[', ']']), port)).await {
            Ok(addresses) => {
                let addresses: Vec<_> = addresses.collect();
                !addresses.is_empty() && addresses.iter().all(|a| is_public_address(a.ip()))
            }
            Err(_) => false,
        }
    }
}

fn is_public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
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
                || a == 0
                || a >= 240)
        }
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_public_address(IpAddr::V4(v4)),
            None => {
                let first = v6.segments()[0];
                !(v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.is_multicast()
                    // Unique local fc00::/7 and link-local fe80::/10.
                    || (first & 0xfe00) == 0xfc00
                    || (first & 0xffc0) == 0xfe80)
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
            images.fetch(&format!("{base}/ok.png")).await.as_deref(),
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
                images.fetch(&format!("{base}/{refused}")).await,
                None,
                "{refused}"
            );
        }
        // And outside a test, nothing on this computer is fetched at all.
        assert_eq!(
            RemoteImages::new(false)
                .fetch(&format!("{base}/ok.png"))
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
}
