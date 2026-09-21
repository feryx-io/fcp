//! What a run is pointed at: the node's SIP address, the accounts to use,
//! and the ways of watching the node from outside the calls.

use anyhow::{Context, Result};
use fcp_testkit::Phone;
use std::net::SocketAddr;

/// The node under load, and the credentials to reach it with.
#[derive(Debug, Clone)]
pub struct Target {
    /// Where the INVITEs and REGISTERs go.
    pub sip: SocketAddr,
    /// The SIP realm the accounts live in.
    pub realm: String,
    /// Accounts are `{user_prefix}{n}`, numbered from `first_user`.
    pub user_prefix: String,
    pub first_user: u32,
    /// The password every account shares. A node with
    /// `auth_enabled = false` ignores it.
    pub password: String,
    /// What a call is placed to: an extension, a room, a number. `{n}`
    /// in it is replaced by the caller's own number, so a run can pair
    /// callers with callees.
    pub destination: String,
    /// The node's `/metrics`, when it has one to scrape.
    pub metrics_url: Option<String>,
    /// The node's process, when it is on this machine, for its resident
    /// memory and CPU.
    pub pid: Option<u32>,
}

impl Target {
    /// The account name for the `n`th phone of a run.
    pub fn user(&self, n: u32) -> String {
        format!("{}{}", self.user_prefix, self.first_user + n)
    }

    /// Where the `n`th phone calls: the destination with `{n}` filled in.
    pub fn destination_for(&self, n: u32) -> String {
        self.destination
            .replace("{n}", &(self.first_user + n).to_string())
    }

    /// A phone for the `n`th account, bound to an ephemeral local port.
    pub async fn phone(&self, n: u32) -> Result<Phone> {
        Phone::new(&self.user(n), &self.password, &self.realm, self.sip)
            .await
            .with_context(|| format!("phone {} could not start", self.user(n)))
    }
}
