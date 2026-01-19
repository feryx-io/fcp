//! Line bindings view for managing phone number to device mappings

use crate::api::{ApiClient, LineBindingResponse, PhoneNumberResponse, DeviceResponse};
use crate::tui::theme::Theme;
use anyhow::Result;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};
use std::time::SystemTime;

/// State for the line bindings view
pub struct LineBindingsViewState {
    /// List of line bindings
    pub bindings: Vec<LineBindingResponse>,
    /// Phone numbers for display
    pub phone_numbers: Vec<PhoneNumberResponse>,
    /// Devices for display
    pub devices: Vec<DeviceResponse>,
    /// Currently selected index
    pub selected_index: usize,
    /// Last update time
    pub last_update: SystemTime,
}

impl LineBindingsViewState {
    /// Create a new line bindings view state
    pub fn new() -> Self {
        Self {
            bindings: Vec::new(),
            phone_numbers: Vec::new(),
            devices: Vec::new(),
            selected_index: 0,
            last_update: SystemTime::now(),
        }
    }

    /// Refresh the line bindings list
    pub async fn refresh(&mut self, api_client: &ApiClient) -> Result<()> {
        self.bindings = api_client.list_line_bindings(None).await?;
        self.phone_numbers = api_client.list_phone_numbers(None).await?;
        self.devices = api_client.list_devices(None).await?;
        self.last_update = SystemTime::now();

        // Keep selection in bounds
        if self.selected_index >= self.bindings.len() && !self.bindings.is_empty() {
            self.selected_index = self.bindings.len() - 1;
        }

        Ok(())
    }

    /// Move selection up
    pub fn select_previous(&mut self) {
        if !self.bindings.is_empty() {
            if self.selected_index == 0 {
                self.selected_index = self.bindings.len() - 1;
            } else {
                self.selected_index -= 1;
            }
        }
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        if !self.bindings.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.bindings.len();
        }
    }

    /// Get currently selected binding
    pub fn selected_binding(&self) -> Option<&LineBindingResponse> {
        self.bindings.get(self.selected_index)
    }

    /// Get phone number by ID
    pub fn get_phone_number(&self, phone_number_id: &str) -> Option<&PhoneNumberResponse> {
        self.phone_numbers.iter().find(|p| p.id == phone_number_id)
    }

    /// Get device by ID
    pub fn get_device(&self, device_id: &str) -> Option<&DeviceResponse> {
        self.devices.iter().find(|d| d.id == device_id)
    }

    /// Delete the currently selected binding
    pub async fn delete_selected(&mut self, api_client: &ApiClient) -> Result<()> {
        if let Some(binding) = self.selected_binding() {
            let id = binding.id.clone();
            api_client.delete_line_binding(&id).await?;
            self.refresh(api_client).await?;
        }
        Ok(())
    }
}

/// Render the line bindings view
pub fn render(frame: &mut Frame, area: Rect, state: &LineBindingsViewState, theme: &Theme) {
    // Split into list and details
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    render_bindings_list(frame, chunks[0], state, theme);
    render_binding_details(frame, chunks[1], state, theme);
}

/// Render the list of line bindings
fn render_bindings_list(frame: &mut Frame, area: Rect, state: &LineBindingsViewState, theme: &Theme) {
    let items: Vec<ListItem> = state
        .bindings
        .iter()
        .enumerate()
        .map(|(idx, binding)| {
            // Get phone number for display
            let phone_number = state.get_phone_number(&binding.phone_number_id)
                .map(|p| p.e164.clone())
                .unwrap_or_else(|| binding.phone_number_id.clone());

            let device_count = binding.devices.len();
            let enabled_count = binding.devices.iter().filter(|d| d.enabled).count();

            let content = format!(
                "{} → {} ({}/{} devices)",
                phone_number,
                binding.binding_type,
                enabled_count,
                device_count
            );

            let style = if idx == state.selected_index {
                theme.selected()
            } else {
                theme.normal()
            };

            ListItem::new(content).style(style)
        })
        .collect();

    let title = format!(" Line Bindings ({}) ", state.bindings.len());
    let list = List::new(items)
        .block(
            Block::default()
                .title(title)
                .title_alignment(Alignment::Center)
                .borders(Borders::ALL)
                .border_style(theme.border()),
        );

    frame.render_widget(list, area);
}

/// Render details of the selected binding
fn render_binding_details(frame: &mut Frame, area: Rect, state: &LineBindingsViewState, theme: &Theme) {
    let selected = state.selected_binding();

    let mut lines = Vec::new();

    if let Some(binding) = selected {
        // Phone number info
        if let Some(phone) = state.get_phone_number(&binding.phone_number_id) {
            lines.push(Line::from(vec![
                Span::styled("Phone Number: ", theme.title()),
                Span::styled(&phone.e164, theme.primary()),
            ]));
            if let Some(cnam) = &phone.cnam {
                lines.push(Line::from(vec![
                    Span::styled("  CNAM: ", theme.muted()),
                    Span::styled(cnam, theme.normal()),
                ]));
            }
        } else {
            lines.push(Line::from(vec![
                Span::styled("Phone Number ID: ", theme.title()),
                Span::styled(&binding.phone_number_id, theme.muted()),
            ]));
        }

        lines.push(Line::from(""));

        // Binding type and owner
        lines.push(Line::from(vec![
            Span::styled("Binding Type: ", theme.title()),
            Span::styled(&binding.binding_type, theme.primary()),
        ]));

        if let Some(owner_id) = &binding.owner_user_id {
            lines.push(Line::from(vec![
                Span::styled("Owner User ID: ", theme.title()),
                Span::styled(owner_id, theme.normal()),
            ]));
        }

        lines.push(Line::from(""));

        // Ring strategy
        lines.push(Line::from(vec![
            Span::styled("Ring Strategy: ", theme.title()),
            Span::styled(&binding.ring_strategy, theme.normal()),
        ]));

        lines.push(Line::from(vec![
            Span::styled("No Answer Timeout: ", theme.title()),
            Span::styled(format!("{}s", binding.no_answer_timeout_secs), theme.normal()),
        ]));

        lines.push(Line::from(""));

        // Call forwarding
        lines.push(Line::from(Span::styled("Call Forwarding:", theme.title())));

        let fwd_busy = binding.forward_on_busy.as_deref().unwrap_or("None");
        lines.push(Line::from(vec![
            Span::styled("  On Busy: ", theme.muted()),
            Span::styled(fwd_busy, theme.normal()),
        ]));

        let fwd_no_answer = binding.forward_on_no_answer.as_deref().unwrap_or("None");
        lines.push(Line::from(vec![
            Span::styled("  No Answer: ", theme.muted()),
            Span::styled(fwd_no_answer, theme.normal()),
        ]));

        let fwd_unconditional = binding.forward_unconditional.as_deref().unwrap_or("None");
        lines.push(Line::from(vec![
            Span::styled("  Unconditional: ", theme.muted()),
            Span::styled(fwd_unconditional, theme.normal()),
        ]));

        lines.push(Line::from(""));

        // Devices
        lines.push(Line::from(Span::styled(
            format!("Devices ({}):", binding.devices.len()),
            theme.title(),
        )));

        for device_binding in &binding.devices {
            if let Some(device) = state.get_device(&device_binding.device_id) {
                let status_icon = if device_binding.enabled { "✓" } else { "✗" };
                let reg_state = match device.registration_state.as_str() {
                    "registered" => "🟢",
                    "unregistered" => "⚪",
                    "failed" => "🔴",
                    _ => "⚫",
                };

                lines.push(Line::from(vec![
                    Span::styled(format!("  {} {} ", status_icon, reg_state), theme.muted()),
                    Span::styled(&device.name, theme.primary()),
                ]));

                lines.push(Line::from(vec![
                    Span::styled("    Priority: ", theme.muted()),
                    Span::styled(device_binding.priority.to_string(), theme.normal()),
                    Span::styled(", Line: ", theme.muted()),
                    Span::styled(device_binding.line_appearance.to_string(), theme.normal()),
                    Span::styled(if device_binding.shared_line { ", Shared" } else { "" }, theme.muted()),
                ]));

                if let Some(sip_uri) = &device.sip_uri {
                    lines.push(Line::from(vec![
                        Span::styled("    URI: ", theme.muted()),
                        Span::styled(sip_uri, theme.muted()),
                    ]));
                }
            } else {
                lines.push(Line::from(vec![
                    Span::styled("  ✗ ", theme.error()),
                    Span::styled(&device_binding.device_id, theme.muted()),
                    Span::styled(" (not found)", theme.error()),
                ]));
            }
        }

        lines.push(Line::from(""));

        // Tenant and timestamps
        lines.push(Line::from(vec![
            Span::styled("Tenant ID: ", theme.muted()),
            Span::styled(&binding.tenant_id, theme.muted()),
        ]));

        // Format timestamps
        let created = format_system_time(&binding.created_at);
        lines.push(Line::from(vec![
            Span::styled("Created: ", theme.muted()),
            Span::styled(created, theme.muted()),
        ]));

        let updated = format_system_time(&binding.updated_at);
        lines.push(Line::from(vec![
            Span::styled("Updated: ", theme.muted()),
            Span::styled(updated, theme.muted()),
        ]));

        lines.push(Line::from(""));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled("Actions:", theme.title())]));
        lines.push(Line::from(vec![
            Span::styled("  ", theme.muted()),
            Span::styled("d", theme.primary()),
            Span::styled(" - Delete  ", theme.normal()),
            Span::styled("r", theme.primary()),
            Span::styled(" - Refresh", theme.normal()),
        ]));
    } else {
        lines.push(Line::from(Span::styled(
            "No line binding selected",
            theme.muted(),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Press 'r' to refresh the list",
            theme.muted(),
        )));
    }

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .title(" Line Binding Details ")
                .title_alignment(Alignment::Center)
                .borders(Borders::ALL)
                .border_style(theme.border()),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);
}

/// Format SystemTime for display
fn format_system_time(time: &SystemTime) -> String {
    match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => {
            let secs = duration.as_secs();
            let datetime = chrono::DateTime::<chrono::Utc>::from_timestamp(secs as i64, 0);
            datetime
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| "Invalid time".to_string())
        }
        Err(_) => "Invalid time".to_string(),
    }
}
