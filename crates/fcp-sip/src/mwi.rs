//! MWI (Message Waiting Indicator) notification support.
//!
//! Implements RFC 3842 unsolicited NOTIFY for voicemail message-waiting
//! indication. Resolves the target endpoint(s) from storage and registrar,
//! then sends SIP NOTIFY with `application/simple-message-summary` body.

use std::sync::Arc;

use anyhow::{Context, Result};
use sip_core::SipUri;
use tracing::{debug, info, warn};

use crate::registrar::FcpRegistrar;
use crate::uac::SipUserAgentClient;
use fcp_core::storage::{DeviceStore, LineBindingStore};
use fcp_core::MwiStatus;

/// Sends MWI notifications over SIP.
///
/// Resolves `line_binding_id` → devices → AORs → registered contacts,
/// then sends unsolicited NOTIFY to each registered contact.
pub struct SipMwiNotifier {
    uac: Arc<SipUserAgentClient>,
    registrar: Arc<FcpRegistrar>,
    storage: Arc<fcp_core::DynStorage>,
}

impl SipMwiNotifier {
    pub fn new(
        uac: Arc<SipUserAgentClient>,
        registrar: Arc<FcpRegistrar>,
        storage: Arc<fcp_core::DynStorage>,
    ) -> Self {
        Self {
            uac,
            registrar,
            storage,
        }
    }
}

#[async_trait::async_trait]
impl fcp_core::MwiNotifier for SipMwiNotifier {
    async fn notify_mwi(&self, mwi_status: &MwiStatus) -> Result<()> {
        // Step 1: Look up the line binding to find devices
        let line_binding = self
            .storage
            .get_line_binding(&mwi_status.line_binding_id)
            .await
            .context("Failed to look up line binding for MWI")?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Line binding not found: {}",
                    mwi_status.line_binding_id
                )
            })?;

        // Step 2: For each device, look up its SIP URI and find registered contacts
        let mut notified = 0u32;
        for device_binding in &line_binding.devices {
            if !device_binding.enabled {
                continue;
            }

            let device = match self
                .storage
                .get_device(&device_binding.device_id)
                .await
                .context("Failed to look up device")?
            {
                Some(d) => d,
                None => {
                    warn!(
                        device_id = %device_binding.device_id,
                        "Device not found for MWI notification"
                    );
                    continue;
                }
            };

            // The device's sip_uri is its AOR (address of record)
            let aor = match &device.sip_uri {
                Some(uri) => uri.clone(),
                None => {
                    debug!(
                        device_id = %device_binding.device_id,
                        "Device has no SIP URI, skipping MWI"
                    );
                    continue;
                }
            };

            // Step 3: Look up registered contacts for this AOR
            let registrations = self
                .registrar
                .get_registrations(&aor)
                .await
                .unwrap_or_default();

            if registrations.is_empty() {
                debug!(aor = %aor, "No active registrations for MWI target");
                continue;
            }

            // Step 4: Build the MWI body (RFC 3842)
            let messages_waiting = if mwi_status.has_messages() {
                "yes"
            } else {
                "no"
            };

            let body = format!(
                "Messages-Waiting: {}\r\nVoice-Message: {}/{} ({}/{})\r\n",
                messages_waiting,
                mwi_status.new_messages,
                mwi_status.old_messages,
                mwi_status.urgent_new_messages,
                mwi_status.urgent_old_messages,
            );

            // Send NOTIFY to each registered contact
            for binding in &registrations {
                let contact_str = binding.contact();
                let target_uri = match SipUri::parse(contact_str) {
                    Ok(uri) => uri,
                    Err(_) => {
                        warn!(contact = %contact_str, "Failed to parse contact URI for MWI");
                        continue;
                    }
                };

                match self
                    .uac
                    .send_unsolicited_notify(
                        &target_uri,
                        "message-summary",
                        "application/simple-message-summary",
                        &body,
                    )
                    .await
                {
                    Ok(response) => {
                        if response.code() >= 200 && response.code() < 300 {
                            info!(
                                contact = %contact_str,
                                "MWI NOTIFY accepted"
                            );
                            notified += 1;
                        } else {
                            warn!(
                                contact = %contact_str,
                                code = response.code(),
                                "MWI NOTIFY rejected"
                            );
                        }
                    }
                    Err(e) => {
                        warn!(
                            contact = %contact_str,
                            error = %e,
                            "Failed to send MWI NOTIFY"
                        );
                    }
                }
            }
        }

        if notified > 0 {
            info!(
                mailbox_id = %mwi_status.mailbox_id,
                notified_contacts = notified,
                "MWI notifications sent"
            );
        } else {
            debug!(
                mailbox_id = %mwi_status.mailbox_id,
                "No contacts were notified for MWI"
            );
        }

        Ok(())
    }
}
