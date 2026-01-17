//! Tests for SQLite storage backend
//!
//! These tests verify that the SQLite storage backend correctly implements
//! all storage traits and properly persists/retrieves data.

use super::*;
use crate::{
    line_binding::*, phone_number::*, porting::*, voicemail::*, Address, CallDetailRecord,
    CallDisposition, Device, DeviceType, Extension, ExtensionTarget, HangupCause, HangupParty,
    MediaPreferences, MessagePriority, MessageStatus, NumberState, Registration,
    RegistrationState, TenantId, User,
};
use std::time::{Duration, SystemTime};

/// Helper to create an in-memory SQLite storage for testing
async fn create_test_storage() -> SqliteStorage {
    SqliteStorage::new(":memory:")
        .await
        .expect("Failed to create in-memory SQLite storage")
}

/// Helper to create a test tenant ID
fn test_tenant_id() -> TenantId {
    TenantId::new()
}

#[cfg(test)]
mod credential_store_tests {
    use super::*;

    #[tokio::test]
    async fn test_user_crud() {
        let storage = create_test_storage().await;
        let tenant_id = test_tenant_id();

        // Create a user
        let user = User {
            username: "alice".to_string(),
            password: "hashed_password".to_string(),
            digest_password: Some("digest_hash".to_string()),
            sip_ha1_hash: Some("ha1_hash".to_string()),
            display_name: Some("Alice Smith".to_string()),
            tenant_id: tenant_id.clone(),
            enabled: true,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            media_preferences: MediaPreferences::default(),
            email: Some("alice@example.com".to_string()),
            password_last_reset: None,
            phone_number: Some("+15551234567".to_string()),
            mobile_number: None,
            pin: None,
            extension: Some("1001".to_string()),
            department: Some("Engineering".to_string()),
            job_title: Some("Developer".to_string()),
            location: Some("San Francisco".to_string()),
            language: "en-US".to_string(),
            timezone: "America/Los_Angeles".to_string(),
            account_status: crate::AccountStatus::Active,
            last_login: None,
            emergency_contact: crate::EmergencyContact::default(),
            primary_phone_number_id: None,
            voicemail_box_id: None,
        };

        // Insert user
        storage
            .put_user(user.clone())
            .await
            .expect("Failed to insert user");

        // Read user
        let loaded = storage
            .get_user(&user.username)
            .await
            .expect("Failed to get user")
            .expect("User not found");

        assert_eq!(loaded.username, user.username);
        assert_eq!(loaded.password, user.password);
        assert_eq!(loaded.email, user.email);
        assert_eq!(loaded.extension, user.extension);

        // Update user
        let mut updated_user = loaded.clone();
        updated_user.email = Some("alice.new@example.com".to_string());
        storage
            .put_user(updated_user.clone())
            .await
            .expect("Failed to update user");

        let loaded_updated = storage
            .get_user(&user.username)
            .await
            .expect("Failed to get updated user")
            .expect("User not found");

        assert_eq!(
            loaded_updated.email,
            Some("alice.new@example.com".to_string())
        );

        // List users
        let users = storage.list_users().await.expect("Failed to list users");
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].username, "alice");

        // Delete user
        storage
            .delete_user(&user.username)
            .await
            .expect("Failed to delete user");

        let deleted = storage
            .get_user(&user.username)
            .await
            .expect("Failed to get deleted user");
        assert!(deleted.is_none());
    }

    #[tokio::test]
    async fn test_user_not_found() {
        let storage = create_test_storage().await;

        let result = storage
            .get_user("nonexistent")
            .await
            .expect("Query should succeed");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_user_phone_fields_persistence() {
        let storage = create_test_storage().await;
        let tenant_id = test_tenant_id();

        // Create phone number IDs and voicemail box ID
        let phone_id = crate::PhoneNumberId::new();
        let voicemail_id = crate::VoicemailBoxId::new();

        let user = User {
            username: "bob".to_string(),
            password: "hashed_password".to_string(),
            digest_password: None,
            sip_ha1_hash: None,
            display_name: Some("Bob Smith".to_string()),
            tenant_id: tenant_id.clone(),
            enabled: true,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            media_preferences: crate::MediaPreferences::default(),
            email: Some("bob@example.com".to_string()),
            password_last_reset: Some(SystemTime::now()),
            phone_number: Some("+15559876543".to_string()),
            mobile_number: None,
            pin: None,
            extension: Some("1002".to_string()),
            department: Some("Sales".to_string()),
            job_title: Some("Manager".to_string()),
            location: Some("New York".to_string()),
            language: "en-US".to_string(),
            timezone: "America/New_York".to_string(),
            account_status: crate::AccountStatus::Active,
            last_login: None,
            emergency_contact: crate::EmergencyContact::default(),
            primary_phone_number_id: Some(phone_id.clone()),
            voicemail_box_id: Some(voicemail_id.clone()),
        };

        // Insert user with phone fields set
        storage
            .put_user(user.clone())
            .await
            .expect("Failed to insert user");

        // Load user and verify phone fields are persisted
        let loaded = storage
            .get_user(&user.username)
            .await
            .expect("Failed to get user")
            .expect("User not found");

        assert_eq!(
            loaded.primary_phone_number_id, user.primary_phone_number_id,
            "primary_phone_number_id should be persisted"
        );
        assert_eq!(
            loaded.voicemail_box_id, user.voicemail_box_id,
            "voicemail_box_id should be persisted"
        );

        // Update to clear the fields
        let mut updated_user = loaded.clone();
        updated_user.primary_phone_number_id = None;
        updated_user.voicemail_box_id = None;
        storage
            .put_user(updated_user)
            .await
            .expect("Failed to update user");

        // Verify fields are now None
        let loaded_updated = storage
            .get_user(&user.username)
            .await
            .expect("Failed to get updated user")
            .expect("User not found");

        assert_eq!(loaded_updated.primary_phone_number_id, None);
        assert_eq!(loaded_updated.voicemail_box_id, None);
    }
}

#[cfg(test)]
mod registration_store_tests {
    use super::*;

    #[tokio::test]
    async fn test_registration_crud() {
        let storage = create_test_storage().await;

        let registration = Registration {
            aor: "alice@example.com".to_string(),
            contact: "sip:alice@192.168.1.100:5060".to_string(),
            expires_at: SystemTime::now() + Duration::from_secs(3600),
            call_id: "test-call-id".to_string(),
            cseq: 1,
            user_agent: Some("Test UA".to_string()),
            q: Some(1.0),
            registered_at: SystemTime::now(),
        };

        // Insert registration
        storage
            .put_registration(registration.clone())
            .await
            .expect("Failed to insert registration");

        // Get registrations
        let registrations = storage
            .get_registrations(&registration.aor)
            .await
            .expect("Failed to get registrations");

        assert_eq!(registrations.len(), 1);
        assert_eq!(registrations[0].aor, registration.aor);
        assert_eq!(registrations[0].contact, registration.contact);
        assert_eq!(registrations[0].call_id, registration.call_id);

        // Delete registration
        storage
            .delete_registration(&registration.aor, &registration.contact)
            .await
            .expect("Failed to delete registration");

        let empty = storage
            .get_registrations(&registration.aor)
            .await
            .expect("Failed to get registrations after delete");
        assert_eq!(empty.len(), 0);
    }

    #[tokio::test]
    async fn test_cleanup_expired() {
        let storage = create_test_storage().await;

        // Create expired registration
        let expired = Registration {
            aor: "bob@example.com".to_string(),
            contact: "sip:bob@192.168.1.101:5060".to_string(),
            expires_at: SystemTime::now() - Duration::from_secs(3600), // Expired 1 hour ago
            call_id: "expired-call-id".to_string(),
            cseq: 1,
            user_agent: Some("Test UA".to_string()),
            q: Some(1.0),
            registered_at: SystemTime::now() - Duration::from_secs(7200),
        };

        // Create valid registration
        let valid = Registration {
            aor: "alice@example.com".to_string(),
            contact: "sip:alice@192.168.1.100:5060".to_string(),
            expires_at: SystemTime::now() + Duration::from_secs(3600),
            call_id: "valid-call-id".to_string(),
            cseq: 1,
            user_agent: Some("Test UA".to_string()),
            q: Some(1.0),
            registered_at: SystemTime::now(),
        };

        storage
            .put_registration(expired.clone())
            .await
            .expect("Failed to insert expired");
        storage
            .put_registration(valid.clone())
            .await
            .expect("Failed to insert valid");

        // Cleanup expired
        let count = storage
            .cleanup_expired()
            .await
            .expect("Failed to cleanup expired");
        assert_eq!(count, 1);

        // Verify only valid registration remains
        let valid_regs = storage
            .get_registrations(&valid.aor)
            .await
            .expect("Failed to get valid registrations");
        assert_eq!(valid_regs.len(), 1);

        let expired_regs = storage
            .get_registrations(&expired.aor)
            .await
            .expect("Failed to get expired registrations");
        assert_eq!(expired_regs.len(), 0);
    }
}

#[cfg(test)]
mod cdr_store_tests {
    use super::*;

    #[tokio::test]
    async fn test_cdr_crud() {
        let storage = create_test_storage().await;
        let tenant_id = test_tenant_id();
        let call_id = crate::CallId::new();

        let cdr = CallDetailRecord {
            call_id: call_id.clone(),
            tenant_id: tenant_id.clone(),
            sip_call_id: "sip-call-id-123".to_string(),
            from_uri: "sip:alice@example.com".to_string(),
            from_display_name: Some("Alice".to_string()),
            to_uri: "sip:bob@example.com".to_string(),
            to_display_name: Some("Bob".to_string()),
            start_time: SystemTime::now(),
            answer_time: Some(SystemTime::now() + Duration::from_secs(2)),
            end_time: Some(SystemTime::now() + Duration::from_secs(62)),
            setup_duration_ms: Some(2000),
            call_duration_ms: Some(60000),
            total_duration_ms: Some(62000),
            disposition: CallDisposition::Answered,
            hangup_cause: HangupCause::NormalClearing,
            hangup_party: Some(HangupParty::Caller),
            sip_status_code: Some(200),
            sip_reason: Some("OK".to_string()),
            route_id: None,
            gateway_id: None,
            media_anchored: false,
            codec_a_leg: Some("PCMU".to_string()),
            codec_b_leg: Some("PCMU".to_string()),
            rtp_packets_sent: Some(3000),
            rtp_packets_received: Some(2980),
            rtp_bytes_sent: Some(480000),
            rtp_bytes_received: Some(476800),
            recording_url: None,
            cost: None,
            currency: None,
            custom_data: None,
        };

        // Insert CDR
        storage
            .insert_cdr(cdr.clone())
            .await
            .expect("Failed to insert CDR");

        // Get CDR
        let loaded = storage
            .get_cdr(&call_id)
            .await
            .expect("Failed to get CDR")
            .expect("CDR not found");

        assert_eq!(loaded.call_id, cdr.call_id);
        assert_eq!(loaded.from_uri, cdr.from_uri);
        assert_eq!(loaded.to_uri, cdr.to_uri);
        assert_eq!(loaded.disposition, cdr.disposition);

        // Query CDRs
        let query = CdrQuery {
            tenant_id: Some(tenant_id.clone()),
            call_id: None,
            from_uri: Some("sip:alice@example.com".to_string()),
            to_uri: None,
            start_time_from: Some(SystemTime::now() - Duration::from_secs(60)),
            start_time_to: Some(SystemTime::now() + Duration::from_secs(60)),
            disposition: None,
            limit: Some(10),
            offset: None,
        };

        let results = storage
            .query_cdrs(query.clone())
            .await
            .expect("Failed to query CDRs");
        assert_eq!(results.len(), 1);

        // Count CDRs
        let count = storage
            .count_cdrs(query)
            .await
            .expect("Failed to count CDRs");
        assert_eq!(count, 1);
    }
}

#[cfg(test)]
mod phone_number_store_tests {
    use super::*;

    #[tokio::test]
    async fn test_phone_number_crud() {
        let storage = create_test_storage().await;
        let tenant_id = test_tenant_id();

        let phone_number = PhoneNumber {
            id: crate::PhoneNumberId::new(),
            e164: "+15551234567".to_string(),
            country_code: "1".to_string(),
            npa: "555".to_string(),
            nxx: "123".to_string(),
            line: "4567".to_string(),
            state: NumberState::Active,
            tenant_id: tenant_id.clone(),
            e911_address: None,
            e911_callback: None,
            cnam: Some("Test Number".to_string()),
            spam_score: None,
            carrier: Some("Test Carrier".to_string()),
            lrn: None,
            ocn: None,
            rate_center: Some("San Francisco".to_string()),
            lata: Some("722".to_string()),
            acquired_at: SystemTime::now(),
            activated_at: Some(SystemTime::now()),
            ported_at: None,
            loa_date: None,
            foc_date: None,
            recording_enabled: false,
            block_anonymous: false,
            allow_list: vec![],
            block_list: vec![],
            tags: vec![],
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
        };

        // Insert phone number
        storage
            .put_phone_number(phone_number.clone())
            .await
            .expect("Failed to insert phone number");

        // Get phone number by ID
        let loaded = storage
            .get_phone_number(&phone_number.id)
            .await
            .expect("Failed to get phone number")
            .expect("Phone number not found");

        assert_eq!(loaded.id, phone_number.id);
        assert_eq!(loaded.e164, phone_number.e164);

        // Get phone number by E.164
        let loaded_e164 = storage
            .get_phone_number_by_e164(&phone_number.e164)
            .await
            .expect("Failed to get phone number by E.164")
            .expect("Phone number not found");

        assert_eq!(loaded_e164.id, phone_number.id);

        // List phone numbers
        let numbers = storage
            .list_phone_numbers(&tenant_id)
            .await
            .expect("Failed to list phone numbers");
        assert_eq!(numbers.len(), 1);

        // Delete phone number
        storage
            .delete_phone_number(&phone_number.id)
            .await
            .expect("Failed to delete phone number");

        let deleted = storage
            .get_phone_number(&phone_number.id)
            .await
            .expect("Failed to get deleted phone number");
        assert!(deleted.is_none());
    }
}

#[cfg(test)]
mod line_binding_store_tests {
    use super::*;

    #[tokio::test]
    async fn test_line_binding_crud() {
        let storage = create_test_storage().await;
        let tenant_id = test_tenant_id();

        let binding = LineBinding {
            id: crate::LineBindingId::new(),
            phone_number_id: crate::PhoneNumberId::new(),
            owner_user_id: Some(crate::UserId::new()),
            binding_type: BindingType::User,
            devices: vec![],
            ring_strategy: RingStrategy::Simultaneous,
            tenant_id: tenant_id.clone(),
            no_answer_timeout: Duration::from_secs(30),
            forward_on_busy: None,
            forward_on_no_answer: None,
            forward_unconditional: None,
            outbound_caller_id: None,
            outbound_cnam: None,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
        };

        // Insert line binding
        storage
            .put_line_binding(binding.clone())
            .await
            .expect("Failed to insert line binding");

        // Get line binding
        let loaded = storage
            .get_line_binding(&binding.id)
            .await
            .expect("Failed to get line binding")
            .expect("Line binding not found");

        assert_eq!(loaded.id, binding.id);
        assert_eq!(loaded.phone_number_id, binding.phone_number_id);

        // Get line binding by phone number
        let by_phone = storage
            .get_line_binding_by_phone_number(&binding.phone_number_id)
            .await
            .expect("Failed to get by phone number")
            .expect("Line binding not found");

        assert_eq!(by_phone.id, binding.id);

        // List line bindings
        let bindings = storage
            .list_line_bindings(&tenant_id)
            .await
            .expect("Failed to list line bindings");
        assert_eq!(bindings.len(), 1);

        // Delete line binding
        storage
            .delete_line_binding(&binding.id)
            .await
            .expect("Failed to delete line binding");

        let deleted = storage
            .get_line_binding(&binding.id)
            .await
            .expect("Failed to get deleted line binding");
        assert!(deleted.is_none());
    }

    #[tokio::test]
    async fn test_list_line_bindings_by_user() {
        let storage = create_test_storage().await;
        let tenant_id = test_tenant_id();
        let user_id = crate::UserId::new();

        let binding1 = LineBinding {
            id: crate::LineBindingId::new(),
            phone_number_id: crate::PhoneNumberId::new(),
            owner_user_id: Some(user_id.clone()),
            binding_type: BindingType::User,
            devices: vec![],
            ring_strategy: RingStrategy::Simultaneous,
            tenant_id: tenant_id.clone(),
            no_answer_timeout: Duration::from_secs(30),
            forward_on_busy: None,
            forward_on_no_answer: None,
            forward_unconditional: None,
            outbound_caller_id: None,
            outbound_cnam: None,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
        };

        let binding2 = LineBinding {
            id: crate::LineBindingId::new(),
            phone_number_id: crate::PhoneNumberId::new(),
            owner_user_id: Some(crate::UserId::new()), // Different user
            binding_type: BindingType::User,
            devices: vec![],
            ring_strategy: RingStrategy::Simultaneous,
            tenant_id: tenant_id.clone(),
            no_answer_timeout: Duration::from_secs(30),
            forward_on_busy: None,
            forward_on_no_answer: None,
            forward_unconditional: None,
            outbound_caller_id: None,
            outbound_cnam: None,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
        };

        storage
            .put_line_binding(binding1.clone())
            .await
            .expect("Failed to insert binding1");
        storage
            .put_line_binding(binding2.clone())
            .await
            .expect("Failed to insert binding2");

        let user_bindings = storage
            .list_line_bindings_by_user(&user_id)
            .await
            .expect("Failed to list by user");

        assert_eq!(user_bindings.len(), 1);
        assert_eq!(user_bindings[0].id, binding1.id);
    }
}

#[cfg(test)]
mod extension_store_tests {
    use super::*;

    #[tokio::test]
    async fn test_extension_crud() {
        let storage = create_test_storage().await;
        let tenant_id = test_tenant_id();

        let extension = Extension {
            id: crate::ExtensionId::new(),
            number: "1234".to_string(),
            target: ExtensionTarget::User(crate::UserId::new()),
            tenant_id: tenant_id.clone(),
            description: Some("Test extension".to_string()),
            enabled: true,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
        };

        // Insert extension
        storage
            .put_extension(extension.clone())
            .await
            .expect("Failed to insert extension");

        // Get extension
        let loaded = storage
            .get_extension(&extension.id)
            .await
            .expect("Failed to get extension")
            .expect("Extension not found");

        assert_eq!(loaded.id, extension.id);
        assert_eq!(loaded.number, extension.number);

        // List extensions
        let extensions = storage
            .list_extensions(&tenant_id)
            .await
            .expect("Failed to list extensions");
        assert_eq!(extensions.len(), 1);

        // Delete extension
        storage
            .delete_extension(&extension.id)
            .await
            .expect("Failed to delete extension");

        let deleted = storage
            .get_extension(&extension.id)
            .await
            .expect("Failed to get deleted extension");
        assert!(deleted.is_none());
    }
}

#[cfg(test)]
mod device_store_tests {
    use super::*;

    #[tokio::test]
    async fn test_device_crud() {
        let storage = create_test_storage().await;
        let tenant_id = test_tenant_id();

        let device = Device {
            id: crate::DeviceId::new(),
            name: "Alice's Desk Phone".to_string(),
            device_type: DeviceType::DeskPhone,
            mac_address: Some("00:11:22:33:44:55".to_string()),
            model: Some("Cisco 8861".to_string()),
            firmware_version: Some("14.1.0".to_string()),
            tenant_id: tenant_id.clone(),
            owner_user_id: Some(crate::UserId::new()),
            sip_uri: Some("sip:alice-desk@192.168.1.100".to_string()),
            registration_state: RegistrationState::Registered,
            last_registration: Some(SystemTime::now()),
            last_ip_address: Some("192.168.1.100".to_string()),
            user_agent: Some("Cisco/CE14.1.0".to_string()),
            max_lines: 6,
            supports_video: true,
            supports_blf: true,
            codec_support: vec![],
            location: Some("Office".to_string()),
            e911_override: None,
            provisioning_url: None,
            config_template_id: None,
            enabled: true,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
        };

        // Insert device
        storage
            .put_device(device.clone())
            .await
            .expect("Failed to insert device");

        // Get device
        let loaded = storage
            .get_device(&device.id)
            .await
            .expect("Failed to get device")
            .expect("Device not found");

        assert_eq!(loaded.id, device.id);
        assert_eq!(loaded.name, device.name);

        // Get device by SIP URI
        let by_sip = storage
            .get_device_by_sip_uri(device.sip_uri.as_ref().unwrap())
            .await
            .expect("Failed to get by SIP URI")
            .expect("Device not found");

        assert_eq!(by_sip.id, device.id);

        // List devices
        let devices = storage
            .list_devices(&tenant_id)
            .await
            .expect("Failed to list devices");
        assert_eq!(devices.len(), 1);

        // Delete device
        storage
            .delete_device(&device.id)
            .await
            .expect("Failed to delete device");

        let deleted = storage
            .get_device(&device.id)
            .await
            .expect("Failed to get deleted device");
        assert!(deleted.is_none());
    }

    #[tokio::test]
    async fn test_list_devices_by_user() {
        let storage = create_test_storage().await;
        let tenant_id = test_tenant_id();
        let user_id = crate::UserId::new();

        let device1 = Device {
            id: crate::DeviceId::new(),
            name: "Device 1".to_string(),
            device_type: DeviceType::DeskPhone,
            mac_address: None,
            model: None,
            firmware_version: None,
            tenant_id: tenant_id.clone(),
            owner_user_id: Some(user_id.clone()),
            sip_uri: None,
            registration_state: RegistrationState::Unregistered,
            last_registration: None,
            last_ip_address: None,
            user_agent: None,
            max_lines: 1,
            supports_video: false,
            supports_blf: false,
            codec_support: vec![],
            location: None,
            e911_override: None,
            provisioning_url: None,
            config_template_id: None,
            enabled: true,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
        };

        let device2 = Device {
            id: crate::DeviceId::new(),
            name: "Device 2".to_string(),
            device_type: DeviceType::Softphone,
            mac_address: None,
            model: None,
            firmware_version: None,
            tenant_id: tenant_id.clone(),
            owner_user_id: Some(crate::UserId::new()), // Different user
            sip_uri: None,
            registration_state: RegistrationState::Unregistered,
            last_registration: None,
            last_ip_address: None,
            user_agent: None,
            max_lines: 1,
            supports_video: false,
            supports_blf: false,
            codec_support: vec![],
            location: None,
            e911_override: None,
            provisioning_url: None,
            config_template_id: None,
            enabled: true,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
        };

        storage
            .put_device(device1.clone())
            .await
            .expect("Failed to insert device1");
        storage
            .put_device(device2.clone())
            .await
            .expect("Failed to insert device2");

        let user_devices = storage
            .list_devices_by_user(&user_id)
            .await
            .expect("Failed to list by user");

        assert_eq!(user_devices.len(), 1);
        assert_eq!(user_devices[0].id, device1.id);
    }
}

#[cfg(test)]
mod voicemail_store_tests {
    use super::*;

    #[tokio::test]
    async fn test_voicemail_crud() {
        let storage = create_test_storage().await;
        let tenant_id = test_tenant_id();
        let user_id = crate::UserId::new();
        let line_binding_id = crate::LineBindingId::new();

        let mailbox = VoicemailMailbox {
            id: "vm-1234".to_string(),
            tenant_id: tenant_id.clone(),
            user_id: user_id.clone(),
            line_binding_id: line_binding_id.clone(),
            extension: "1234".to_string(),
            pin: "1234".to_string(),
            enabled: true,
            max_message_duration: 300,
            max_messages: 100,
            email_notification: true,
            email_address: Some("alice@example.com".to_string()),
            attach_audio: true,
            include_transcription: false,
            transcription_enabled: false,
            greetings: vec![],
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
        };

        // Insert mailbox
        storage
            .put_mailbox(mailbox.clone())
            .await
            .expect("Failed to insert mailbox");

        // Get mailbox
        let loaded = storage
            .get_mailbox(&mailbox.id)
            .await
            .expect("Failed to get mailbox")
            .expect("Mailbox not found");

        assert_eq!(loaded.id, mailbox.id);
        assert_eq!(loaded.extension, mailbox.extension);

        // Get mailbox by user
        let by_user = storage
            .get_mailbox_by_user(&user_id)
            .await
            .expect("Failed to get by user")
            .expect("Mailbox not found");

        assert_eq!(by_user.id, mailbox.id);

        // Get mailbox by line binding
        let by_line = storage
            .get_mailbox_by_line_binding(&line_binding_id)
            .await
            .expect("Failed to get by line binding")
            .expect("Mailbox not found");

        assert_eq!(by_line.id, mailbox.id);

        // List mailboxes
        let mailboxes = storage
            .list_mailboxes(&tenant_id)
            .await
            .expect("Failed to list mailboxes");
        assert_eq!(mailboxes.len(), 1);

        // Delete mailbox
        storage
            .delete_mailbox(&mailbox.id)
            .await
            .expect("Failed to delete mailbox");

        let deleted = storage
            .get_mailbox(&mailbox.id)
            .await
            .expect("Failed to get deleted mailbox");
        assert!(deleted.is_none());
    }

    #[tokio::test]
    async fn test_voicemail_message_crud() {
        let storage = create_test_storage().await;
        let mailbox_id = "vm-1234";
        let tenant_id = test_tenant_id();
        let user_id = crate::UserId::new();
        let line_binding_id = crate::LineBindingId::new();

        // Create mailbox first (required for foreign key constraint)
        let mailbox = VoicemailMailbox {
            id: mailbox_id.to_string(),
            tenant_id: tenant_id.clone(),
            user_id: user_id.clone(),
            line_binding_id: line_binding_id.clone(),
            extension: "1234".to_string(),
            pin: "1234".to_string(),
            enabled: true,
            max_messages: 100,
            max_message_duration: 300,
            email_notification: false,
            email_address: None,
            attach_audio: false,
            transcription_enabled: false,
            include_transcription: false,
            greetings: vec![],
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
        };
        storage.put_mailbox(mailbox.clone()).await.expect("Failed to insert mailbox");

        let message = VoicemailMessage {
            id: "msg-123".to_string(),
            mailbox_id: mailbox_id.to_string(),
            caller: "+15551234567".to_string(),
            caller_name: Some("Alice".to_string()),
            status: MessageStatus::New,
            priority: MessagePriority::Normal,
            duration: 45,
            // audio_format: .wav"),
            recording_path: std::path::PathBuf::from("/var/spool/voicemail/msg-123.wav"),
            // audio_size: 720000,
            transcription: None,
            created_at: SystemTime::now(),
            accessed_at: None,
            // metadata: std::collections::HashMap::new(),
        };

        // Insert message
        storage
            .put_message(message.clone())
            .await
            .expect("Failed to insert message");

        // Get message
        let loaded = storage
            .get_message(&message.id)
            .await
            .expect("Failed to get message")
            .expect("Message not found");

        assert_eq!(loaded.id, message.id);
        assert_eq!(loaded.status, MessageStatus::New);

        // List messages
        let messages = storage
            .list_messages(mailbox_id)
            .await
            .expect("Failed to list messages");
        assert_eq!(messages.len(), 1);

        // List messages by status
        let new_messages = storage
            .list_messages_by_status(mailbox_id, MessageStatus::New)
            .await
            .expect("Failed to list by status");
        assert_eq!(new_messages.len(), 1);

        // Get MWI status
        let mwi = storage
            .get_mwi_status(mailbox_id)
            .await
            .expect("Failed to get MWI status");

        assert_eq!(mwi.mailbox_id, mailbox_id);
        assert_eq!(mwi.new_messages, 1);
        assert_eq!(mwi.old_messages, 0);

        // Delete message
        storage
            .delete_message(&message.id)
            .await
            .expect("Failed to delete message");

        let deleted = storage
            .get_message(&message.id)
            .await
            .expect("Failed to get deleted message");
        assert!(deleted.is_none());
    }

    #[tokio::test]
    async fn test_mailbox_stats() {
        let storage = create_test_storage().await;
        let tenant_id = test_tenant_id();
        let user_id = crate::UserId::new();
        let line_binding_id = crate::LineBindingId::new();
        let mailbox_id = "vm-stats";

        let mailbox = VoicemailMailbox {
            id: mailbox_id.to_string(),
            tenant_id: tenant_id.clone(),
            user_id: user_id.clone(),
            line_binding_id: line_binding_id.clone(),
            extension: "5000".to_string(),
            pin: "5000".to_string(),
            enabled: true,
            max_messages: 100,
            max_message_duration: 300,
            email_notification: false,
            email_address: None,
            attach_audio: false,
            // delete_after_email: false,
            transcription_enabled: false,
            include_transcription: false,
            greetings: vec![],
            // greeting_unavailable: None,
            // greeting_busy: None,
            // greeting_name: None,
            // greeting_temporary: None,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
        };

        storage
            .put_mailbox(mailbox)
            .await
            .expect("Failed to insert mailbox");

        // Add some messages with different statuses
        for i in 0..3 {
            let message = VoicemailMessage {
                id: format!("msg-new-{}", i),
                mailbox_id: mailbox_id.to_string(),
                caller: "+15551234567".to_string(),
                caller_name: None,
                status: MessageStatus::New,
                priority: MessagePriority::Normal,
                duration: 30 + i * 10,
                recording_path: std::path::PathBuf::from(format!("/var/spool/voicemail/msg-{}.wav", i)),
                transcription: None,
                created_at: SystemTime::now(),
                accessed_at: None,
                // metadata: std::collections::HashMap::new(),
            };
            storage
                .put_message(message)
                .await
                .expect("Failed to insert message");
        }

        for i in 0..2 {
            let message = VoicemailMessage {
                id: format!("msg-saved-{}", i),
                mailbox_id: mailbox_id.to_string(),
                caller: "+15559876543".to_string(),
                caller_name: None,
                status: MessageStatus::Saved,
                priority: MessagePriority::Normal,
                duration: 45,
                recording_path: std::path::PathBuf::from(format!("/var/spool/voicemail/msg-saved-{}.wav", i)),
                transcription: None,
                created_at: SystemTime::now(),
                accessed_at: Some(SystemTime::now()),
                // metadata: std::collections::HashMap::new(),
            };
            storage
                .put_message(message)
                .await
                .expect("Failed to insert message");
        }

        let stats = storage
            .get_mailbox_stats(mailbox_id)
            .await
            .expect("Failed to get stats");

        assert_eq!(stats.total_messages, 5);
        assert_eq!(stats.new_messages, 3);
        assert_eq!(stats.saved_messages, 2);
        assert_eq!(stats.heard_messages, 2);
        // Average duration: (30 + 40 + 50 + 45 + 45) / 5 = 42
        assert_eq!(stats.avg_duration, 42);
    }
}

#[cfg(test)]
mod porting_store_tests {
    use super::*;

    #[tokio::test]
    async fn test_port_request_crud() {
        let storage = create_test_storage().await;
        let tenant_id = test_tenant_id();

        let address = Address {
            street1: "123 Main St".to_string(),
            street2: None,
            city: "San Francisco".to_string(),
            state: "CA".to_string(),
            postal_code: "94105".to_string(),
            country: "US".to_string(),
        };

        let request = PortRequest {
            id: PortRequestId::new(),
            tenant_id: tenant_id.clone(),
            phone_numbers: vec!["+14155551234".to_string()],
            port_type: PortType::Single,
            status: PortStatus::Draft,
            customer_name: "Acme Corp".to_string(),
            account_number: "ACC-12345".to_string(),
            pin: None,
            service_address: address.clone(),
            // billing_telephone_number: None,
            authorized_contact_name: String::new(),
            authorized_contact_phone: String::new(),
            authorized_contact_email: String::new(),
            losing_carrier: "OldCarrier".to_string(),
            losing_carrier_spid: None,
            winning_carrier: "NewCarrier".to_string(),
            winning_carrier_spid: None,
            requested_foc_date: None,
            actual_foc_date: None,
            submitted_at: None,
            completed_at: None,
            rejection_reason: None,
            notes: None,
            internal_notes: None,
            metadata: std::collections::HashMap::new(),
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
        };

        // Insert port request
        storage
            .put_port_request(request.clone())
            .await
            .expect("Failed to insert port request");

        // Get port request
        let loaded = storage
            .get_port_request(&request.id)
            .await
            .expect("Failed to get port request")
            .expect("Port request not found");

        assert_eq!(loaded.id, request.id);
        assert_eq!(loaded.status, PortStatus::Draft);
        assert_eq!(loaded.customer_name, request.customer_name);

        // List port requests
        let requests = storage
            .list_port_requests(&tenant_id)
            .await
            .expect("Failed to list port requests");
        assert_eq!(requests.len(), 1);

        // List by status
        let draft_requests = storage
            .list_port_requests_by_status(&tenant_id, PortStatus::Draft)
            .await
            .expect("Failed to list by status");
        assert_eq!(draft_requests.len(), 1);

        // Delete port request
        storage
            .delete_port_request(&request.id)
            .await
            .expect("Failed to delete port request");

        let deleted = storage
            .get_port_request(&request.id)
            .await
            .expect("Failed to get deleted port request");
        assert!(deleted.is_none());
    }
}
