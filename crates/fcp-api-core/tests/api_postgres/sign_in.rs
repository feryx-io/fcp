//! Signing in and the caller's own account, route by route
//! (docs/DEEP_TESTING.md D3b).

use super::*;
use fcp_core::CredentialStore;

impl App {
    /// The app as it would be with `config` and `policy`, on the same
    /// database.
    fn reconfigured(mut self, config: ApiConfig, policy: fcp_api_core::SecurityPolicy) -> Self {
        self.state = self.state.clone().with_security_policy(policy);
        self.router = create_router(self.state.clone(), &config);
        self
    }

    async fn stored_user(&self, username: &str) -> fcp_core::User {
        self.storage
            .get_user_in_tenant(username, &self.tenant_a)
            .await
            .unwrap()
            .unwrap()
    }
}

fn expiring_after(days: u32) -> fcp_api_core::SecurityPolicy {
    fcp_api_core::SecurityPolicy {
        password_expiration_days: Some(days),
        ..Default::default()
    }
}

/// A password past its age signs in to nothing but a change; the change
/// takes the right password once, refuses the same one again, and signs
/// in. A password not yet expired is changed the ordinary way.
#[tokio::test]
async fn an_expired_password_is_changed_before_anything_else() {
    let app = require_app!().reconfigured(test_api_config(), expiring_after(90));
    app.create_user_with_role("old", "end_user").await;
    app.create_user_with_role("fresh", "end_user").await;
    // A password an administrator set is the user's to change first.
    let first = app.login("fresh", PASSWORD).await;
    assert_eq!(first.json["password_expired"], true, "{}", first.text);
    let mut user = app.stored_user("fresh").await;
    user.password_last_reset = Some(SystemTime::now() - Duration::from_secs(89 * 86400));
    app.storage.put_user(user).await.unwrap();
    assert!(app.login("fresh", PASSWORD).await.json["token"].is_string());
    let mut user = app.stored_user("old").await;
    user.password_last_reset = Some(SystemTime::now() - Duration::from_secs(91 * 86400));
    app.storage.put_user(user).await.unwrap();

    let login = app.login("old", PASSWORD).await;
    assert_eq!(login.json["password_expired"], true, "{}", login.text);
    assert!(login.json.get("token").is_none(), "{}", login.text);

    let change = |current: &str, new: &str, username: &str| json!({ "username": username, "current_password": current, "new_password": new });
    let path = "/api/v1/auth/change-expired-password";
    let same = app
        .send(
            Method::POST,
            path,
            &[],
            Some(change(PASSWORD, PASSWORD, "old")),
        )
        .await;
    assert_eq!(same.status, StatusCode::BAD_REQUEST, "{}", same.text);
    let wrong = app
        .send(
            Method::POST,
            path,
            &[],
            Some(change("not-it", "N3w-passw0rd!", "old")),
        )
        .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED, "{}", wrong.text);
    let stranger = app
        .send(
            Method::POST,
            path,
            &[],
            Some(change(PASSWORD, "N3w-passw0rd!", "nobody")),
        )
        .await;
    assert_eq!(stranger.status, StatusCode::UNAUTHORIZED);
    let not_expired = app
        .send(
            Method::POST,
            path,
            &[],
            Some(change(PASSWORD, "N3w-passw0rd!", "fresh")),
        )
        .await;
    assert_eq!(
        not_expired.status,
        StatusCode::BAD_REQUEST,
        "{}",
        not_expired.text
    );

    let changed = app
        .send(
            Method::POST,
            path,
            &[],
            Some(change(PASSWORD, "N3w-passw0rd!", "old")),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.text);
    assert!(changed.json["token"].is_string(), "{}", changed.text);
    // The wrong attempt above is forgiven by the right one.
    assert_eq!(app.stored_user("old").await.failed_login_attempts, 0);
    assert_eq!(
        app.login("old", PASSWORD).await.status,
        StatusCode::UNAUTHORIZED
    );
    let again = app.login("old", "N3w-passw0rd!").await;
    assert!(again.json["token"].is_string(), "{}", again.text);
    let twice = app
        .send(
            Method::POST,
            path,
            &[],
            Some(change("N3w-passw0rd!", "An0ther-one!", "old")),
        )
        .await;
    assert_eq!(twice.status, StatusCode::BAD_REQUEST, "{}", twice.text);
    app.finish().await;
}

/// Without password expiry the route changes nothing.
#[tokio::test]
async fn without_expiry_there_is_no_expired_password_to_change() {
    let app = require_app!();
    app.create_user_with_role("plain", "end_user").await;
    let res = app
        .send(
            Method::POST,
            "/api/v1/auth/change-expired-password",
            &[],
            Some(json!({ "username": "plain", "current_password": PASSWORD,
                         "new_password": "N3w-passw0rd!" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST, "{}", res.text);
    assert!(app.login("plain", PASSWORD).await.json["token"].is_string());
    app.finish().await;
}

/// Public sign-up, where the deployment allows it: a trial tenant and its
/// administrator, signed in; a name taken or a request short of what it
/// needs is refused and leaves nothing behind.
#[tokio::test]
async fn a_company_signs_up_and_its_administrator_is_signed_in() {
    let app = require_app!();
    let closed = app
        .send(
            Method::POST,
            "/api/v1/auth/signup",
            &[],
            Some(json!({ "company_name": "Acme", "username": "founder",
                         "password": "Str0ng-pass!", "email": "f@acme.test" })),
        )
        .await;
    assert_eq!(closed.status, StatusCode::NOT_FOUND, "{}", closed.text);

    let app = app.reconfigured(
        ApiConfig {
            allow_public_signup: true,
            ..test_api_config()
        },
        Default::default(),
    );
    let body = |company: &str, username: &str, password: &str, email: &str| {
        json!({ "company_name": company, "username": username,
                "password": password, "email": email })
    };
    for (bad, what) in [
        (
            body(" ", "founder", "Str0ng-pass!", "f@acme.test"),
            "company",
        ),
        (
            body("Acme", "fo", "Str0ng-pass!", "f@acme.test"),
            "short name",
        ),
        (
            body("Acme", "founder", "short", "f@acme.test"),
            "short password",
        ),
        (body("Acme", "founder", "Str0ng-pass!", "nowhere"), "email"),
        (
            body("Acme", "found\r\ner", "Str0ng-pass!", "f@acme.test"),
            "CRLF name",
        ),
        (
            body("Acme", "fo/../under", "Str0ng-pass!", "f@acme.test"),
            "path name",
        ),
    ] {
        let res = app
            .send(Method::POST, "/api/v1/auth/signup", &[], Some(bad))
            .await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST, "{what}: {}", res.text);
    }

    let made = app
        .send(
            Method::POST,
            "/api/v1/auth/signup",
            &[],
            Some(body("Acme Ltd", "founder", "Str0ng-pass!", "f@acme.test")),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    assert_eq!(made.json["tenant_name"], "Acme Ltd");
    assert_eq!(made.json["username"], "founder");
    let tenant: TenantId = made.json["tenant_id"].as_str().unwrap().parse().unwrap();
    assert_ne!(tenant, app.tenant_a);
    let token = made.json["token"].as_str().unwrap().to_string();
    let me = app
        .send_as(&token, Method::GET, "/api/v1/auth/me", None)
        .await;
    assert_eq!(me.json["username"], "founder", "{}", me.text);
    assert_eq!(me.json["role"], "tenant_admin", "{}", me.text);
    assert_eq!(me.json["tenant_id"], tenant.to_string(), "{}", me.text);
    // Their tenant, and only it.
    let users = app
        .send_as(&token, Method::GET, "/api/v1/users", None)
        .await;
    assert_eq!(users.status, StatusCode::OK, "{}", users.text);
    let names: Vec<&str> = users
        .json
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["username"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["founder"], "{}", users.text);
    let stored = app.storage.get_tenant(&tenant).await.unwrap().unwrap();
    assert_eq!(stored.status, fcp_core::TenantStatus::Trial);
    assert!(stored.trial_expires_at.is_some());
    let tenants = app
        .send_as(&token, Method::GET, "/api/v1/tenants", None)
        .await;
    assert_eq!(tenants.status, StatusCode::OK, "{}", tenants.text);
    let seen: Vec<&str> = tenants
        .json
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(seen, vec![tenant.to_string().as_str()], "{}", tenants.text);

    let taken = app
        .send(
            Method::POST,
            "/api/v1/auth/signup",
            &[],
            Some(body("Acme Ltd", "second", "Str0ng-pass!", "s@acme.test")),
        )
        .await;
    assert_eq!(taken.status, StatusCode::CONFLICT, "{}", taken.text);
    let named: Vec<_> = app
        .storage
        .list_tenants()
        .await
        .unwrap()
        .into_iter()
        .filter(|t| t.name == "Acme Ltd")
        .collect();
    assert_eq!(named.len(), 1);
    app.finish().await;
}

/// A personal token is rotated by its owner, signed in: the new secret
/// works, the old one for its grace and not after a rotation with none;
/// another user's token is not theirs to rotate, and a token cannot
/// rotate itself.
#[tokio::test]
async fn a_personal_token_is_rotated_by_its_owner() {
    let app = require_app!();
    app.create_user_with_role("pat", "end_user").await;
    app.create_user_with_role("sam", "end_user").await;
    let pat = session(&app, "pat").await;
    let sam = session(&app, "sam").await;
    let made = app
        .send_as(
            &pat,
            Method::POST,
            "/api/v1/me/api-tokens",
            Some(json!({ "name": "laptop" })),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    let id = made.json["token"]["id"].as_str().unwrap().to_string();
    let first = made.json["secret"].as_str().unwrap().to_string();
    let path = format!("/api/v1/me/api-tokens/{id}/rotate");

    let theirs = app.send_as(&sam, Method::POST, &path, None).await;
    assert_eq!(theirs.status, StatusCode::NOT_FOUND, "{}", theirs.text);
    let itself = app.send_as(&first, Method::POST, &path, None).await;
    assert_eq!(itself.status, StatusCode::FORBIDDEN, "{}", itself.text);

    let rotated = app
        .send_as(
            &pat,
            Method::POST,
            &path,
            Some(json!({ "grace_seconds": 3600 })),
        )
        .await;
    assert_eq!(rotated.status, StatusCode::OK, "{}", rotated.text);
    let second = rotated.json["secret"].as_str().unwrap().to_string();
    assert_ne!(second, first);
    assert_eq!(rotated.json["token"]["id"], id.as_str());
    for secret in [&first, &second] {
        let me = app
            .send_as(secret, Method::GET, "/api/v1/me/profile", None)
            .await;
        assert_eq!(me.status, StatusCode::OK, "{}", me.text);
    }

    let at_once = app
        .send_as(
            &pat,
            Method::POST,
            &path,
            Some(json!({ "grace_seconds": 0 })),
        )
        .await;
    assert_eq!(at_once.status, StatusCode::OK, "{}", at_once.text);
    let third = at_once.json["secret"].as_str().unwrap().to_string();
    let old = app
        .send_as(&second, Method::GET, "/api/v1/me/profile", None)
        .await;
    assert_eq!(old.status, StatusCode::UNAUTHORIZED, "{}", old.text);
    let new = app
        .send_as(&third, Method::GET, "/api/v1/me/profile", None)
        .await;
    assert_eq!(new.status, StatusCode::OK, "{}", new.text);
    let too_long = app
        .send_as(
            &pat,
            Method::POST,
            &path,
            Some(json!({ "grace_seconds": 30 * 86400 })),
        )
        .await;
    assert_eq!(
        too_long.status,
        StatusCode::BAD_REQUEST,
        "{}",
        too_long.text
    );
    app.finish().await;
}

/// New recovery codes need a current code and spend the old ones.
#[tokio::test]
async fn recovery_codes_are_renewed_with_a_code() {
    let app = require_app!();
    app.create_user_with_role("ria", "end_user").await;
    let jwt = session(&app, "ria").await;
    let secret = app
        .send_as(&jwt, Method::POST, "/api/v1/me/totp/enroll", None)
        .await
        .json["secret"]
        .as_str()
        .unwrap()
        .to_string();
    let confirmed = app
        .send_as(
            &jwt,
            Method::POST,
            "/api/v1/me/totp/confirm",
            Some(json!({ "code": totp_code(&secret, 0) })),
        )
        .await;
    let old_codes = confirmed.json["recovery_codes"].clone();

    let no_code = app
        .send_as(
            &jwt,
            Method::POST,
            "/api/v1/me/totp/recovery-codes",
            Some(json!({ "code": "000000" })),
        )
        .await;
    assert_eq!(no_code.status, StatusCode::UNAUTHORIZED, "{}", no_code.text);
    let renewed = app
        .send_as(
            &jwt,
            Method::POST,
            "/api/v1/me/totp/recovery-codes",
            Some(json!({ "code": totp_code(&secret, 1) })),
        )
        .await;
    assert_eq!(renewed.status, StatusCode::OK, "{}", renewed.text);
    let new_codes = renewed.json["recovery_codes"].as_array().unwrap().clone();
    assert_eq!(new_codes.len(), 10);
    assert_ne!(renewed.json["recovery_codes"], old_codes);

    // The old codes are spent; a new one signs in.
    let ticket = app.login("ria", PASSWORD).await.json["mfa_token"]
        .as_str()
        .unwrap()
        .to_string();
    let stale = app
        .login_totp(&ticket, json!({ "recovery_code": old_codes[0] }))
        .await;
    assert_eq!(stale.status, StatusCode::UNAUTHORIZED, "{}", stale.text);
    let fresh = app
        .login_totp(&ticket, json!({ "recovery_code": new_codes[0] }))
        .await;
    assert_eq!(fresh.status, StatusCode::OK, "{}", fresh.text);

    app.finish().await;
}

/// Turning the second factor off needs a current code; then a password
/// alone signs in, and there are no recovery codes to renew.
#[tokio::test]
async fn the_second_factor_is_turned_off_with_a_code() {
    let app = require_app!();
    app.create_user_with_role("dee", "end_user").await;
    let jwt = session(&app, "dee").await;
    let secret = app
        .send_as(&jwt, Method::POST, "/api/v1/me/totp/enroll", None)
        .await
        .json["secret"]
        .as_str()
        .unwrap()
        .to_string();
    app.send_as(
        &jwt,
        Method::POST,
        "/api/v1/me/totp/confirm",
        Some(json!({ "code": totp_code(&secret, 0) })),
    )
    .await;
    let off_wrong = app
        .send_as(
            &jwt,
            Method::POST,
            "/api/v1/me/totp/disable",
            Some(json!({ "code": "000000" })),
        )
        .await;
    assert_eq!(off_wrong.status, StatusCode::UNAUTHORIZED);
    let off = app
        .send_as(
            &jwt,
            Method::POST,
            "/api/v1/me/totp/disable",
            Some(json!({ "code": totp_code(&secret, 1) })),
        )
        .await;
    assert_eq!(off.status, StatusCode::NO_CONTENT, "{}", off.text);
    let status = app
        .send_as(&jwt, Method::GET, "/api/v1/me/totp", None)
        .await;
    assert_eq!(status.json["enabled"], false, "{}", status.text);
    assert!(app.login("dee", PASSWORD).await.json["token"].is_string());
    let renew_off = app
        .send_as(
            &jwt,
            Method::POST,
            "/api/v1/me/totp/recovery-codes",
            Some(json!({ "code": totp_code(&secret, 1) })),
        )
        .await;
    assert_ne!(renew_off.status, StatusCode::OK, "{}", renew_off.text);
    app.finish().await;
}

/// An administrator unlinks a user's identity at a provider: the next
/// sign-in there links again by the verified email, not by the old link,
/// and another tenant's administrator reaches none of it.
#[tokio::test]
async fn an_identity_is_unlinked_from_its_user() {
    let app = require_app!();
    let idp = StandInIdp::start(IDP_CLIENT, Some(IDP_SECRET))
        .await
        .unwrap();
    let provider = app.make_provider(&idp, json!({"jit": true})).await;
    let jane = json!({ "sub": "jane-1", "email": "jane@acme.test", "email_verified": true,
                       "preferred_username": "jane@acme.test" });
    let landing = app.sso_landing(&idp, &provider, jane).await;
    let signed_in = app.sso_exchange(&landing).await;
    assert_eq!(signed_in.status, StatusCode::OK, "{}", signed_in.text);
    let path = format!("/api/v1/users/jane/identities/{provider}");

    let from_b = app
        .admin_in(app.tenant_b, Method::DELETE, &path, None)
        .await;
    assert_eq!(from_b.status, StatusCode::NOT_FOUND, "{}", from_b.text);
    let unknown = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/users/jane/identities/{}", Uuid::new_v4()),
            None,
        )
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND, "{}", unknown.text);

    let unlinked = app.admin(Method::DELETE, &path, None).await;
    assert_eq!(unlinked.status, StatusCode::NO_CONTENT, "{}", unlinked.text);
    let listed = app
        .admin(Method::GET, "/api/v1/users/jane/identities", None)
        .await;
    assert_eq!(listed.json, json!([]), "{}", listed.text);
    let again = app.admin(Method::DELETE, &path, None).await;
    assert_eq!(again.status, StatusCode::NOT_FOUND, "{}", again.text);
    app.finish().await;
}

/// A separate SIP password: set, the phone's credential made from it and
/// the web password left alone; removed, SIP has no credential until a
/// password is set again.
#[tokio::test]
async fn a_separate_sip_password_is_set_and_removed() {
    let app = require_app!();
    app.create_user_with_role("phil", "end_user").await;
    let before = app.stored_user("phil").await;
    let path = "/api/v1/users/phil/digest-password";

    let crlf = app
        .admin(
            Method::PUT,
            path,
            Some(json!({ "digest_password": "a\r\nb" })),
        )
        .await;
    assert_eq!(crlf.status, StatusCode::BAD_REQUEST, "{}", crlf.text);
    let set = app
        .admin(
            Method::PUT,
            path,
            Some(json!({ "digest_password": "Ph0ne-only!", "store_for_provisioning": true })),
        )
        .await;
    assert_eq!(set.status, StatusCode::NO_CONTENT, "{}", set.text);
    let after = app.stored_user("phil").await;
    assert_eq!(
        after.sip_ha1_hash.as_deref(),
        Some(fcp_core::storage::password::compute_ha1("phil", REALM, "Ph0ne-only!").as_str())
    );
    assert_eq!(
        after.sip_provisioning_password.as_deref(),
        Some("Ph0ne-only!")
    );
    assert_eq!(after.password, before.password, "the web password stays");
    assert!(app.login("phil", PASSWORD).await.json["token"].is_string());

    // The web password changing now leaves the phone's alone.
    let web = app
        .admin(
            Method::PUT,
            "/api/v1/users/phil/password",
            Some(json!({ "password": "W3b-only-now!" })),
        )
        .await;
    assert_eq!(web.status, StatusCode::NO_CONTENT, "{}", web.text);
    assert_eq!(
        app.stored_user("phil").await.sip_ha1_hash,
        after.sip_ha1_hash
    );

    let from_b = app.admin_in(app.tenant_b, Method::DELETE, path, None).await;
    assert_eq!(from_b.status, StatusCode::NOT_FOUND, "{}", from_b.text);
    let removed = app.admin(Method::DELETE, path, None).await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT, "{}", removed.text);
    let gone = app.stored_user("phil").await;
    assert!(gone.digest_password.is_none());
    assert!(gone.sip_ha1_hash.is_none());
    assert!(gone.sip_provisioning_password.is_none());
    let twice = app.admin(Method::DELETE, path, None).await;
    assert_eq!(twice.status, StatusCode::NOT_FOUND, "{}", twice.text);

    // The next web password is the SIP one again.
    app.admin(
        Method::PUT,
        "/api/v1/users/phil/password",
        Some(json!({ "password": "B0th-again!" })),
    )
    .await;
    assert_eq!(
        app.stored_user("phil").await.sip_ha1_hash.as_deref(),
        Some(fcp_core::storage::password::compute_ha1("phil", REALM, "B0th-again!").as_str())
    );
    app.finish().await;
}

/// No one gives a role holding more than their own, or takes over the
/// account of a user who holds one: a user administrator neither makes a
/// platform administrator nor sets a tenant administrator's password.
#[tokio::test]
async fn no_one_gives_a_role_beyond_their_own() {
    let app = require_app!();
    app.create_user_with_role("ua", "user_admin").await;
    app.create_user_with_role("ta", "tenant_admin").await;
    app.create_user_with_role("boss", "tenant_admin").await;
    app.create_user_with_role("eu", "end_user").await;
    let ua = session(&app, "ua").await;
    let ta = session(&app, "ta").await;

    let create =
        |name: &str, role: &str| json!({ "username": name, "password": PASSWORD, "role": role });
    for role in [
        "super_admin",
        "admin",
        "tenant_admin",
        "telecom_admin",
        "read_only",
    ] {
        let res = app
            .send_as(
                &ua,
                Method::POST,
                "/api/v1/users",
                Some(create("made", role)),
            )
            .await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "{role}: {}", res.text);
    }
    for role in ["user_admin", "end_user"] {
        let name = format!("ua-{}", role.replace('_', "-"));
        let res = app
            .send_as(
                &ua,
                Method::POST,
                "/api/v1/users",
                Some(create(&name, role)),
            )
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{role}: {}", res.text);
    }
    let sa = app
        .send_as(
            &ta,
            Method::POST,
            "/api/v1/users",
            Some(create("root", "super_admin")),
        )
        .await;
    assert_eq!(sa.status, StatusCode::FORBIDDEN, "{}", sa.text);
    let promote = app
        .send_as(
            &ta,
            Method::PUT,
            "/api/v1/users/eu",
            Some(json!({ "role": "super_admin" })),
        )
        .await;
    assert_eq!(promote.status, StatusCode::FORBIDDEN, "{}", promote.text);
    let ok = app
        .send_as(
            &ta,
            Method::PUT,
            "/api/v1/users/eu",
            Some(json!({ "role": "read_only" })),
        )
        .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text);
    let self_promote = app
        .send_as(
            &ua,
            Method::PUT,
            "/api/v1/users/ua",
            Some(json!({ "role": "tenant_admin" })),
        )
        .await;
    assert_eq!(
        self_promote.status,
        StatusCode::FORBIDDEN,
        "{}",
        self_promote.text
    );

    // A tenant administrator's account is out of a user administrator's
    // reach, however it is asked.
    for (method, path, body) in [
        (
            Method::PUT,
            "/api/v1/users/boss/password",
            Some(json!({ "password": "Ta-ken0ver!" })),
        ),
        (
            Method::PUT,
            "/api/v1/users/boss/digest-password",
            Some(json!({ "digest_password": "Ta-ken0ver!" })),
        ),
        (Method::DELETE, "/api/v1/users/boss/digest-password", None),
        (
            Method::PUT,
            "/api/v1/users/boss",
            Some(json!({ "role": "end_user" })),
        ),
        (
            Method::PUT,
            "/api/v1/users/boss",
            Some(json!({ "email": "me@evil.test" })),
        ),
        (Method::DELETE, "/api/v1/users/boss/totp", None),
        (Method::POST, "/api/v1/users/boss/unlock", None),
        (Method::DELETE, "/api/v1/users/boss", None),
    ] {
        let res = app.send_as(&ua, method.clone(), path, body).await;
        assert_eq!(
            res.status,
            StatusCode::FORBIDDEN,
            "{method} {path}: {}",
            res.text
        );
    }
    assert!(app.login("boss", PASSWORD).await.json["token"].is_string());
    // An end user's is not.
    let theirs = app
        .send_as(
            &ua,
            Method::PUT,
            "/api/v1/users/ua-end-user/password",
            Some(json!({ "password": "Fr3sh-start!" })),
        )
        .await;
    assert_eq!(theirs.status, StatusCode::NO_CONTENT, "{}", theirs.text);

    // The platform's administrator gives any role.
    let root = app
        .admin(
            Method::POST,
            "/api/v1/users",
            Some(create("root", "super_admin")),
        )
        .await;
    assert_eq!(root.status, StatusCode::CREATED, "{}", root.text);
    app.finish().await;
}
