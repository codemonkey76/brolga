#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Staying signed in: the milestone's acceptance criteria, against the fake
//! identity platform.

use std::sync::Arc;
use std::time::Duration;

use brolga_auth::{
    AccountError, Accounts, AuthError, Authority, BrowserSignIn, MemorySecrets, Problem,
    Registration, SIGN_IN_SCOPES, SecretStore, TokenClient,
};
use brolga_fake_graph::FakeIdentity;
use brolga_store::GlobalDb;

fn registration() -> Registration {
    Registration::new("11111111-2222-4333-8444-555555555555", None)
}

struct Fixture {
    identity: FakeIdentity,
    db: Arc<GlobalDb>,
    secrets: Arc<MemorySecrets>,
}

impl Fixture {
    async fn new() -> Self {
        Self {
            identity: FakeIdentity::start().await.unwrap(),
            db: Arc::new(GlobalDb::in_memory().unwrap()),
            secrets: Arc::new(MemorySecrets::default()),
        }
    }

    /// The accounts as a freshly started Brolga would have them: the same
    /// database and secret store, and nothing in memory.
    fn start(&self) -> Accounts {
        let client = TokenClient::new(Authority::new(self.identity.base_url())).unwrap();
        Accounts::new(self.db.clone(), self.secrets.clone(), client)
    }

    async fn sign_in(
        &self,
        accounts: &Accounts,
        registration: &Registration,
    ) -> brolga_store::Account {
        let sign_in =
            BrowserSignIn::begin(accounts.client(), registration, SIGN_IN_SCOPES, None, None)
                .await
                .unwrap();
        let url = sign_in.url.clone();
        tokio::spawn(async move {
            let _ = reqwest::get(url).await;
        });
        let tokens = sign_in
            .finish(accounts.client(), Duration::from_secs(10))
            .await
            .unwrap();

        accounts.add(registration, tokens).await.unwrap()
    }
}

#[tokio::test]
async fn sign_in_restart_and_still_signed_in() {
    let fixture = Fixture::new().await;
    let account = fixture.sign_in(&fixture.start(), &registration()).await;

    let restarted = fixture.start();

    assert_eq!(restarted.list().unwrap(), std::slice::from_ref(&account));
    assert!(
        restarted
            .access_token(&account.id)
            .await
            .unwrap()
            .starts_with("access-")
    );
    assert_eq!(account.username, "robin@example.test");
}

#[tokio::test]
async fn a_revoked_sign_in_shows_as_needing_sign_in_and_nothing_crashes() {
    let fixture = Fixture::new().await;
    let account = fixture.sign_in(&fixture.start(), &registration()).await;
    fixture.identity.revoke_refresh_tokens();
    let restarted = fixture.start();

    let error = restarted.access_token(&account.id).await.unwrap_err();

    assert!(
        matches!(&error, AccountError::NeedsSignIn { username, problem: Problem::SignInExpired } if username == "robin@example.test"),
        "{error:?}"
    );
    assert!(restarted.list().unwrap()[0].needs_sign_in);
    // Asking again says the same, without going back to the server.
    let asked = fixture.identity.token_requests().len();
    assert!(matches!(
        restarted.access_token(&account.id).await,
        Err(AccountError::NeedsSignIn { .. })
    ));
    assert_eq!(fixture.identity.token_requests().len(), asked);
}

#[tokio::test]
async fn signing_in_again_clears_needs_sign_in() {
    let fixture = Fixture::new().await;
    let accounts = fixture.start();
    let account = fixture.sign_in(&accounts, &registration()).await;
    fixture.identity.revoke_refresh_tokens();
    let restarted = fixture.start();
    restarted.access_token(&account.id).await.unwrap_err();

    let again = fixture.sign_in(&restarted, &registration()).await;

    assert_eq!(again.id, account.id);
    assert!(!again.needs_sign_in);
    assert_eq!(restarted.list().unwrap().len(), 1);
    assert!(restarted.access_token(&account.id).await.is_ok());
}

#[tokio::test]
async fn an_organisations_own_client_id_is_what_the_account_refreshes_with() {
    let fixture = Fixture::new().await;
    let own = Registration::new(
        "99999999-aaaa-4bbb-8ccc-dddddddddddd",
        Some("contoso.example".to_owned()),
    );
    let account = fixture.sign_in(&fixture.start(), &own).await;

    fixture.start().access_token(&account.id).await.unwrap();

    assert_eq!(account.client_id, own.client_id);
    assert_eq!(account.tenant, "contoso.example");
    let refresh = fixture
        .identity
        .token_requests()
        .into_iter()
        .last()
        .unwrap();
    assert_eq!(refresh["grant_type"], "refresh_token");
    assert_eq!(refresh["client_id"], own.client_id);
}

#[tokio::test]
async fn an_access_token_is_reused_until_it_is_nearly_spent() {
    let fixture = Fixture::new().await;
    let accounts = fixture.start();
    let account = fixture.sign_in(&accounts, &registration()).await;
    let after_sign_in = fixture.identity.token_requests().len();

    let first = accounts.access_token(&account.id).await.unwrap();
    let second = accounts.access_token(&account.id).await.unwrap();

    assert_eq!(first, second);
    assert_eq!(
        fixture.identity.token_requests().len(),
        after_sign_in,
        "an hour-long token was refreshed early"
    );
}

#[tokio::test]
async fn an_access_token_about_to_expire_is_replaced_before_it_is_handed_out() {
    let fixture = Fixture::new().await;
    // Shorter than the margin, so every token issued is already "nearly spent".
    fixture.identity.access_tokens_last(60);
    let accounts = fixture.start();
    let account = fixture.sign_in(&accounts, &registration()).await;
    let from_sign_in = fixture
        .secrets
        .get(&format!("refresh-token.{}", account.id))
        .unwrap();

    let first = accounts.access_token(&account.id).await.unwrap();
    let second = accounts.access_token(&account.id).await.unwrap();

    assert_ne!(first, second);
    // And each refresh kept the new refresh token in place of the old.
    assert_ne!(
        fixture
            .secrets
            .get(&format!("refresh-token.{}", account.id))
            .unwrap(),
        from_sign_in
    );
}

#[tokio::test]
async fn being_offline_does_not_sign_anyone_out() {
    let fixture = Fixture::new().await;
    let account = fixture.sign_in(&fixture.start(), &registration()).await;
    // The same data, and a sign-in service nothing answers at.
    let offline = Accounts::new(
        fixture.db.clone(),
        fixture.secrets.clone(),
        TokenClient::new(Authority::new("http://127.0.0.1:9")).unwrap(),
    );

    let error = offline.access_token(&account.id).await.unwrap_err();

    assert!(
        matches!(error, AccountError::Auth(AuthError::Network(_))),
        "{error:?}"
    );
    assert!(!offline.list().unwrap()[0].needs_sign_in);
    // Back online, it carries on where it was.
    assert!(fixture.start().access_token(&account.id).await.is_ok());
}

#[tokio::test]
async fn two_accounts_are_kept_apart_and_removing_one_forgets_its_sign_in() {
    let fixture = Fixture::new().await;
    let accounts = fixture.start();
    let account = fixture.sign_in(&accounts, &registration()).await;
    let key = format!("refresh-token.{}", account.id);
    assert!(fixture.secrets.get(&key).unwrap().is_some());

    assert!(accounts.remove(&account.id).await.unwrap());

    assert!(accounts.list().unwrap().is_empty());
    assert_eq!(fixture.secrets.get(&key).unwrap(), None);
    assert!(matches!(
        accounts.access_token(&account.id).await,
        Err(AccountError::Unknown(_))
    ));
    assert!(!accounts.remove(&account.id).await.unwrap());
}

#[tokio::test]
async fn no_token_is_ever_written_to_the_database_file() {
    let folder = std::env::temp_dir().join(format!("brolga-auth-test-{}", std::process::id()));
    let path = folder.join("global.db");
    let fixture = Fixture {
        identity: FakeIdentity::start().await.unwrap(),
        db: Arc::new(GlobalDb::open(&path).unwrap()),
        secrets: Arc::new(MemorySecrets::default()),
    };
    fixture.identity.access_tokens_last(60);
    let accounts = fixture.start();
    let account = fixture.sign_in(&accounts, &registration()).await;
    accounts.access_token(&account.id).await.unwrap();

    let mut on_disk = Vec::new();
    for entry in std::fs::read_dir(&folder).unwrap() {
        on_disk.extend(std::fs::read(entry.unwrap().path()).unwrap());
    }
    let on_disk = String::from_utf8_lossy(&on_disk);

    assert!(
        on_disk.contains("robin@example.test"),
        "the test is not reading the database"
    );
    assert!(
        !on_disk.contains("refresh-") && !on_disk.contains("access-"),
        "a token was written to disk"
    );
    std::fs::remove_dir_all(folder).unwrap();
}
