//! Main TUI application state

use super::theme::Theme;
use super::views::calls::CallsViewState;
use super::views::debug::DebugViewState;
use super::views::registrations::RegistrationsViewState;
use super::views::users::UsersViewState;
use crate::api::ApiClient;
use crate::config::ResolvedContext;
use anyhow::Result;

/// Active tab/view in the application
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dashboard,
    Users,
    Calls,
    Registrations,
    Debug,
}

impl Tab {
    /// Get all available tabs
    pub fn all() -> &'static [Tab] {
        &[
            Tab::Dashboard,
            Tab::Users,
            Tab::Calls,
            Tab::Registrations,
            Tab::Debug,
        ]
    }

    /// Get tab title
    pub fn title(&self) -> &'static str {
        match self {
            Tab::Dashboard => "Dashboard",
            Tab::Users => "Users",
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

    /// Get tab by index (0-4)
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
    /// Last error message (if any)
    pub error: Option<String>,
    /// Last update timestamp
    pub last_update: std::time::Instant,
    /// Users view state
    pub users_view: UsersViewState,
    /// Calls view state
    pub calls_view: CallsViewState,
    /// Registrations view state
    pub registrations_view: RegistrationsViewState,
    /// Debug view state
    pub debug_view: DebugViewState,
}

impl App {
    /// Create a new application
    pub fn new(context: ResolvedContext) -> Result<Self> {
        let api_client = ApiClient::new(context.url.clone(), context.token.clone())?;

        Ok(Self {
            should_quit: false,
            active_tab: Tab::Dashboard,
            theme: Theme::default(),
            api_client,
            context,
            error: None,
            last_update: std::time::Instant::now(),
            users_view: UsersViewState::default(),
            calls_view: CallsViewState::default(),
            registrations_view: RegistrationsViewState::default(),
            debug_view: DebugViewState::default(),
        })
    }

    /// Navigate to the next tab
    pub fn next_tab(&mut self) {
        self.active_tab = self.active_tab.next();
    }

    /// Navigate to the previous tab
    pub fn prev_tab(&mut self) {
        self.active_tab = self.active_tab.prev();
    }

    /// Set the active tab
    pub fn set_tab(&mut self, tab: Tab) {
        self.active_tab = tab;
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
}
