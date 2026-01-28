//! Multi-tenant isolation tests.
//!
//! These tests verify that tenant isolation works correctly:
//! - Users from different tenants cannot access each other's policies
//! - Policy evaluation is tenant-specific
//!
//! Note: For MVP, get_user_tenant() only searches the default tenant.
//! Multi-tenant user lookup across all tenants is a future enhancement.

use fcp_core::TenantId;
use fcp_routing::{RoutingService, YamlRoutingStore};
use std::io::Write;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::Arc;
use tempfile::NamedTempFile;

#[tokio::test]
async fn test_multi_tenant_isolation() {
    // Create two different tenant IDs
    let tenant_a = TenantId::from_str("aaaaaaaa-0000-0000-0000-000000000000").unwrap();
    let tenant_b = TenantId::from_str("bbbbbbbb-0000-0000-0000-000000000000").unwrap();

    // Create temporary YAML file with users and policies for both tenants
    let mut file = NamedTempFile::new().unwrap();
    writeln!(
        file,
        r#"
users:
  # Tenant A users
  - id: "alice@tenant-a.com"
    tenant_id: "{}"
    display_name: "Alice (Tenant A)"
    roles: ["employee"]
    groups: []
    endpoints: []
    enabled: true

  # Tenant B users
  - id: "bob@tenant-b.com"
    tenant_id: "{}"
    display_name: "Bob (Tenant B)"
    roles: ["employee"]
    groups: []
    endpoints: []
    enabled: true

policies:
  # Tenant A policy - allow internal calls
  - name: "tenant-a-allow-internal"
    tenant_id: "{}"
    priority: 100
    enabled: true
    applies_to:
      roles: ["employee"]
    destination_match:
      pattern: "^1[0-9]{{3}}$"  # 1000-1999
    time_windows: []
    source_ip_ranges: []
    action: Allow
    reason: "Tenant A employees can call 1xxx extensions"

  # Tenant B policy - allow different internal range
  - name: "tenant-b-allow-internal"
    tenant_id: "{}"
    priority: 100
    enabled: true
    applies_to:
      roles: ["employee"]
    destination_match:
      pattern: "^2[0-9]{{3}}$"  # 2000-2999
    time_windows: []
    source_ip_ranges: []
    action: Allow
    reason: "Tenant B employees can call 2xxx extensions"
"#,
        tenant_a.as_uuid(),
        tenant_b.as_uuid(),
        tenant_a.as_uuid(),
        tenant_b.as_uuid()
    )
    .unwrap();

    let path = file.path().to_path_buf();
    let store = YamlRoutingStore::new(path, false).await.unwrap();
    let service = Arc::new(RoutingService::new(Arc::new(store)));

    let source_ip: IpAddr = "10.0.1.100".parse().unwrap();

    // Test 1: Tenant A user calling 1xxx (allowed by Tenant A policy)
    let decision = service
        .evaluate_call("alice@tenant-a.com", "1234", &tenant_a, source_ip)
        .await
        .unwrap();
    assert!(decision.action.is_allow());
    assert_eq!(
        decision.matched_rule,
        Some("tenant-a-allow-internal".to_string())
    );

    // Test 2: Tenant A user calling 2xxx (denied - Tenant B's range)
    let decision = service
        .evaluate_call("alice@tenant-a.com", "2234", &tenant_a, source_ip)
        .await
        .unwrap();
    assert!(decision.action.is_deny());
    assert_eq!(decision.matched_rule, None); // No rule matched (default deny)

    // Test 3: Tenant B user calling 2xxx (allowed by Tenant B policy)
    let decision = service
        .evaluate_call("bob@tenant-b.com", "2234", &tenant_b, source_ip)
        .await
        .unwrap();
    assert!(decision.action.is_allow());
    assert_eq!(
        decision.matched_rule,
        Some("tenant-b-allow-internal".to_string())
    );

    // Test 4: Tenant B user calling 1xxx (denied - Tenant A's range)
    let decision = service
        .evaluate_call("bob@tenant-b.com", "1234", &tenant_b, source_ip)
        .await
        .unwrap();
    assert!(decision.action.is_deny());
    assert_eq!(decision.matched_rule, None);
}

#[tokio::test]
async fn test_cross_tenant_user_lookup() {
    // Create two different tenant IDs
    let tenant_a = TenantId::from_str("aaaaaaaa-0000-0000-0000-000000000000").unwrap();
    let tenant_b = TenantId::from_str("bbbbbbbb-0000-0000-0000-000000000000").unwrap();

    // Create temporary YAML file
    let mut file = NamedTempFile::new().unwrap();
    writeln!(
        file,
        r#"
users:
  - id: "alice@tenant-a.com"
    tenant_id: "{}"
    display_name: "Alice (Tenant A)"
    roles: ["employee"]
    groups: []
    endpoints: []
    enabled: true

  - id: "bob@tenant-b.com"
    tenant_id: "{}"
    display_name: "Bob (Tenant B)"
    roles: ["employee"]
    groups: []
    endpoints: []
    enabled: true

policies: []
"#,
        tenant_a.as_uuid(),
        tenant_b.as_uuid()
    )
    .unwrap();

    let path = file.path().to_path_buf();
    let store = YamlRoutingStore::new(path, false).await.unwrap();
    let service = Arc::new(RoutingService::new(Arc::new(store)));

    let source_ip: IpAddr = "10.0.1.100".parse().unwrap();

    // Test 1: Calling as Alice with wrong tenant ID
    // Since user is not found in tenant_b's routing store, a minimal identity context
    // is created (to support all_users policies). With no matching policies, this
    // results in a deny decision (default deny).
    let result = service
        .evaluate_call(
            "alice@tenant-a.com",
            "1234",
            &tenant_b, // Wrong tenant!
            source_ip,
        )
        .await;
    assert!(result.is_ok());
    let decision = result.unwrap();
    // Should be denied because no policies match for tenant_b
    assert!(decision.action.is_deny());
    assert_eq!(decision.reason, "No policy rule matched (default deny)");

    // Test 2: Calling as Alice with correct tenant ID should work
    let result = service
        .evaluate_call(
            "alice@tenant-a.com",
            "1234",
            &tenant_a, // Correct tenant
            source_ip,
        )
        .await;
    assert!(result.is_ok());
    // Will be denied (no policies), but user was found
    let decision = result.unwrap();
    assert!(decision.action.is_deny());
    assert_eq!(decision.reason, "No policy rule matched (default deny)");
}

#[tokio::test]
async fn test_get_user_tenant() {
    // For MVP, all users are in the default tenant
    let default_tenant = TenantId::nil();

    // Create temporary YAML file with users in default tenant
    let mut file = NamedTempFile::new().unwrap();
    writeln!(
        file,
        r#"
users:
  - id: "alice@company.com"
    tenant_id: "{}"
    display_name: "Alice"
    roles: ["employee"]
    groups: []
    endpoints: []
    enabled: true

  - id: "bob@company.com"
    tenant_id: "{}"
    display_name: "Bob"
    roles: ["employee"]
    groups: []
    endpoints: []
    enabled: true

policies: []
"#,
        default_tenant.as_uuid(),
        default_tenant.as_uuid()
    )
    .unwrap();

    let path = file.path().to_path_buf();
    let store = YamlRoutingStore::new(path, false).await.unwrap();
    let service = Arc::new(RoutingService::new(Arc::new(store)));

    // Test: get_user_tenant returns correct tenant (default tenant)
    let alice_tenant = service.get_user_tenant("alice@company.com").await.unwrap();
    assert_eq!(alice_tenant, default_tenant);

    let bob_tenant = service.get_user_tenant("bob@company.com").await.unwrap();
    assert_eq!(bob_tenant, default_tenant);

    // Test: get_user_tenant fails for non-existent user
    let result = service.get_user_tenant("nobody@nowhere.com").await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("User"));
}
