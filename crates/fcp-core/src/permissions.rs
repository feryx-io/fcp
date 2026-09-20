//! User roles and permissions system

use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Deployment mode determines feature availability and UI complexity
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum DeploymentMode {
    /// Carrier/ITSP - Full technical access, routing, trunking
    Carrier,
    /// Multi-tenant SaaS platform (CPaaS/UCaaS)
    ServiceProvider,
    /// Single organization PBX (default)
    #[default]
    Enterprise,
}

impl DeploymentMode {
    /// Get human-readable name
    pub fn name(&self) -> &'static str {
        match self {
            Self::Carrier => "Carrier/ITSP",
            Self::ServiceProvider => "Service Provider",
            Self::Enterprise => "Enterprise PBX",
        }
    }

    /// Check if multi-tenancy is enabled
    pub fn is_multi_tenant(&self) -> bool {
        matches!(self, Self::ServiceProvider)
    }

    /// Check if carrier features are enabled
    pub fn has_carrier_features(&self) -> bool {
        matches!(self, Self::Carrier | Self::ServiceProvider)
    }

    /// Check if trunk management is enabled
    pub fn has_trunk_management(&self) -> bool {
        matches!(self, Self::Carrier | Self::ServiceProvider)
    }

    /// Check if advanced routing is enabled
    pub fn has_advanced_routing(&self) -> bool {
        matches!(self, Self::Carrier | Self::ServiceProvider)
    }
}

/// User role defines the level of access
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum UserRole {
    /// Platform super admin - full system access across every tenant.
    SuperAdmin,
    /// Tenant system admin - full administrative access within a single
    /// tenant. Cannot edit platform/system configuration, cannot create or
    /// delete tenants, and cannot see resources belonging to other tenants.
    TenantAdmin,
    /// Telecom/voice infrastructure admin
    TelecomAdmin,
    /// User and extension management
    UserAdmin,
    /// Department/location manager - limited scope
    DepartmentAdmin,
    /// Read-only access for monitoring/reporting
    ReadOnly,
    /// Regular end user - personal settings only
    #[default]
    EndUser,
}

impl FromStr for UserRole {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "super_admin" => Ok(Self::SuperAdmin),
            "tenant_admin" => Ok(Self::TenantAdmin),
            "telecom_admin" => Ok(Self::TelecomAdmin),
            "user_admin" => Ok(Self::UserAdmin),
            "department_admin" => Ok(Self::DepartmentAdmin),
            "read_only" => Ok(Self::ReadOnly),
            "end_user" => Ok(Self::EndUser),
            other => Err(format!("unknown role: {}", other)),
        }
    }
}

impl UserRole {
    /// Get human-readable name
    pub fn name(&self) -> &'static str {
        match self {
            Self::SuperAdmin => "Super Administrator",
            Self::TenantAdmin => "Tenant Administrator",
            Self::TelecomAdmin => "Telecom Administrator",
            Self::UserAdmin => "User Administrator",
            Self::DepartmentAdmin => "Department Administrator",
            Self::ReadOnly => "Read-Only User",
            Self::EndUser => "End User",
        }
    }

    /// Get description of role
    pub fn description(&self) -> &'static str {
        match self {
            Self::SuperAdmin => "Full system access including configuration and all resources",
            Self::TenantAdmin => {
                "Full administrative access within a single tenant; cannot edit system settings or manage other tenants"
            }
            Self::TelecomAdmin => {
                "Manage voice infrastructure: phone numbers, devices, trunks, routing"
            }
            Self::UserAdmin => "Manage users, passwords, and basic settings",
            Self::DepartmentAdmin => "Manage users and resources within assigned department",
            Self::ReadOnly => "View-only access for monitoring and reporting",
            Self::EndUser => "Personal settings and assigned resources only",
        }
    }

    /// True when this role has cross-tenant visibility on the platform.
    ///
    /// Only [`UserRole::SuperAdmin`] is a platform-level role. TenantAdmin and
    /// all other roles are scoped to a single tenant and the API layer must
    /// enforce that scope when serving requests from a session with one of
    /// those roles.
    pub fn is_platform_admin(&self) -> bool {
        matches!(self, Self::SuperAdmin)
    }
}

/// Granular permissions for specific actions
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    // System
    SystemConfig,
    SystemViewLogs,

    // Users
    UserCreate,
    UserRead,
    UserUpdate,
    UserDelete,
    UserSetPassword,

    // Phone Numbers
    PhoneNumberCreate,
    PhoneNumberRead,
    PhoneNumberUpdate,
    PhoneNumberDelete,
    PhoneNumberViewCarrierData,

    // Devices
    DeviceCreate,
    DeviceRead,
    DeviceUpdate,
    DeviceDelete,

    // Calls
    CallView,
    CallControl,
    CallRecordingAccess,

    // Registrations
    RegistrationView,
    RegistrationManage,

    // Trunks (Carrier/Service Provider only)
    TrunkCreate,
    TrunkRead,
    TrunkUpdate,
    TrunkDelete,

    // Routing rules (`docs/TENANT_TRUNKS.md` §12, phase 6d): the tenant's
    // own call policy. A rule that routes to a trunk also takes
    // `TrunkUpdate`, since it names a carrier.
    RoutingRuleRead,
    RoutingRuleManage,

    // Tenants (Service Provider only)
    TenantCreate,
    TenantRead,
    TenantUpdate,
    TenantDelete,

    // Debug
    DebugView,

    // Compliance (docs/COMPLIANCE_FOLLOWUPS.md §4)
    /// The retention and consent policy, and retention runs.
    ComplianceManage,
    /// Placing and releasing legal holds.
    LegalHoldManage,
    /// Subject export and erasure requests.
    SubjectRequestManage,
    /// The admin audit log and the deletion log.
    AuditView,
}

impl FromStr for Permission {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "system_config" => Ok(Self::SystemConfig),
            "system_view_logs" => Ok(Self::SystemViewLogs),
            "user_create" => Ok(Self::UserCreate),
            "user_read" => Ok(Self::UserRead),
            "user_update" => Ok(Self::UserUpdate),
            "user_delete" => Ok(Self::UserDelete),
            "user_set_password" => Ok(Self::UserSetPassword),
            "phone_number_create" => Ok(Self::PhoneNumberCreate),
            "phone_number_read" => Ok(Self::PhoneNumberRead),
            "phone_number_update" => Ok(Self::PhoneNumberUpdate),
            "phone_number_delete" => Ok(Self::PhoneNumberDelete),
            "phone_number_view_carrier_data" => Ok(Self::PhoneNumberViewCarrierData),
            "device_create" => Ok(Self::DeviceCreate),
            "device_read" => Ok(Self::DeviceRead),
            "device_update" => Ok(Self::DeviceUpdate),
            "device_delete" => Ok(Self::DeviceDelete),
            "call_view" => Ok(Self::CallView),
            "call_control" => Ok(Self::CallControl),
            "call_recording_access" => Ok(Self::CallRecordingAccess),
            "registration_view" => Ok(Self::RegistrationView),
            "registration_manage" => Ok(Self::RegistrationManage),
            "trunk_create" => Ok(Self::TrunkCreate),
            "trunk_read" => Ok(Self::TrunkRead),
            "trunk_update" => Ok(Self::TrunkUpdate),
            "trunk_delete" => Ok(Self::TrunkDelete),
            "routing_rule_read" => Ok(Self::RoutingRuleRead),
            "routing_rule_manage" => Ok(Self::RoutingRuleManage),
            "tenant_create" => Ok(Self::TenantCreate),
            "tenant_read" => Ok(Self::TenantRead),
            "tenant_update" => Ok(Self::TenantUpdate),
            "tenant_delete" => Ok(Self::TenantDelete),
            "debug_view" => Ok(Self::DebugView),
            "compliance_manage" => Ok(Self::ComplianceManage),
            "legal_hold_manage" => Ok(Self::LegalHoldManage),
            "subject_request_manage" => Ok(Self::SubjectRequestManage),
            "audit_view" => Ok(Self::AuditView),
            other => Err(format!("unknown permission: {}", other)),
        }
    }
}

impl Permission {
    /// Get the snake_case string representation used in API token permission arrays.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SystemConfig => "system_config",
            Self::SystemViewLogs => "system_view_logs",
            Self::UserCreate => "user_create",
            Self::UserRead => "user_read",
            Self::UserUpdate => "user_update",
            Self::UserDelete => "user_delete",
            Self::UserSetPassword => "user_set_password",
            Self::PhoneNumberCreate => "phone_number_create",
            Self::PhoneNumberRead => "phone_number_read",
            Self::PhoneNumberUpdate => "phone_number_update",
            Self::PhoneNumberDelete => "phone_number_delete",
            Self::PhoneNumberViewCarrierData => "phone_number_view_carrier_data",
            Self::DeviceCreate => "device_create",
            Self::DeviceRead => "device_read",
            Self::DeviceUpdate => "device_update",
            Self::DeviceDelete => "device_delete",
            Self::CallView => "call_view",
            Self::CallControl => "call_control",
            Self::CallRecordingAccess => "call_recording_access",
            Self::RegistrationView => "registration_view",
            Self::RegistrationManage => "registration_manage",
            Self::TrunkCreate => "trunk_create",
            Self::TrunkRead => "trunk_read",
            Self::TrunkUpdate => "trunk_update",
            Self::TrunkDelete => "trunk_delete",
            Self::RoutingRuleRead => "routing_rule_read",
            Self::RoutingRuleManage => "routing_rule_manage",
            Self::TenantCreate => "tenant_create",
            Self::TenantRead => "tenant_read",
            Self::TenantUpdate => "tenant_update",
            Self::TenantDelete => "tenant_delete",
            Self::DebugView => "debug_view",
            Self::ComplianceManage => "compliance_manage",
            Self::LegalHoldManage => "legal_hold_manage",
            Self::SubjectRequestManage => "subject_request_manage",
            Self::AuditView => "audit_view",
        }
    }

    /// Get human-readable name
    pub fn name(&self) -> &'static str {
        match self {
            Self::SystemConfig => "System Configuration",
            Self::SystemViewLogs => "View System Logs",
            Self::UserCreate => "Create Users",
            Self::UserRead => "View Users",
            Self::UserUpdate => "Update Users",
            Self::UserDelete => "Delete Users",
            Self::UserSetPassword => "Set User Passwords",
            Self::PhoneNumberCreate => "Create Phone Numbers",
            Self::PhoneNumberRead => "View Phone Numbers",
            Self::PhoneNumberUpdate => "Update Phone Numbers",
            Self::PhoneNumberDelete => "Delete Phone Numbers",
            Self::PhoneNumberViewCarrierData => "View Carrier Data",
            Self::DeviceCreate => "Create Devices",
            Self::DeviceRead => "View Devices",
            Self::DeviceUpdate => "Update Devices",
            Self::DeviceDelete => "Delete Devices",
            Self::CallView => "View Calls",
            Self::CallControl => "Control Calls",
            Self::CallRecordingAccess => "Access Recordings",
            Self::RegistrationView => "View Registrations",
            Self::RegistrationManage => "Manage Registrations",
            Self::TrunkCreate => "Create Trunks",
            Self::TrunkRead => "View Trunks",
            Self::TrunkUpdate => "Update Trunks",
            Self::TrunkDelete => "Delete Trunks",
            Self::RoutingRuleRead => "View Routing Rules",
            Self::RoutingRuleManage => "Manage Routing Rules",
            Self::TenantCreate => "Create Tenants",
            Self::TenantRead => "View Tenants",
            Self::TenantUpdate => "Update Tenants",
            Self::TenantDelete => "Delete Tenants",
            Self::DebugView => "View Debug Information",
            Self::ComplianceManage => "Manage Compliance Policy",
            Self::LegalHoldManage => "Manage Legal Holds",
            Self::SubjectRequestManage => "Manage Subject Requests",
            Self::AuditView => "View Audit Logs",
        }
    }
}

impl UserRole {
    /// Check if this role has a specific permission in the given deployment mode
    pub fn has_permission(&self, permission: Permission, mode: DeploymentMode) -> bool {
        use Permission::*;
        use UserRole::*;

        match (self, permission, mode) {
            // SuperAdmin has all permissions in all modes
            (SuperAdmin, _, _) => true,

            // Compliance belongs to whoever administers the tenant: the
            // policy decides what is deleted, holds stop deletion, and
            // subject requests export or erase people's data.
            (
                TenantAdmin,
                ComplianceManage | LegalHoldManage | SubjectRequestManage | AuditView,
                _,
            ) => true,
            (_, ComplianceManage | LegalHoldManage | SubjectRequestManage | AuditView, _) => false,

            // A tenant's routing rules are its call policy — barring premium
            // numbers, sending international calls to a trunk group — and
            // belong to whoever administers the tenant or its telephony, in
            // every mode; the read-only role may look.
            (TenantAdmin | TelecomAdmin, RoutingRuleRead | RoutingRuleManage, _) => true,
            (ReadOnly, RoutingRuleRead, _) => true,
            (_, RoutingRuleRead | RoutingRuleManage, _) => false,

            // TenantAdmin - full admin within a single tenant.
            //
            // Can manage every resource that lives inside a tenant (users,
            // numbers, devices, calls, trunks when enabled, recordings,
            // registrations) but CANNOT touch platform-wide configuration,
            // cannot create or delete tenants, and cannot see logs from
            // other tenants. The API layer is responsible for scoping
            // list/read operations to the session's own tenant_id.
            (TenantAdmin, SystemConfig, _) => false,
            (TenantAdmin, SystemViewLogs, _) => true,
            (TenantAdmin, UserCreate | UserRead | UserUpdate | UserDelete, _) => true,
            (TenantAdmin, UserSetPassword, _) => true,
            (
                TenantAdmin,
                PhoneNumberCreate | PhoneNumberRead | PhoneNumberUpdate | PhoneNumberDelete,
                _,
            ) => true,
            (
                TenantAdmin,
                PhoneNumberViewCarrierData,
                DeploymentMode::Carrier | DeploymentMode::ServiceProvider,
            ) => true,
            (TenantAdmin, PhoneNumberViewCarrierData, DeploymentMode::Enterprise) => false,
            (TenantAdmin, DeviceCreate | DeviceRead | DeviceUpdate | DeviceDelete, _) => true,
            (TenantAdmin, CallView | CallControl | CallRecordingAccess, _) => true,
            (TenantAdmin, RegistrationView | RegistrationManage, _) => true,
            (
                TenantAdmin,
                TrunkCreate | TrunkRead | TrunkUpdate | TrunkDelete,
                DeploymentMode::Carrier | DeploymentMode::ServiceProvider,
            ) => true,
            (
                TenantAdmin,
                TrunkCreate | TrunkRead | TrunkUpdate | TrunkDelete,
                DeploymentMode::Enterprise,
            ) => false,
            // Tenant CRUD: read and update own tenant, never create/delete.
            (TenantAdmin, TenantRead | TenantUpdate, _) => true,
            (TenantAdmin, TenantCreate | TenantDelete, _) => false,
            (TenantAdmin, DebugView, _) => true,

            // TelecomAdmin - voice infrastructure management
            (TelecomAdmin, SystemConfig, _) => false, // Can't change system config
            (TelecomAdmin, SystemViewLogs, _) => true,
            (TelecomAdmin, UserCreate | UserRead | UserUpdate | UserDelete, _) => false, // User management is UserAdmin's job
            (TelecomAdmin, UserSetPassword, _) => false,
            (
                TelecomAdmin,
                PhoneNumberCreate | PhoneNumberRead | PhoneNumberUpdate | PhoneNumberDelete,
                _,
            ) => true,
            (
                TelecomAdmin,
                PhoneNumberViewCarrierData,
                DeploymentMode::Carrier | DeploymentMode::ServiceProvider,
            ) => true,
            (TelecomAdmin, PhoneNumberViewCarrierData, DeploymentMode::Enterprise) => false,
            (TelecomAdmin, DeviceCreate | DeviceRead | DeviceUpdate | DeviceDelete, _) => true,
            (TelecomAdmin, CallView | CallControl, _) => true,
            (TelecomAdmin, CallRecordingAccess, _) => true,
            (TelecomAdmin, RegistrationView | RegistrationManage, _) => true,
            (
                TelecomAdmin,
                TrunkCreate | TrunkRead | TrunkUpdate | TrunkDelete,
                DeploymentMode::Carrier | DeploymentMode::ServiceProvider,
            ) => true,
            (
                TelecomAdmin,
                TrunkCreate | TrunkRead | TrunkUpdate | TrunkDelete,
                DeploymentMode::Enterprise,
            ) => false,
            (
                TelecomAdmin,
                TenantCreate | TenantRead | TenantUpdate | TenantDelete,
                DeploymentMode::ServiceProvider,
            ) => true,
            (TelecomAdmin, TenantCreate | TenantRead | TenantUpdate | TenantDelete, _) => false,
            (TelecomAdmin, DebugView, _) => true,

            // UserAdmin - user and password management only
            (UserAdmin, SystemConfig | SystemViewLogs, _) => false,
            (UserAdmin, UserCreate | UserRead | UserUpdate | UserDelete, _) => true,
            (UserAdmin, UserSetPassword, _) => true,
            (UserAdmin, PhoneNumberRead, _) => true, // View only
            (
                UserAdmin,
                PhoneNumberCreate
                | PhoneNumberUpdate
                | PhoneNumberDelete
                | PhoneNumberViewCarrierData,
                _,
            ) => false,
            (UserAdmin, DeviceRead, _) => true, // View only
            (UserAdmin, DeviceCreate | DeviceUpdate | DeviceDelete, _) => false,
            (UserAdmin, CallView, _) => true,
            (UserAdmin, CallControl | CallRecordingAccess, _) => false,
            (UserAdmin, RegistrationView, _) => true,
            (UserAdmin, RegistrationManage, _) => false,
            (UserAdmin, TrunkCreate | TrunkRead | TrunkUpdate | TrunkDelete, _) => false,
            (UserAdmin, TenantCreate | TenantRead | TenantUpdate | TenantDelete, _) => false,
            (UserAdmin, DebugView, _) => false,

            // DepartmentAdmin - limited scope management
            (DepartmentAdmin, SystemConfig | SystemViewLogs, _) => false,
            (DepartmentAdmin, UserRead, _) => true, // Within department only
            (DepartmentAdmin, UserCreate | UserUpdate | UserDelete | UserSetPassword, _) => false,
            (DepartmentAdmin, PhoneNumberRead | DeviceRead, _) => true, // Within department only
            (
                DepartmentAdmin,
                PhoneNumberCreate
                | PhoneNumberUpdate
                | PhoneNumberDelete
                | PhoneNumberViewCarrierData,
                _,
            ) => false,
            (DepartmentAdmin, DeviceCreate | DeviceUpdate | DeviceDelete, _) => false,
            (DepartmentAdmin, CallView, _) => true, // Within department only
            (DepartmentAdmin, CallControl | CallRecordingAccess, _) => false,
            (DepartmentAdmin, RegistrationView, _) => true,
            (DepartmentAdmin, RegistrationManage, _) => false,
            (DepartmentAdmin, TrunkCreate | TrunkRead | TrunkUpdate | TrunkDelete, _) => false,
            (DepartmentAdmin, TenantCreate | TenantRead | TenantUpdate | TenantDelete, _) => false,
            (DepartmentAdmin, DebugView, _) => false,

            // ReadOnly - view everything, change nothing
            (ReadOnly, SystemConfig, _) => false,
            (ReadOnly, SystemViewLogs, _) => true,
            (ReadOnly, UserRead | PhoneNumberRead | DeviceRead, _) => true,
            (ReadOnly, UserCreate | UserUpdate | UserDelete | UserSetPassword, _) => false,
            (ReadOnly, PhoneNumberCreate | PhoneNumberUpdate | PhoneNumberDelete, _) => false,
            (
                ReadOnly,
                PhoneNumberViewCarrierData,
                DeploymentMode::Carrier | DeploymentMode::ServiceProvider,
            ) => true,
            (ReadOnly, PhoneNumberViewCarrierData, DeploymentMode::Enterprise) => false,
            (ReadOnly, DeviceCreate | DeviceUpdate | DeviceDelete, _) => false,
            (ReadOnly, CallView, _) => true,
            (ReadOnly, CallControl | CallRecordingAccess, _) => false,
            (ReadOnly, RegistrationView, _) => true,
            (ReadOnly, RegistrationManage, _) => false,
            (ReadOnly, TrunkRead, DeploymentMode::Carrier | DeploymentMode::ServiceProvider) => {
                true
            }
            (ReadOnly, TrunkRead, DeploymentMode::Enterprise) => false,
            (ReadOnly, TrunkCreate | TrunkUpdate | TrunkDelete, _) => false,
            (ReadOnly, TenantRead, DeploymentMode::ServiceProvider) => true,
            (ReadOnly, TenantRead, DeploymentMode::Carrier | DeploymentMode::Enterprise) => false,
            (ReadOnly, TenantCreate | TenantUpdate | TenantDelete, _) => false,
            (ReadOnly, DebugView, _) => true,

            // EndUser - very limited, personal settings only
            (EndUser, SystemConfig | SystemViewLogs, _) => false,
            (EndUser, UserRead, _) => true, // Only their own profile
            (EndUser, UserCreate | UserUpdate | UserDelete | UserSetPassword, _) => false,
            (EndUser, PhoneNumberRead | DeviceRead, _) => true, // Only their assigned resources
            (
                EndUser,
                PhoneNumberCreate
                | PhoneNumberUpdate
                | PhoneNumberDelete
                | PhoneNumberViewCarrierData,
                _,
            ) => false,
            (EndUser, DeviceCreate | DeviceUpdate | DeviceDelete, _) => false,
            (EndUser, CallView, _) => true, // Only their own calls
            (EndUser, CallControl | CallRecordingAccess, _) => false,
            (EndUser, RegistrationView | RegistrationManage, _) => false,
            (EndUser, TrunkCreate | TrunkRead | TrunkUpdate | TrunkDelete, _) => false,
            (EndUser, TenantCreate | TenantRead | TenantUpdate | TenantDelete, _) => false,
            (EndUser, DebugView, _) => false,
        }
    }

    /// Get all permissions this role has in the given mode
    pub fn permissions(&self, mode: DeploymentMode) -> Vec<Permission> {
        use Permission::*;

        let all_permissions = vec![
            SystemConfig,
            SystemViewLogs,
            UserCreate,
            UserRead,
            UserUpdate,
            UserDelete,
            UserSetPassword,
            PhoneNumberCreate,
            PhoneNumberRead,
            PhoneNumberUpdate,
            PhoneNumberDelete,
            PhoneNumberViewCarrierData,
            DeviceCreate,
            DeviceRead,
            DeviceUpdate,
            DeviceDelete,
            CallView,
            CallControl,
            CallRecordingAccess,
            RegistrationView,
            RegistrationManage,
            TrunkCreate,
            TrunkRead,
            TrunkUpdate,
            TrunkDelete,
            TenantCreate,
            TenantRead,
            TenantUpdate,
            TenantDelete,
            DebugView,
            ComplianceManage,
            LegalHoldManage,
            SubjectRequestManage,
            AuditView,
        ];

        all_permissions
            .into_iter()
            .filter(|p| self.has_permission(*p, mode))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_super_admin_has_all_permissions() {
        let role = UserRole::SuperAdmin;
        let mode = DeploymentMode::Enterprise;

        assert!(role.has_permission(Permission::SystemConfig, mode));
        assert!(role.has_permission(Permission::UserCreate, mode));
        assert!(role.has_permission(Permission::PhoneNumberDelete, mode));
        assert!(role.has_permission(Permission::TrunkCreate, mode));
    }

    #[test]
    fn test_telecom_admin_carrier_data_visibility() {
        let role = UserRole::TelecomAdmin;

        // Can see carrier data in Carrier/ServiceProvider mode
        assert!(role.has_permission(
            Permission::PhoneNumberViewCarrierData,
            DeploymentMode::Carrier
        ));
        assert!(role.has_permission(
            Permission::PhoneNumberViewCarrierData,
            DeploymentMode::ServiceProvider
        ));

        // Cannot see carrier data in Enterprise mode
        assert!(!role.has_permission(
            Permission::PhoneNumberViewCarrierData,
            DeploymentMode::Enterprise
        ));
    }

    #[test]
    fn compliance_belongs_to_tenant_and_super_admins_only() {
        use Permission::*;
        for mode in [
            DeploymentMode::Enterprise,
            DeploymentMode::Carrier,
            DeploymentMode::ServiceProvider,
        ] {
            for perm in [
                ComplianceManage,
                LegalHoldManage,
                SubjectRequestManage,
                AuditView,
            ] {
                assert!(UserRole::SuperAdmin.has_permission(perm, mode));
                assert!(UserRole::TenantAdmin.has_permission(perm, mode));
                for role in [
                    UserRole::TelecomAdmin,
                    UserRole::UserAdmin,
                    UserRole::DepartmentAdmin,
                    UserRole::ReadOnly,
                    UserRole::EndUser,
                ] {
                    assert!(!role.has_permission(perm, mode), "{role:?} {perm:?}");
                }
                assert_eq!(perm.as_str().parse::<Permission>(), Ok(perm));
            }
        }
    }

    #[test]
    fn test_read_only_cannot_modify() {
        let role = UserRole::ReadOnly;
        let mode = DeploymentMode::Enterprise;

        // Can view
        assert!(role.has_permission(Permission::UserRead, mode));
        assert!(role.has_permission(Permission::PhoneNumberRead, mode));

        // Cannot modify
        assert!(!role.has_permission(Permission::UserCreate, mode));
        assert!(!role.has_permission(Permission::PhoneNumberUpdate, mode));
        assert!(!role.has_permission(Permission::DeviceDelete, mode));
    }

    #[test]
    fn test_trunk_permissions_by_mode() {
        let role = UserRole::TelecomAdmin;

        // Trunks available in Carrier/ServiceProvider
        assert!(role.has_permission(Permission::TrunkCreate, DeploymentMode::Carrier));
        assert!(role.has_permission(Permission::TrunkCreate, DeploymentMode::ServiceProvider));

        // Not available in Enterprise
        assert!(!role.has_permission(Permission::TrunkCreate, DeploymentMode::Enterprise));
    }

    #[test]
    fn test_tenant_permissions_only_in_service_provider() {
        let role = UserRole::TelecomAdmin;

        // Only in ServiceProvider mode
        assert!(role.has_permission(Permission::TenantCreate, DeploymentMode::ServiceProvider));

        // Not in other modes
        assert!(!role.has_permission(Permission::TenantCreate, DeploymentMode::Carrier));
        assert!(!role.has_permission(Permission::TenantCreate, DeploymentMode::Enterprise));
    }

    #[test]
    fn test_tenant_admin_has_full_tenant_scope_but_no_platform_rights() {
        let role = UserRole::TenantAdmin;
        let mode = DeploymentMode::ServiceProvider;

        // Full user + resource management inside the tenant.
        assert!(role.has_permission(Permission::UserCreate, mode));
        assert!(role.has_permission(Permission::UserUpdate, mode));
        assert!(role.has_permission(Permission::UserDelete, mode));
        assert!(role.has_permission(Permission::UserSetPassword, mode));
        assert!(role.has_permission(Permission::PhoneNumberCreate, mode));
        assert!(role.has_permission(Permission::DeviceCreate, mode));
        assert!(role.has_permission(Permission::CallView, mode));
        assert!(role.has_permission(Permission::CallControl, mode));
        assert!(role.has_permission(Permission::CallRecordingAccess, mode));
        assert!(role.has_permission(Permission::RegistrationManage, mode));
        assert!(role.has_permission(Permission::TrunkCreate, mode));
        assert!(role.has_permission(Permission::TenantRead, mode));
        assert!(role.has_permission(Permission::TenantUpdate, mode));

        // Platform-level rights are denied.
        assert!(!role.has_permission(Permission::SystemConfig, mode));
        assert!(!role.has_permission(Permission::TenantCreate, mode));
        assert!(!role.has_permission(Permission::TenantDelete, mode));
    }

    #[test]
    fn test_tenant_admin_is_not_platform_admin() {
        // This is the key check the API layer relies on to gate
        // cross-tenant and system-wide endpoints.
        assert!(!UserRole::TenantAdmin.is_platform_admin());
        assert!(!UserRole::TelecomAdmin.is_platform_admin());
        assert!(!UserRole::UserAdmin.is_platform_admin());
        assert!(!UserRole::ReadOnly.is_platform_admin());
        assert!(!UserRole::EndUser.is_platform_admin());
        assert!(UserRole::SuperAdmin.is_platform_admin());
    }

    #[test]
    fn test_tenant_admin_enterprise_mode_has_no_trunks_or_tenant_rights() {
        // Trunks aren't a feature in Enterprise mode and tenants don't exist
        // there either, so a tenant admin should not have those permissions
        // even though they would in a multi-tenant deployment.
        let role = UserRole::TenantAdmin;
        let mode = DeploymentMode::Enterprise;

        assert!(!role.has_permission(Permission::TrunkCreate, mode));
        assert!(!role.has_permission(Permission::TrunkRead, mode));
    }

    /// A tenant's routing rules belong to whoever administers it or its
    /// telephony, in every mode — including enterprise, where trunks are
    /// the platform's (`docs/TENANT_TRUNKS.md` §12, phase 6d).
    #[test]
    fn routing_rules_belong_to_the_tenants_administrators_in_every_mode() {
        for mode in [DeploymentMode::Enterprise, DeploymentMode::Carrier] {
            for role in [UserRole::TenantAdmin, UserRole::TelecomAdmin] {
                assert!(role.has_permission(Permission::RoutingRuleManage, mode));
                assert!(role.has_permission(Permission::RoutingRuleRead, mode));
            }
            assert!(UserRole::ReadOnly.has_permission(Permission::RoutingRuleRead, mode));
            assert!(!UserRole::ReadOnly.has_permission(Permission::RoutingRuleManage, mode));
            for role in [
                UserRole::EndUser,
                UserRole::UserAdmin,
                UserRole::DepartmentAdmin,
            ] {
                assert!(!role.has_permission(Permission::RoutingRuleRead, mode));
            }
        }
        assert_eq!(
            "routing_rule_manage".parse::<Permission>(),
            Ok(Permission::RoutingRuleManage)
        );
    }
}
