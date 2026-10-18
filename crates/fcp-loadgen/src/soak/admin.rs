//! The admin API, as the soak uses it: to seed its tenant, and as the
//! steady API traffic beside the calls (`docs/DEEP_TESTING.md` D6).

use anyhow::{bail, Context, Result};
use fcp_testkit::smoke::http_request;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

/// The admin API with the platform's bearer token.
pub struct Admin {
    /// `http://127.0.0.1:8080`.
    pub base: String,
    pub token: String,
}

impl Admin {
    /// One request; the body parsed when it is JSON.
    pub async fn call(
        &self,
        method: &str,
        path: &str,
        tenant: Option<&str>,
        bearer: Option<&str>,
        body: Option<&Value>,
    ) -> Result<(u16, Value)> {
        let auth = format!("Bearer {}", bearer.unwrap_or(&self.token));
        let mut headers = vec![
            ("Authorization", auth.as_str()),
            ("Content-Type", "application/json"),
        ];
        if let Some(tenant) = tenant {
            headers.push(("x-fcp-tenant-id", tenant));
        }
        let url = format!("{}{path}", self.base);
        let body = body.map(Value::to_string);
        let (code, text) = tokio::time::timeout(
            Duration::from_secs(30),
            http_request(method, &url, &headers, body.as_deref()),
        )
        .await
        .with_context(|| format!("{method} {path}: no answer in 30 s"))??;
        Ok((
            code,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        ))
    }

    /// As [`Admin::call`], wanting one of `ok`.
    async fn want(
        &self,
        method: &str,
        path: &str,
        tenant: Option<&str>,
        body: Option<&Value>,
        ok: &[u16],
    ) -> Result<Value> {
        let (code, answer) = self.call(method, path, tenant, None, body).await?;
        if !ok.contains(&code) {
            bail!("{method} {path}: {code} {answer}");
        }
        Ok(answer)
    }
}

/// What the soak seeds: one tenant, its phones' accounts, the
/// extensions the callees answer on, and the rooms.
#[derive(Debug, Clone)]
pub struct SeedPlan {
    pub tenant: String,
    pub realm: String,
    pub password: String,
    /// `(username, extension)`; an extension of `None` has none.
    pub users: Vec<(String, Option<String>)>,
    pub rooms: Vec<String>,
}

/// Seed the plan, keeping what is there: a second soak on the same stack
/// finds its tenant, users and rooms. Returns the tenant's id.
pub async fn seed(admin: &Admin, plan: &SeedPlan) -> Result<String> {
    let listed = admin
        .want("GET", "/api/v1/tenants?limit=500", None, None, &[200])
        .await?;
    let found = listed["data"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["name"] == plan.tenant.as_str())
        .and_then(|t| t["id"].as_str())
        .map(str::to_string);
    let tenant = match found {
        Some(id) => id,
        None => {
            let made = admin
                .want(
                    "POST",
                    "/api/v1/tenants",
                    None,
                    Some(&json!({ "name": plan.tenant })),
                    &[201],
                )
                .await?;
            let id = made["id"].as_str().context("a tenant id")?.to_string();
            admin
                .want(
                    "PUT",
                    &format!("/api/v1/tenants/{id}"),
                    None,
                    Some(&json!({ "sip_domains": [plan.realm] })),
                    &[200],
                )
                .await?;
            id
        }
    };
    let t = Some(tenant.as_str());
    for (username, extension) in &plan.users {
        let (code, made) = admin
            .call(
                "POST",
                "/api/v1/users",
                t,
                None,
                Some(&json!({
                    "username": username,
                    "password": plan.password,
                    "display_name": username,
                    "tenant_id": tenant,
                })),
            )
            .await?;
        let id = match code {
            201 => made["id"].as_str().context("a user id")?.to_string(),
            409 => {
                let user = admin
                    .want("GET", &format!("/api/v1/users/{username}"), t, None, &[200])
                    .await?;
                user["id"].as_str().context("a user id")?.to_string()
            }
            _ => bail!("POST /api/v1/users {username}: {code} {made}"),
        };
        if let Some(number) = extension {
            admin
                .want(
                    "POST",
                    "/api/v1/extensions",
                    t,
                    Some(&json!({
                        "number": number,
                        "target": { "type": "user", "id": id },
                        "tenant_id": tenant,
                    })),
                    &[201, 409],
                )
                .await?;
        }
    }
    for room in &plan.rooms {
        admin
            .want(
                "POST",
                "/api/v1/conferences/meetings",
                t,
                Some(&json!({ "room_number": room, "name": format!("Soak {room}") })),
                &[201, 409],
            )
            .await?;
    }
    Ok(tenant)
}

/// The API traffic's outcomes: per operation, per status.
#[derive(Default)]
pub struct ApiCounts {
    counts: Mutex<BTreeMap<String, BTreeMap<u16, u64>>>,
}

impl ApiCounts {
    fn record(&self, op: &str, code: u16) {
        let mut counts = self.counts.lock().unwrap();
        *counts
            .entry(op.to_string())
            .or_default()
            .entry(code)
            .or_default() += 1;
    }

    pub fn snapshot(&self) -> BTreeMap<String, BTreeMap<u16, u64>> {
        self.counts.lock().unwrap().clone()
    }

    /// Answers that were not what the operation expects: a 5xx, or no
    /// answer at all (0).
    pub fn failures(&self) -> u64 {
        self.snapshot()
            .values()
            .flat_map(|by_code| by_code.iter())
            .filter(|(code, _)| **code == 0 || **code >= 500)
            .map(|(_, n)| n)
            .sum()
    }
}

/// One round of what an administrator and a user do through the API:
/// list users, records, rooms and nodes; make an extension and delete
/// it; change a user's calling settings and change them back; sign in
/// and sign out. `user` places no calls, so its do-not-disturb turns
/// nobody away.
pub async fn api_round(
    admin: &Admin,
    tenant: &str,
    user: &str,
    user_id: &str,
    password: &str,
    round: u64,
    counts: &ApiCounts,
) {
    let t = Some(tenant);
    let note = |op: &str, result: Result<(u16, Value)>| -> Option<Value> {
        match result {
            Ok((code, body)) => {
                counts.record(op, code);
                if code >= 500 {
                    tracing::warn!(op, code, %body, "the API failed");
                }
                Some(body)
            }
            Err(e) => {
                counts.record(op, 0);
                tracing::warn!(op, error = %e, "the API did not answer");
                None
            }
        }
    };
    for (op, path) in [
        ("list users", "/api/v1/users?limit=50"),
        ("list records", "/api/v1/cdrs?limit=20"),
        ("list rooms", "/api/v1/conferences/meetings"),
        ("list calls", "/api/v1/calls"),
    ] {
        note(op, admin.call("GET", path, t, None, None).await);
    }
    note(
        "list nodes",
        admin
            .call("GET", "/api/v1/cluster/nodes", None, None, None)
            .await,
    );

    // An extension made and deleted: rows that must not stay behind.
    let number = format!("59{:02}", round % 100);
    let made = note(
        "make extension",
        admin
            .call(
                "POST",
                "/api/v1/extensions",
                t,
                None,
                Some(&json!({
                    "number": number,
                    "target": { "type": "user", "id": user_id },
                    "tenant_id": tenant,
                })),
            )
            .await,
    );
    if made.as_ref().is_some_and(|m| m["id"].is_string()) {
        note(
            "delete extension",
            admin
                .call(
                    "DELETE",
                    &format!("/api/v1/extensions/{number}"),
                    t,
                    None,
                    None,
                )
                .await,
        );
    }

    // Do-not-disturb on and off.
    for dnd in [true, false] {
        note(
            "calling settings",
            admin
                .call(
                    "PUT",
                    &format!("/api/v1/users/{user}/calling"),
                    t,
                    None,
                    Some(&json!({ "dnd": dnd })),
                )
                .await,
        );
    }

    // A person signs in and out.
    let signed_in = note(
        "sign in",
        admin
            .call(
                "POST",
                "/api/v1/auth/login",
                None,
                None,
                Some(&json!({ "username": user, "password": password, "tenant_id": tenant })),
            )
            .await,
    );
    if let Some(token) = signed_in.as_ref().and_then(|s| s["token"].as_str()) {
        note(
            "read me",
            admin
                .call("GET", "/api/v1/me/profile", None, Some(token), None)
                .await,
        );
        note(
            "sign out",
            admin
                .call("POST", "/api/v1/auth/logout", None, Some(token), None)
                .await,
        );
    }
}
