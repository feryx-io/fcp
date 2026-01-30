//! Main TUI application state

use super::theme::Theme;
use super::views::calls::CallsViewState;
use super::views::debug::DebugViewState;
use super::views::devices::DevicesViewState;
use super::views::line_bindings::LineBindingsViewState;
use super::views::phone_numbers::PhoneNumbersViewState;
use super::views::registrations::RegistrationsViewState;
use super::views::tenants::TenantsState;
use super::views::users::UsersViewState;
use crate::api::ApiClient;
use crate::config::ResolvedContext;
use anyhow::Result;
use fcp_core::{DeploymentMode, Permission, TenantId, UserRole};

/// Current user information for permission checks
#[derive(Debug, Clone)]
pub struct CurrentUser {
    pub username: String,
    pub role: UserRole,
    pub tenant_id: TenantId,
    pub department_id: Option<String>,
}

impl CurrentUser {
    /// Check if user has a specific permission
    pub fn has_permission(&self, permission: Permission, mode: DeploymentMode) -> bool {
        self.role.has_permission(permission, mode)
    }

    /// Check if user can access a tenant
    pub fn can_access_tenant(&self, tenant_id: &TenantId) -> bool {
        match self.role {
            UserRole::SuperAdmin => true,
            _ => &self.tenant_id == tenant_id,
        }
    }

    /// Check if user can access a department
    pub fn can_access_department(&self, dept_id: &str) -> bool {
        match self.role {
            UserRole::SuperAdmin | UserRole::TelecomAdmin | UserRole::UserAdmin => true,
            UserRole::DepartmentAdmin => self.department_id.as_deref() == Some(dept_id),
            _ => false,
        }
    }
}

/// Active tab/view in the application
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dashboard,
    Tenants,
    Users,
    PhoneNumbers,
    Devices,
    LineBindings,
    Calls,
    Registrations,
    Debug,
}

impl Tab {
    /// Get all available tabs
    pub fn all() -> &'static [Tab] {
        &[
            Tab::Dashboard,
            Tab::Tenants,
            Tab::Users,
            Tab::PhoneNumbers,
            Tab::Devices,
            Tab::LineBindings,
            Tab::Calls,
            Tab::Registrations,
            Tab::Debug,
        ]
    }

    /// Get tab title
    pub fn title(&self) -> &'static str {
        match self {
            Tab::Dashboard => "Dashboard",
            Tab::Tenants => "Tenants",
            Tab::Users => "Users",
            Tab::PhoneNumbers => "Phone Numbers",
            Tab::Devices => "Devices",
            Tab::LineBindings => "Line Bindings",
            Tab::Calls => "Calls",
            Tab::Registrations => "Registrations",
            Tab::Debug => "Debug",
        }
    }

    /// Get next tab
    pub fn next(&self) -> Tab {
        let tabs = Self::all();
        let current_idx = tabs.iter().position(|t| t == self).unwrap();
        tabs[(current_idx + 1) % tabs.len()]
    }

    /// Get previous tab
    pub fn prev(&self) -> Tab {
        let tabs = Self::all();
        let current_idx = tabs.iter().position(|t| t == self).unwrap();
        tabs[(current_idx + tabs.len() - 1) % tabs.len()]
    }

    /// Get tab by index (0-7)
    pub fn from_index(index: usize) -> Option<Tab> {
        Self::all().get(index).copied()
    }
}

/// Main application state
pub struct App {
    /// Should the app quit?
    pub should_quit: bool,
    /// Current active tab
    pub active_tab: Tab,
    /// Application theme
    pub theme: Theme,
    /// API client for making requests
    pub api_client: ApiClient,
    /// Server context information
    pub context: ResolvedContext,
    /// Deployment mode for feature availability
    pub deployment_mode: DeploymentMode,
    /// Current user for permission checks
    pub current_user: CurrentUser,
    /// Last error message (if any)
    pub error: Option<String>,
    /// Last update timestamp
    pub last_update: std::time::Instant,
    /// Users view state
    pub users_view: UsersViewState,
    /// Phone numbers view state
    pub phone_numbers_view: PhoneNumbersViewState,
    /// Devices view state
    pub devices_view: DevicesViewState,
    /// Line bindings view state
    pub line_bindings_view: LineBindingsViewState,
    /// Calls view state
    pub calls_view: CallsViewState,
    /// Registrations view state
    pub registrations_view: RegistrationsViewState,
    /// Debug view state
    pub debug_view: DebugViewState,
    /// Tenants view state
    pub tenants_view: TenantsState,
}

impl App {
    /// Create a new application
    pub fn new(context: ResolvedContext) -> Result<Self> {
        let api_client = ApiClient::new(context.url.clone(), context.token.clone())?;

        // TODO: Fetch the actual authenticated user and deployment mode from the API
        // The current implementation uses a hard-coded SuperAdmin user as a workaround.
        // This should be replaced with:
        //   1. GET /api/v1/auth/me or similar endpoint to fetch authenticated user details
        //   2. GET /api/v1/config or read from local config to get deployment mode
        // Using SuperAdmin for now to avoid permission issues (e.g., TelecomAdmin lacks UserRead/UserCreate)
        let deployment_mode = DeploymentMode::Enterprise; // Default for now
        let current_user = CurrentUser {
            username: "admin".to_string(),
            role: UserRole::SuperAdmin, // Temporary: Use SuperAdmin to ensure full access until proper auth is implemented
            tenant_id: TenantId::nil(),
            department_id: None,
        };

        Ok(Self {
            should_quit: false,
            active_tab: Tab::Dashboard,
            theme: Theme::default(),
            api_client,
            context,
            deployment_mode,
            current_user,
            error: None,
            last_update: std::time::Instant::now(),
            users_view: UsersViewState::default(),
            phone_numbers_view: PhoneNumbersViewState::default(),
            devices_view: DevicesViewState::default(),
            line_bindings_view: LineBindingsViewState::new(),
            calls_view: CallsViewState::default(),
            registrations_view: RegistrationsViewState::default(),
            debug_view: DebugViewState::default(),
            tenants_view: TenantsState::new(),
        })
    }

    /// Navigate to the next tab (skips inaccessible tabs)
    pub fn next_tab(&mut self) {
        let accessible = self.accessible_tabs();
        if accessible.is_empty() {
            return;
        }

        let current_idx = accessible.iter().position(|t| t == &self.active_tab);
        let next_idx = match current_idx {
            Some(idx) => (idx + 1) % accessible.len(),
            None => 0, // If current tab not accessible, go to first accessible
        };
        self.active_tab = accessible[next_idx];
    }

    /// Navigate to the previous tab (skips inaccessible tabs)
    pub fn prev_tab(&mut self) {
        let accessible = self.accessible_tabs();
        if accessible.is_empty() {
            return;
        }

        let current_idx = accessible.iter().position(|t| t == &self.active_tab);
        let prev_idx = match current_idx {
            Some(idx) => (idx + accessible.len() - 1) % accessible.len(),
            None => 0, // If current tab not accessible, go to first accessible
        };
        self.active_tab = accessible[prev_idx];
    }

    /// Set the active tab (only if user has access)
    pub fn set_tab(&mut self, tab: Tab) {
        if self.can_access_tab(&tab) {
            self.active_tab = tab;
        }
    }

    /// Set an error message
    pub fn set_error(&mut self, error: String) {
        self.error = Some(error);
    }

    /// Clear the error message
    pub fn clear_error(&mut self) {
        self.error = None;
    }

    /// Mark that an update occurred
    pub fn mark_update(&mut self) {
        self.last_update = std::time::Instant::now();
    }

    /// Get time since last update
    pub fn time_since_update(&self) -> std::time::Duration {
        self.last_update.elapsed()
    }

    /// Request quit
    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    /// Check if user can view carrier data
    pub fn can_view_carrier_data(&self) -> bool {
        self.deployment_mode.has_carrier_features()
            && self
                .current_user
                .has_permission(Permission::PhoneNumberViewCarrierData, self.deployment_mode)
    }

    /// Check if user can edit phone numbers
    pub fn can_edit_phone_numbers(&self) -> bool {
        self.current_user
            .has_permission(Permission::PhoneNumberUpdate, self.deployment_mode)
    }

    /// Check if user can delete phone numbers
    pub fn can_delete_phone_numbers(&self) -> bool {
        self.current_user
            .has_permission(Permission::PhoneNumberDelete, self.deployment_mode)
    }

    /// Check if user can edit devices
    pub fn can_edit_devices(&self) -> bool {
        self.current_user
            .has_permission(Permission::DeviceUpdate, self.deployment_mode)
    }

    /// Check if user can delete devices
    pub fn can_delete_devices(&self) -> bool {
        self.current_user
            .has_permission(Permission::DeviceDelete, self.deployment_mode)
    }

    /// Check if user can create phone numbers
    pub fn can_create_phone_numbers(&self) -> bool {
        self.current_user
            .has_permission(Permission::PhoneNumberCreate, self.deployment_mode)
    }

    /// Check if user can create devices
    pub fn can_create_devices(&self) -> bool {
        self.current_user
            .has_permission(Permission::DeviceCreate, self.deployment_mode)
    }

    /// Check if user can create users
    pub fn can_create_users(&self) -> bool {
        self.current_user
            .has_permission(Permission::UserCreate, self.deployment_mode)
    }

    /// Check if user can edit users
    pub fn can_edit_users(&self) -> bool {
        self.current_user
            .has_permission(Permission::UserUpdate, self.deployment_mode)
    }

    /// Check if user can delete users
    pub fn can_delete_users(&self) -> bool {
        self.current_user
            .has_permission(Permission::UserDelete, self.deployment_mode)
    }

    /// Check if user can set passwords
    pub fn can_set_passwords(&self) -> bool {
        self.current_user
            .has_permission(Permission::UserSetPassword, self.deployment_mode)
    }

    /// Check if user can view debug information
    pub fn can_view_debug(&self) -> bool {
        self.current_user
            .has_permission(Permission::DebugView, self.deployment_mode)
    }

    /// Check if user can view system logs
    pub fn can_view_logs(&self) -> bool {
        self.current_user
            .has_permission(Permission::SystemViewLogs, self.deployment_mode)
    }

    /// Check if user can view calls
    pub fn can_view_calls(&self) -> bool {
        self.current_user
            .has_permission(Permission::CallView, self.deployment_mode)
    }

    /// Check if user can view registrations
    pub fn can_view_registrations(&self) -> bool {
        self.current_user
            .has_permission(Permission::RegistrationView, self.deployment_mode)
    }

    /// Get list of tabs accessible to current user
    pub fn accessible_tabs(&self) -> Vec<Tab> {
        Tab::all()
            .iter()
            .filter(|tab| self.can_access_tab(tab))
            .copied()
            .collect()
    }

    /// Check if user can access a specific tab
    pub fn can_access_tab(&self, tab: &Tab) -> bool {
        match tab {
            Tab::Dashboard => true, // Everyone can see dashboard
            Tab::Tenants => self.current_user.role == UserRole::SuperAdmin, // Only SuperAdmin can manage tenants
            Tab::Users => self
                .current_user
                .has_permission(Permission::UserRead, self.deployment_mode),
            Tab::PhoneNumbers => self
                .current_user
                .has_permission(Permission::PhoneNumberRead, self.deployment_mode),
            Tab::Devices => self
                .current_user
                .has_permission(Permission::DeviceRead, self.deployment_mode),
            Tab::LineBindings => {
                self.current_user
                    .has_permission(Permission::PhoneNumberRead, self.deployment_mode)
                    && self
                        .current_user
                        .has_permission(Permission::DeviceRead, self.deployment_mode)
            }
            Tab::Calls => self
                .current_user
                .has_permission(Permission::CallView, self.deployment_mode),
            Tab::Registrations => self
                .current_user
                .has_permission(Permission::RegistrationView, self.deployment_mode),
            Tab::Debug => self
                .current_user
                .has_permission(Permission::DebugView, self.deployment_mode),
        }
    }
}
