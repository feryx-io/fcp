//! Address validation providers (`docs/E911_BLOCK_2.md` §7).
//!
//! `basic` (`fcp_core::e911::BasicAddressValidator`) checks completeness
//! and splits the street into RFC 5139 elements with no external call.
//! `http` posts the address to a service that can answer against an
//! MSAG-grade database — the contract a thin adapter for any such provider
//! implements:
//!
//! ```json
//! POST <url>                     Authorization: Bearer <token>
//! { "address": { "address1": "1455 Market St", "city": "San Francisco",
//!                "state": "CA", "postal_code": "94103", "country": "US",
//!                "house_number": "1455", "street_name": "Market",
//!                "street_suffix": "St" } }
//!
//! 200 { "valid": true,
//!       "normalized": { …the same shape… },
//!       "issues": [], "notes": [], "provider_reference": "abc-123" }
//! ```
//!
//! An answer that does not arrive — a timeout, a 5xx, a body that does not
//! parse — is an error, not an invalid address: nothing is marked
//! validated on a provider that could not be reached.

use async_trait::async_trait;
use fcp_core::e911::{AddressValidation, BasicAddressValidator, E911AddressValidator};
use fcp_core::E911Address;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// A validator that asks an HTTP service.
pub struct HttpAddressValidator {
    name: String,
    url: String,
    token: Option<String>,
    client: reqwest::Client,
}

#[derive(Debug, Serialize)]
struct ValidateRequest<'a> {
    address: &'a E911Address,
}

#[derive(Debug, Deserialize)]
struct ValidateReply {
    valid: bool,
    #[serde(default)]
    normalized: Option<E911Address>,
    #[serde(default)]
    issues: Vec<String>,
    #[serde(default)]
    notes: Vec<String>,
    #[serde(default)]
    provider_reference: Option<String>,
}

impl HttpAddressValidator {
    /// `name` is what the address is marked validated by (`http` unless
    /// the deployment names its provider); `timeout` bounds the call, which
    /// sits in front of an administrator waiting for an answer.
    pub fn new(
        name: impl Into<String>,
        url: impl Into<String>,
        token: Option<String>,
        timeout: Duration,
    ) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder().timeout(timeout).build()?;
        Ok(Self {
            name: name.into(),
            url: url.into(),
            token,
            client,
        })
    }
}

#[async_trait]
impl E911AddressValidator for HttpAddressValidator {
    fn name(&self) -> &str {
        &self.name
    }

    async fn validate(&self, address: &E911Address) -> anyhow::Result<AddressValidation> {
        // Send the tidied, structured address: a provider that only checks
        // it against a database should not also have to parse it.
        let local = BasicAddressValidator::check(address);
        let mut request = self.client.post(&self.url).json(&ValidateRequest {
            address: &local.normalized,
        });
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let body: String = body.chars().take(200).collect();
            anyhow::bail!("validator {} answered {status}: {body}", self.name);
        }
        let reply: ValidateReply = response.json().await?;
        // Check what would be stored, whichever address that is: an answer
        // this platform cannot dispatch to is not validated however sure
        // the provider is, and one the provider did not structure is
        // structured here.
        let answer = reply.normalized.unwrap_or(local.normalized);
        let checked = BasicAddressValidator::check(&answer);
        let mut normalized = checked.normalized;
        // A provider never decides that an address is validated; that is
        // the mark this platform writes once it accepts the answer.
        normalized.clear_validation();
        let mut issues = reply.issues;
        for issue in checked.issues {
            if !issues.contains(&issue) {
                issues.push(issue);
            }
        }
        if !reply.valid && issues.is_empty() {
            issues.push(format!("{} rejected this address", self.name));
        }
        let mut notes = checked.notes;
        notes.extend(reply.notes);
        Ok(AddressValidation {
            valid: reply.valid && issues.is_empty(),
            normalized,
            issues,
            notes,
            validator: self.name.clone(),
            provider_reference: reply.provider_reference,
        })
    }
}
