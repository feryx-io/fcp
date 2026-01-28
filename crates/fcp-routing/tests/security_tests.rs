//! Security-focused tests for fcp-routing
//!
//! These tests verify that security validations are working correctly
//! across policy rules, identity management, and routing decisions.

use chrono::{NaiveTime, Weekday};
use fcp_core::TenantId;
use fcp_routing::models::{
    AppliesTo, DestinationMatch, DeviceType, Endpoint, IpRange, PolicyAction, PolicyRule,
    TimeWindow, User,
};
use std::collections::HashSet;

// ============================================================================
// Helper Functions
// ============================================================================

fn create_basic_user(id: &str) -> User {
    User {
        id: id.to_string(),
        tenant_id: TenantId::new(),
        display_name: None,
        roles: HashSet::new(),
        groups: HashSet::new(),
        endpoints: vec![],
        enabled: true,
    }
}

fn create_basic_endpoint(uri: &str) -> Endpoint {
    Endpoint {
        uri: uri.to_string(),
        device_type: DeviceType::Softphone,
        location: None,
    }
}

// ============================================================================
// User Identity Tests
// ============================================================================

#[test]
fn test_user_crlf_injection_in_id() {
    let user = User {
        id: "user\r\ninjection".to_string(),
        tenant_id: TenantId::new(),
        display_name: None,
        roles: HashSet::new(),
        groups: HashSet::new(),
        endpoints: vec![],
        enabled: true,
    };
    assert!(user.validate().is_err(), "Should reject CRLF in user ID");
}

#[test]
fn test_user_crlf_injection_in_display_name() {
    let user = User {
        id: "user123".to_string(),
        tenant_id: TenantId::new(),
        display_name: Some("Alice\nInjection".to_string()),
        roles: HashSet::new(),
        groups: HashSet::new(),
        enabled: true,
        endpoints: vec![],
    };
    assert!(
        user.validate().is_err(),
        "Should reject CRLF in display name"
    );
}

#[test]
fn test_user_crlf_injection_in_roles() {
    let user = User {
        id: "user123".to_string(),
        tenant_id: TenantId::new(),
        display_name: None,
        roles: HashSet::from(["admin\r\nevil".to_string()]),
        groups: HashSet::new(),
        enabled: true,
        endpoints: vec![],
    };
    assert!(user.validate().is_err(), "Should reject CRLF in roles");
}

#[test]
fn test_user_crlf_injection_in_groups() {
    let user = User {
        id: "user123".to_string(),
        tenant_id: TenantId::new(),
        display_name: None,
        roles: HashSet::new(),
        groups: HashSet::from(["sales\ninjection".to_string()]),
        enabled: true,
        endpoints: vec![],
    };
    assert!(user.validate().is_err(), "Should reject CRLF in groups");
}

#[test]
fn test_user_empty_id() {
    let user = User {
        id: "".to_string(),
        tenant_id: TenantId::new(),
        display_name: None,
        roles: HashSet::new(),
        groups: HashSet::new(),
        enabled: true,
        endpoints: vec![],
    };
    assert!(user.validate().is_err(), "Should reject empty user ID");
}

#[test]
fn test_valid_user() {
    let user = User {
        id: "alice@company.com".to_string(),
        tenant_id: TenantId::new(),
        display_name: Some("Alice Johnson".to_string()),
        roles: HashSet::from(["user".to_string(), "admin".to_string()]),
        groups: HashSet::from(["engineering".to_string()]),
        enabled: true,
        endpoints: vec![],
    };
    assert!(user.validate().is_ok(), "Should accept valid user");
}

// ============================================================================
// Endpoint Tests
// ============================================================================

#[test]
fn test_endpoint_crlf_injection_in_uri() {
    let endpoint = Endpoint {
        uri: "sip:user@example.com\r\ninjection".to_string(),
        location: None,
        device_type: DeviceType::Softphone,
    };
    assert!(
        endpoint.validate().is_err(),
        "Should reject CRLF in endpoint URI"
    );
}

#[test]
fn test_endpoint_crlf_injection_in_location() {
    let endpoint = Endpoint {
        uri: "sip:user@example.com".to_string(),
        location: Some("office\ninjection".to_string()),
        device_type: DeviceType::Softphone,
    };
    assert!(
        endpoint.validate().is_err(),
        "Should reject CRLF in endpoint location"
    );
}

#[test]
fn test_endpoint_empty_uri() {
    let endpoint = Endpoint {
        uri: "".to_string(),
        location: None,
        device_type: DeviceType::Softphone,
    };
    assert!(
        endpoint.validate().is_err(),
        "Should reject empty endpoint URI"
    );
}

#[test]
fn test_endpoint_invalid_scheme() {
    let endpoint = Endpoint {
        uri: "http://example.com".to_string(),
        location: None,
        device_type: DeviceType::Softphone,
    };
    assert!(
        endpoint.validate().is_err(),
        "Should reject non-SIP URI scheme"
    );
}

#[test]
fn test_endpoint_valid_sip_uri() {
    let endpoint = Endpoint {
        uri: "sip:alice@example.com".to_string(),
        location: Some("office".to_string()),
        device_type: DeviceType::Softphone,
    };
    assert!(endpoint.validate().is_ok(), "Should accept valid SIP URI");
}

#[test]
fn test_endpoint_valid_sips_uri() {
    let endpoint = Endpoint {
        uri: "sips:secure@example.com".to_string(),
        location: None,
        device_type: DeviceType::Softphone,
    };
    assert!(endpoint.validate().is_ok(), "Should accept valid SIPS URI");
}

#[test]
fn test_endpoint_case_insensitive_scheme() {
    let endpoint = Endpoint {
        uri: "SIP:user@example.com".to_string(),
        location: None,
        device_type: DeviceType::Softphone,
    };
    assert!(
        endpoint.validate().is_ok(),
        "Should accept uppercase SIP scheme"
    );
}

// ============================================================================
// Policy Rule Tests
// ============================================================================

#[test]
fn test_policy_crlf_injection_in_name() {
    let rule = PolicyRule {
        name: "rule\ninjection".to_string(),
        tenant_id: TenantId::new(),
        priority: 100,
        enabled: true,
        applies_to: AppliesTo {
            all_users: false,
            users: None,
            roles: None,
            groups: None,
        },
        destination_match: None,
        time_windows: vec![],
        source_ip_ranges: vec![],
        action: PolicyAction::Allow,
        reason: None,
    };
    assert!(
        rule.validate().is_err(),
        "Should reject CRLF in policy name"
    );
}

#[test]
fn test_policy_crlf_injection_in_reason() {
    let rule = PolicyRule {
        name: "test-rule".to_string(),
        tenant_id: TenantId::new(),
        priority: 100,
        enabled: true,
        applies_to: AppliesTo {
            all_users: false,
            users: None,
            roles: None,
            groups: None,
        },
        destination_match: None,
        time_windows: vec![],
        source_ip_ranges: vec![],
        action: PolicyAction::Deny,
        reason: Some("blocked\r\ninjection".to_string()),
    };
    assert!(
        rule.validate().is_err(),
        "Should reject CRLF in policy reason"
    );
}

#[test]
fn test_policy_empty_name() {
    let rule = PolicyRule {
        name: "".to_string(),
        tenant_id: TenantId::new(),
        priority: 100,
        enabled: true,
        applies_to: AppliesTo {
            all_users: false,
            users: None,
            roles: None,
            groups: None,
        },
        destination_match: None,
        time_windows: vec![],
        source_ip_ranges: vec![],
        action: PolicyAction::Allow,
        reason: None,
    };
    assert!(rule.validate().is_err(), "Should reject empty policy name");
}

#[test]
fn test_policy_negative_priority() {
    let rule = PolicyRule {
        name: "test-rule".to_string(),
        tenant_id: TenantId::new(),
        priority: -1,
        enabled: true,
        applies_to: AppliesTo {
            all_users: false,
            users: None,
            roles: None,
            groups: None,
        },
        destination_match: None,
        time_windows: vec![],
        source_ip_ranges: vec![],
        action: PolicyAction::Allow,
        reason: None,
    };
    assert!(rule.validate().is_err(), "Should reject negative priority");
}

// ============================================================================
// AppliesTo Tests
// ============================================================================

#[test]
fn test_applies_to_crlf_in_users() {
    let applies_to = AppliesTo {
        all_users: false,
        users: Some(vec!["alice\ninjection".to_string()]),
        roles: None,
        groups: None,
    };
    assert!(
        applies_to.validate().is_err(),
        "Should reject CRLF in applies_to users"
    );
}

#[test]
fn test_applies_to_crlf_in_roles() {
    let applies_to = AppliesTo {
        all_users: false,
        users: None,
        roles: Some(vec!["admin\r\nevil".to_string()]),
        groups: None,
    };
    assert!(
        applies_to.validate().is_err(),
        "Should reject CRLF in applies_to roles"
    );
}

#[test]
fn test_applies_to_crlf_in_groups() {
    let applies_to = AppliesTo {
        all_users: false,
        users: None,
        roles: None,
        groups: Some(vec!["sales\ninjection".to_string()]),
    };
    assert!(
        applies_to.validate().is_err(),
        "Should reject CRLF in applies_to groups"
    );
}

// ============================================================================
// DestinationMatch Tests (ReDoS Prevention)
// ============================================================================

#[test]
fn test_destination_match_crlf_in_pattern() {
    let dest_match = DestinationMatch {
        pattern: "^\\d{4}$\ninjection".to_string(),
        tags: None,
    };
    assert!(
        dest_match.validate().is_err(),
        "Should reject CRLF in destination pattern"
    );
}

#[test]
fn test_destination_match_crlf_in_tags() {
    let dest_match = DestinationMatch {
        pattern: "^\\d{4}$".to_string(),
        tags: Some(vec!["internal\ninjection".to_string()]),
    };
    assert!(
        dest_match.validate().is_err(),
        "Should reject CRLF in destination tags"
    );
}

#[test]
fn test_destination_match_empty_pattern() {
    let dest_match = DestinationMatch {
        pattern: "".to_string(),
        tags: None,
    };
    assert!(
        dest_match.validate().is_err(),
        "Should reject empty destination pattern"
    );
}

#[test]
fn test_destination_match_invalid_regex() {
    let dest_match = DestinationMatch {
        pattern: "[invalid(".to_string(), // Invalid regex syntax
        tags: None,
    };
    assert!(
        dest_match.validate().is_err(),
        "Should reject invalid regex pattern"
    );
}

#[test]
fn test_destination_match_valid_regex() {
    let dest_match = DestinationMatch {
        pattern: "^[2-5]\\d{3}$".to_string(), // Match 2000-5999
        tags: Some(vec!["internal".to_string(), "free".to_string()]),
    };
    assert!(
        dest_match.validate().is_ok(),
        "Should accept valid regex pattern"
    );
}

#[test]
fn test_destination_match_catastrophic_backtracking() {
    // This regex is valid but would be very slow - policy decision
    let dest_match = DestinationMatch {
        pattern: "(a+)+$".to_string(),
        tags: None,
    };
    // The regex compiles successfully, demonstrating we catch compilation errors
    // but don't prevent all potentially slow patterns (that's a policy decision)
    assert!(
        dest_match.validate().is_ok(),
        "Regex compiles but may be slow - policy decision"
    );
}

// ============================================================================
// TimeWindow Tests
// ============================================================================

#[test]
fn test_time_window_crlf_in_timezone() {
    let window = TimeWindow {
        days: vec![Weekday::Mon],
        start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        timezone: "UTC\ninjection".to_string(),
    };
    assert!(window.validate().is_err(), "Should reject CRLF in timezone");
}

#[test]
fn test_time_window_empty_days() {
    let window = TimeWindow {
        days: vec![],
        start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        timezone: "UTC".to_string(),
    };
    assert!(window.validate().is_err(), "Should reject empty days list");
}

#[test]
fn test_time_window_start_after_end() {
    let window = TimeWindow {
        days: vec![Weekday::Mon],
        start_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(), // End before start
        timezone: "UTC".to_string(),
    };
    assert!(
        window.validate().is_err(),
        "Should reject start_time >= end_time"
    );
}

#[test]
fn test_time_window_invalid_timezone() {
    let window = TimeWindow {
        days: vec![Weekday::Mon],
        start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        timezone: "InvalidTimezone".to_string(),
    };
    assert!(window.validate().is_err(), "Should reject invalid timezone");
}

#[test]
fn test_time_window_valid() {
    let window = TimeWindow {
        days: vec![Weekday::Mon, Weekday::Tue, Weekday::Wed],
        start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        timezone: "America/New_York".to_string(),
    };
    assert!(window.validate().is_ok(), "Should accept valid time window");
}

#[test]
fn test_time_window_utc_timezone() {
    let window = TimeWindow {
        days: vec![Weekday::Mon],
        start_time: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        end_time: NaiveTime::from_hms_opt(23, 59, 59).unwrap(),
        timezone: "UTC".to_string(),
    };
    assert!(window.validate().is_ok(), "Should accept UTC timezone");
}

// ============================================================================
// IpRange Tests (CIDR Validation)
// ============================================================================

#[test]
fn test_ip_range_crlf_in_cidr() {
    let range = IpRange {
        cidr: "192.168.1.0/24\ninjection".to_string(),
    };
    assert!(range.validate().is_err(), "Should reject CRLF in CIDR");
}

#[test]
fn test_ip_range_empty_cidr() {
    let range = IpRange {
        cidr: "".to_string(),
    };
    assert!(range.validate().is_err(), "Should reject empty CIDR");
}

#[test]
fn test_ip_range_invalid_cidr_format() {
    let range = IpRange {
        cidr: "not.a.cidr".to_string(),
    };
    assert!(
        range.validate().is_err(),
        "Should reject invalid CIDR format"
    );
}

#[test]
fn test_ip_range_invalid_cidr_prefix() {
    let range = IpRange {
        cidr: "192.168.1.0/33".to_string(), // Invalid prefix (max 32 for IPv4)
    };
    assert!(
        range.validate().is_err(),
        "Should reject invalid CIDR prefix"
    );
}

#[test]
fn test_ip_range_valid_ipv4() {
    let range = IpRange {
        cidr: "192.168.1.0/24".to_string(),
    };
    assert!(range.validate().is_ok(), "Should accept valid IPv4 CIDR");
}

#[test]
fn test_ip_range_valid_ipv6() {
    let range = IpRange {
        cidr: "2001:db8::/32".to_string(),
    };
    assert!(range.validate().is_ok(), "Should accept valid IPv6 CIDR");
}

#[test]
fn test_ip_range_single_host_ipv4() {
    let range = IpRange {
        cidr: "192.168.1.1/32".to_string(), // Single host
    };
    assert!(range.validate().is_ok(), "Should accept single host IPv4");
}

#[test]
fn test_ip_range_entire_internet_ipv4() {
    let range = IpRange {
        cidr: "0.0.0.0/0".to_string(), // Entire IPv4 space
    };
    assert!(range.validate().is_ok(), "Should accept 0.0.0.0/0 CIDR");
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_user_with_invalid_endpoint() {
    let user = User {
        id: "alice@company.com".to_string(),
        tenant_id: TenantId::new(),
        display_name: None,
        roles: HashSet::new(),
        groups: HashSet::new(),
        enabled: true,
        endpoints: vec![Endpoint {
            uri: "http://invalid.com".to_string(), // Wrong scheme
            location: None,
            device_type: DeviceType::Softphone,
        }],
    };
    assert!(
        user.validate().is_err(),
        "Should reject user with invalid endpoint"
    );
}

#[test]
fn test_policy_with_invalid_destination_match() {
    let rule = PolicyRule {
        name: "test-rule".to_string(),
        tenant_id: TenantId::new(),
        priority: 100,
        enabled: true,
        applies_to: AppliesTo {
            all_users: false,
            users: None,
            roles: None,
            groups: None,
        },
        destination_match: Some(DestinationMatch {
            pattern: "[invalid(".to_string(), // Invalid regex
            tags: None,
        }),
        time_windows: vec![],
        source_ip_ranges: vec![],
        action: PolicyAction::Allow,
        reason: None,
    };
    assert!(
        rule.validate().is_err(),
        "Should reject policy with invalid regex"
    );
}

#[test]
fn test_policy_with_invalid_time_window() {
    let rule = PolicyRule {
        name: "test-rule".to_string(),
        tenant_id: TenantId::new(),
        priority: 100,
        enabled: true,
        applies_to: AppliesTo {
            all_users: false,
            users: None,
            roles: None,
            groups: None,
        },
        destination_match: None,
        time_windows: vec![TimeWindow {
            days: vec![], // Empty days
            start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            timezone: "UTC".to_string(),
        }],
        source_ip_ranges: vec![],
        action: PolicyAction::Allow,
        reason: None,
    };
    assert!(
        rule.validate().is_err(),
        "Should reject policy with invalid time window"
    );
}

#[test]
fn test_policy_with_invalid_ip_range() {
    let rule = PolicyRule {
        name: "test-rule".to_string(),
        tenant_id: TenantId::new(),
        priority: 100,
        enabled: true,
        applies_to: AppliesTo {
            all_users: false,
            users: None,
            roles: None,
            groups: None,
        },
        destination_match: None,
        time_windows: vec![],
        source_ip_ranges: vec![IpRange {
            cidr: "invalid".to_string(),
        }],
        action: PolicyAction::Allow,
        reason: None,
    };
    assert!(
        rule.validate().is_err(),
        "Should reject policy with invalid CIDR"
    );
}

#[test]
fn test_fully_valid_policy() {
    let rule = PolicyRule {
        name: "business-hours-allow".to_string(),
        tenant_id: TenantId::new(),
        priority: 100,
        enabled: true,
        applies_to: AppliesTo {
            all_users: false,
            users: Some(vec!["alice@company.com".to_string()]),
            roles: Some(vec!["employee".to_string()]),
            groups: Some(vec!["sales".to_string()]),
        },
        destination_match: Some(DestinationMatch {
            pattern: "^[2-5]\\d{3}$".to_string(),
            tags: Some(vec!["internal".to_string()]),
        }),
        time_windows: vec![TimeWindow {
            days: vec![
                Weekday::Mon,
                Weekday::Tue,
                Weekday::Wed,
                Weekday::Thu,
                Weekday::Fri,
            ],
            start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            timezone: "America/New_York".to_string(),
        }],
        source_ip_ranges: vec![IpRange {
            cidr: "10.0.0.0/8".to_string(),
        }],
        action: PolicyAction::Allow,
        reason: Some("Allow internal calls during business hours".to_string()),
    };
    assert!(rule.validate().is_ok(), "Should accept fully valid policy");
}
