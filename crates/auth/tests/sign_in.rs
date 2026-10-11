#![allow(clippy::unwrap_used, clippy::expect_used)]

//! The sign-in flows, end to end against the fake identity platform.

use std::time::Duration;

use brolga_auth::{
    AuthError, Authority, BrowserSignIn, DeviceSignIn, Problem, Registration, SIGN_IN_SCOPES,
    TokenClient,
};
use brolga_fake_graph::FakeIdentity;

fn registration() -> Registration {
    Registration::new("11111111-2222-4333-8444-555555555555", None)
}

async fn client(identity: &FakeIdentity) -> TokenClient {
    TokenClient::new(Authority::new(identity.base_url())).unwrap()
}

/// What the person's browser does: open the page, and follow it back.
async fn browse(url: String) {
    let _ = reqwest::get(url).await;
}

#[tokio::test]
async fn signing_in_through_the_browser_gives_tokens_and_says_who() {
    let identity = FakeIdentity::start().await.unwrap();
    let client = client(&identity).await;

    let sign_in = BrowserSignIn::begin(&client, &registration(), SIGN_IN_SCOPES, None, None)
        .await
        .unwrap();
    tokio::spawn(browse(sign_in.url.clone()));
    let tokens = sign_in
        .finish(&client, Duration::from_secs(10))
        .await
        .unwrap();

    assert!(tokens.access_token.starts_with("access-"));
    assert!(tokens.refresh_token.is_some());
    let who = tokens.identity.unwrap();
    assert_eq!(who.username, "robin@example.test");
    assert_eq!(who.tenant_id, "00000000-0000-4000-8000-0000000000aa");
}

#[tokio::test]
async fn the_browser_is_sent_what_the_documentation_asks_for() {
    let identity = FakeIdentity::start().await.unwrap();
    let client = client(&identity).await;

    let sign_in = BrowserSignIn::begin(
        &client,
        &registration(),
        SIGN_IN_SCOPES,
        Some("robin@example.test"),
        Some("{\"a\":1}"),
    )
    .await
    .unwrap();
    tokio::spawn(browse(sign_in.url.clone()));
    sign_in
        .finish(&client, Duration::from_secs(10))
        .await
        .unwrap();

    let asked = &identity.authorize_requests()[0];
    assert_eq!(asked["client_id"], registration().client_id);
    assert_eq!(asked["response_type"], "code");
    assert_eq!(asked["response_mode"], "query");
    assert_eq!(
        asked["scope"],
        "openid profile offline_access User.Read Mail.ReadWrite"
    );
    assert_eq!(asked["code_challenge_method"], "S256");
    assert_eq!(asked["login_hint"], "robin@example.test");
    assert_eq!(asked["claims"], "{\"a\":1}");
    assert!(asked["redirect_uri"].starts_with("http://localhost:"));
    assert!(asked["state"].len() >= 16);

    let exchanged = &identity.token_requests()[0];
    assert_eq!(exchanged["grant_type"], "authorization_code");
    assert_eq!(exchanged["redirect_uri"], asked["redirect_uri"]);
    assert_eq!(exchanged["code_verifier"].len(), 43);
}

#[tokio::test]
async fn a_redirect_with_the_wrong_state_is_not_accepted() {
    let identity = FakeIdentity::start().await.unwrap();
    let client = client(&identity).await;
    let sign_in = BrowserSignIn::begin(&client, &registration(), SIGN_IN_SCOPES, None, None)
        .await
        .unwrap();
    let redirect_uri = url::Url::parse(&sign_in.url)
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "redirect_uri")
        .unwrap()
        .1
        .into_owned();

    // Somebody else on this machine posts a code of their own at the port.
    let forged = tokio::spawn(async move {
        reqwest::get(format!("{redirect_uri}/?code=stolen&state=guessed"))
            .await
            .unwrap()
            .status()
    });
    let result = sign_in.finish(&client, Duration::from_millis(600)).await;

    assert_eq!(forged.await.unwrap(), 404);
    assert!(matches!(result, Err(AuthError::TimedOut)), "{result:?}");
    assert!(
        identity.token_requests().is_empty(),
        "a forged code reached the token endpoint"
    );
}

#[tokio::test]
async fn cancelling_in_the_browser_is_explained() {
    let identity = FakeIdentity::start().await.unwrap();
    let client = client(&identity).await;
    identity.refuse_next_authorize(
        "access_denied",
        "AADSTS65004: User declined to consent to access the app.",
    );

    let sign_in = BrowserSignIn::begin(&client, &registration(), SIGN_IN_SCOPES, None, None)
        .await
        .unwrap();
    tokio::spawn(browse(sign_in.url.clone()));
    let error = sign_in
        .finish(&client, Duration::from_secs(10))
        .await
        .unwrap_err();

    assert_eq!(error.problem(), Problem::Declined);
}

#[tokio::test]
async fn an_organisation_that_blocks_the_app_is_explained() {
    let identity = FakeIdentity::start().await.unwrap();
    let client = client(&identity).await;
    identity.refuse_next_token(
        "invalid_grant",
        &[53003],
        "AADSTS53003: Access has been blocked by Conditional Access policies.",
    );

    let sign_in = BrowserSignIn::begin(&client, &registration(), SIGN_IN_SCOPES, None, None)
        .await
        .unwrap();
    tokio::spawn(browse(sign_in.url.clone()));
    let error = sign_in
        .finish(&client, Duration::from_secs(10))
        .await
        .unwrap_err();

    assert_eq!(error.problem(), Problem::ConditionalAccess);
    assert!(error.to_string().contains("access rules"), "{error}");
}

#[tokio::test]
async fn nobody_coming_back_from_the_browser_times_out() {
    let identity = FakeIdentity::start().await.unwrap();
    let client = client(&identity).await;

    let sign_in = BrowserSignIn::begin(&client, &registration(), SIGN_IN_SCOPES, None, None)
        .await
        .unwrap();
    let result = sign_in.finish(&client, Duration::from_millis(200)).await;

    assert!(matches!(result, Err(AuthError::TimedOut)));
}

#[tokio::test]
async fn a_refresh_token_buys_a_new_access_token_and_a_new_refresh_token() {
    let identity = FakeIdentity::start().await.unwrap();
    let client = client(&identity).await;
    let sign_in = BrowserSignIn::begin(&client, &registration(), SIGN_IN_SCOPES, None, None)
        .await
        .unwrap();
    tokio::spawn(browse(sign_in.url.clone()));
    let first = sign_in
        .finish(&client, Duration::from_secs(10))
        .await
        .unwrap();

    let second = client
        .refresh(&registration(), first.refresh_token.as_deref().unwrap())
        .await
        .unwrap();

    assert_ne!(second.access_token, first.access_token);
    assert_ne!(second.refresh_token, first.refresh_token);
}

#[tokio::test]
async fn a_revoked_refresh_token_means_signing_in_again() {
    let identity = FakeIdentity::start().await.unwrap();
    let client = client(&identity).await;
    let sign_in = BrowserSignIn::begin(&client, &registration(), SIGN_IN_SCOPES, None, None)
        .await
        .unwrap();
    tokio::spawn(browse(sign_in.url.clone()));
    let tokens = sign_in
        .finish(&client, Duration::from_secs(10))
        .await
        .unwrap();
    identity.revoke_refresh_tokens();

    let error = client
        .refresh(&registration(), tokens.refresh_token.as_deref().unwrap())
        .await
        .unwrap_err();

    assert_eq!(error.problem(), Problem::SignInExpired);
}

#[tokio::test]
async fn a_device_code_signs_in_once_the_person_has_entered_it() {
    let identity = FakeIdentity::start().await.unwrap();
    let client = client(&identity).await;
    identity.device_polls_before_sign_in(2);

    let sign_in = DeviceSignIn::begin(&client, &registration(), SIGN_IN_SCOPES)
        .await
        .unwrap();
    assert_eq!(sign_in.user_code, "ABCD1234");
    assert!(sign_in.message.contains("ABCD1234"));
    let tokens = sign_in.finish(&client).await.unwrap();

    assert!(tokens.refresh_token.is_some());
    // Two "not yet" answers, then the one that worked.
    assert_eq!(identity.token_requests().len(), 3);
}

#[tokio::test]
async fn a_device_code_that_is_refused_stops_the_waiting() {
    let identity = FakeIdentity::start().await.unwrap();
    let client = client(&identity).await;
    identity.refuse_next_token(
        "authorization_declined",
        &[],
        "The end user denied the request.",
    );

    let sign_in = DeviceSignIn::begin(&client, &registration(), SIGN_IN_SCOPES)
        .await
        .unwrap();
    let error = sign_in.finish(&client).await.unwrap_err();

    assert_eq!(error.problem(), Problem::Declined);
    assert_eq!(identity.token_requests().len(), 1);
}

#[tokio::test]
async fn the_sign_in_service_being_unreachable_is_said_plainly() {
    // A port nothing listens on.
    let client = TokenClient::new(Authority::new("http://127.0.0.1:9")).unwrap();

    let error = client
        .refresh(&registration(), "anything")
        .await
        .unwrap_err();

    assert!(matches!(error, AuthError::Network(_)));
    assert!(error.to_string().contains("could not be reached"));
}
