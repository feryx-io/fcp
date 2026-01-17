//! Devices view showing devices and details

use crate::api::DeviceResponse;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};

/// State for the devices view
pub struct DevicesViewState {
    /// List of devices
    pub devices: Vec<DeviceResponse>,
    /// Table state for selection
    pub table_state: TableState,
    /// Last error message
    pub error: Option<String>,
    /// Loading state
    pub loading: bool,
}

impl Default for DevicesViewState {
    fn default() -> Self {
        let mut table_state = TableState::default();
        table_state.select(Some(0));

        Self {
            devices: Vec::new(),
            table_state,
            error: None,
            loading: false,
        }
    }
}

impl DevicesViewState {
    /// Get currently selected device
    pub fn selected_device(&self) -> Option<&DeviceResponse> {
        self.table_state.selected().and_then(|i| self.devices.get(i))
    }

    /// Move selection up
    pub fn select_previous(&mut self) {
        if self.devices.is_empty() {
            return;
        }

        let selected = self.table_state.selected().unwrap_or(0);
        if selected > 0 {
            self.table_state.select(Some(selected - 1));
        }
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        if self.devices.is_empty() {
            return;
        }

        let selected = self.table_state.selected().unwrap_or(0);
        if selected < self.devices.len() - 1 {
            self.table_state.select(Some(selected + 1));
        }
    }

    /// Update devices list
    pub fn set_devices(&mut self, devices: Vec<DeviceResponse>) {
        self.devices = devices;
        // Reset selection to first item if we had no selection or it's out of bounds
        if self.table_state.selected().is_none()
            || self.table_state.selected().unwrap() >= self.devices.len()
        {
            self.table_state.select(if self.devices.is_empty() {
                None
            } else {
                Some(0)
            });
        }
    }
}

/// Render the devices view
pub fn render(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &mut DevicesViewState,
    area: Rect,
) {
    // Split into table and details panel
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(55), // Devices table
            Constraint::Percentage(45), // Details panel
        ])
        .split(area);

    render_devices_table(f, theme, state, chunks[0]);
    render_details_panel(f, theme, state, chunks[1]);
}

/// Render the devices table
fn render_devices_table(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &mut DevicesViewState,
    area: Rect,
) {
    let header = Row::new(vec!["Name", "Type", "MAC Address", "Model", "Status"])
        .style(theme.title())
        .bottom_margin(1);

    let rows: Vec<Row> = state
        .devices
        .iter()
        .map(|device| {
            // Format status
            let status = if device.enabled {
                match device.registration_state.as_str() {
                    "registered" => "✓ Registered",
                    _ => "○ Enabled",
                }
            } else {
                "✗ Disabled"
            };

            // Format MAC address
            let mac = device.mac_address.as_deref().unwrap_or("-");

            // Format model
            let model = device.model.as_deref().unwrap_or("-");

            Row::new(vec![
                Cell::from(device.name.clone()),
                Cell::from(device.device_type.clone()),
                Cell::from(mac),
                Cell::from(model),
                Cell::from(status),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(20), // Name
            Constraint::Length(12), // Type
            Constraint::Length(18), // MAC
            Constraint::Length(20), // Model
            Constraint::Length(15), // Status
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title("Devices (↑↓ to navigate)")
            .borders(Borders::ALL)
            .border_style(theme.border()),
    )
    .row_highlight_style(theme.selected())
    .highlight_symbol("► ");

    f.render_stateful_widget(table, area, &mut state.table_state);
}

/// Render the details panel
fn render_details_panel(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &DevicesViewState,
    area: Rect,
) {
    let content = if let Some(device) = state.selected_device() {
        vec![
            Line::from(vec![
                Span::styled("Device Name: ", theme.muted()),
                Span::raw(&device.name),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("ID: ", theme.muted()),
                Span::raw(format_short_id(&device.id)),
            ]),
            Line::from(vec![
                Span::styled("Type: ", theme.muted()),
                Span::raw(&device.device_type),
            ]),
            Line::from(vec![
                Span::styled("Enabled: ", theme.muted()),
                Span::raw(if device.enabled { "Yes" } else { "No" }),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("MAC Address: ", theme.muted()),
                Span::raw(device.mac_address.as_deref().unwrap_or("-")),
            ]),
            Line::from(vec![
                Span::styled("Model: ", theme.muted()),
                Span::raw(device.model.as_deref().unwrap_or("-")),
            ]),
            Line::from(vec![
                Span::styled("Firmware: ", theme.muted()),
                Span::raw(device.firmware_version.as_deref().unwrap_or("-")),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("SIP URI: ", theme.muted()),
                Span::raw(device.sip_uri.as_deref().unwrap_or("-")),
            ]),
            Line::from(vec![
                Span::styled("Registration: ", theme.muted()),
                Span::raw(&device.registration_state),
            ]),
            if let Some(ip) = &device.last_ip_address {
                Line::from(vec![
                    Span::styled("Last IP: ", theme.muted()),
                    Span::raw(ip),
                ])
            } else {
                Line::from("")
            },
            Line::from(""),
            Line::from(vec![
                Span::styled("Owner User: ", theme.muted()),
                Span::raw(device.owner_user_id.as_deref().unwrap_or("-")),
            ]),
            Line::from(vec![
                Span::styled("Location: ", theme.muted()),
                Span::raw(device.location.as_deref().unwrap_or("-")),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Max Lines: ", theme.muted()),
                Span::raw(device.max_lines.to_string()),
            ]),
            Line::from(vec![
                Span::styled("Video: ", theme.muted()),
                Span::raw(if device.supports_video {
                    "Supported"
                } else {
                    "Not supported"
                }),
            ]),
            Line::from(vec![
                Span::styled("BLF: ", theme.muted()),
                Span::raw(if device.supports_blf {
                    "Supported"
                } else {
                    "Not supported"
                }),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Codecs: ", theme.muted()),
                Span::raw(if device.codec_support.is_empty() {
                    "-".to_string()
                } else {
                    device.codec_support.join(", ")
                }),
            ]),
            if let Some(ua) = &device.user_agent {
                Line::from(vec![Span::styled("User Agent: ", theme.muted()), Span::raw(ua)])
            } else {
                Line::from("")
            },
        ]
    } else {
        vec![Line::from("No device selected")]
    };

    let paragraph = Paragraph::new(content).block(
        Block::default()
            .title("Details")
            .borders(Borders::ALL)
            .border_style(theme.border()),
    );

    f.render_widget(paragraph, area);
}

/// Format a short ID for display
fn format_short_id(id: &str) -> String {
    if id.len() > 20 {
        format!("{}...", &id[..20])
    } else {
        id.to_string()
    }
}

/// Fetch devices from the API
pub async fn fetch_devices(api_client: &crate::api::ApiClient, state: &mut DevicesViewState) {
    state.loading = true;
    state.error = None;

    match api_client.list_devices(None).await {
        Ok(devices) => {
            state.set_devices(devices);
            state.loading = false;
        }
        Err(e) => {
            state.error = Some(format!("Failed to fetch devices: {}", e));
            state.loading = false;
        }
    }
}

/// Handle navigation events
pub fn handle_navigation(state: &mut DevicesViewState, event: &crate::tui::event::Event) {
    use crate::tui::event::Event;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    match event {
        Event::Key(KeyEvent {
            code: KeyCode::Up,
            modifiers: KeyModifiers::NONE,
            ..
        }) => {
            state.select_previous();
        }
        Event::Key(KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::NONE,
            ..
        }) => {
            state.select_next();
        }
        _ => {}
    }
}
