//! Devices view showing devices and details

use crate::api::DeviceResponse;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState},
    Frame,
};
use tui_input::Input;

/// Modal type for device operations
#[derive(Debug, Clone, PartialEq)]
pub enum DeviceModal {
    None,
    Add,
    Edit,
    Delete,
    SelectOwner,
}

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
    /// Active modal
    pub modal: DeviceModal,
    /// Previous modal (used when returning from SelectOwner)
    pub previous_modal: Option<DeviceModal>,
    /// Input field: device name
    pub input_name: Input,
    /// Input field: device type
    pub input_device_type: Input,
    /// Input field: tenant ID (defaults to nil UUID)
    pub input_tenant_id: Input,
    /// Input field: MAC address (optional)
    pub input_mac_address: Input,
    /// Input field: model (optional)
    pub input_model: Input,
    /// Input field: owner user ID (optional)
    pub input_owner_user_id: Input,
    /// List of users for owner selection
    pub users_list: Vec<crate::api::UserResponse>,
    /// User selection state
    pub user_table_state: TableState,
    /// Which input field is focused (0-5)
    pub focused_field: usize,
}

impl Default for DevicesViewState {
    fn default() -> Self {
        let mut table_state = TableState::default();
        table_state.select(Some(0));

        let mut user_table_state = TableState::default();
        user_table_state.select(Some(0));

        Self {
            devices: Vec::new(),
            table_state,
            error: None,
            loading: false,
            modal: DeviceModal::None,
            previous_modal: None,
            input_name: Input::default(),
            input_device_type: Input::new("softphone".to_string()),
            input_tenant_id: Input::new("00000000-0000-0000-0000-000000000000".to_string()),
            input_mac_address: Input::default(),
            input_model: Input::default(),
            input_owner_user_id: Input::default(),
            users_list: Vec::new(),
            user_table_state,
            focused_field: 0,
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

    /// Open add device modal
    pub fn open_add_modal(&mut self) {
        self.modal = DeviceModal::Add;
        self.clear_inputs();
        self.focused_field = 0;
        self.error = None;
    }

    /// Open edit device modal
    pub fn open_edit_modal(&mut self) {
        // Get the data we need before mutating self
        let device_data = self.selected_device().map(|device| {
            (
                device.name.clone(),
                device.device_type.clone(),
                device.mac_address.clone(),
                device.model.clone(),
                device.owner_user_id.clone(),
            )
        });

        if let Some((name, device_type, mac_address, model, owner_user_id)) = device_data {
            self.modal = DeviceModal::Edit;
            self.input_name = Input::new(name);
            self.input_device_type = Input::new(device_type);
            self.input_tenant_id = Input::new("00000000-0000-0000-0000-000000000000".to_string());
            self.input_mac_address = Input::new(mac_address.unwrap_or_default());
            self.input_model = Input::new(model.unwrap_or_default());
            self.input_owner_user_id = Input::new(owner_user_id.unwrap_or_default());
            self.focused_field = 0;
            self.error = None;
        }
    }

    /// Open delete device modal
    pub fn open_delete_modal(&mut self) {
        if self.selected_device().is_some() {
            self.modal = DeviceModal::Delete;
            self.error = None;
        }
    }

    /// Close modal
    pub fn close_modal(&mut self) {
        self.modal = DeviceModal::None;
        self.clear_inputs();
        self.error = None;
    }

    /// Clear input fields
    fn clear_inputs(&mut self) {
        self.input_name = Input::default();
        self.input_device_type = Input::new("softphone".to_string());
        self.input_tenant_id = Input::new("00000000-0000-0000-0000-000000000000".to_string());
        self.input_mac_address = Input::default();
        self.input_model = Input::default();
        self.input_owner_user_id = Input::default();
    }

    /// Open user selection modal
    pub fn open_select_owner_modal(&mut self) {
        // Save current modal to return to after selection
        self.previous_modal = Some(self.modal.clone());
        self.modal = DeviceModal::SelectOwner;
        self.user_table_state.select(Some(0));
        self.error = None;
    }

    /// Get selected user for owner assignment
    pub fn selected_user(&self) -> Option<&crate::api::UserResponse> {
        self.user_table_state.selected().and_then(|i| self.users_list.get(i))
    }

    /// Move user selection up
    pub fn select_user_previous(&mut self) {
        if self.users_list.is_empty() {
            return;
        }
        let selected = self.user_table_state.selected().unwrap_or(0);
        if selected > 0 {
            self.user_table_state.select(Some(selected - 1));
        }
    }

    /// Move user selection down
    pub fn select_user_next(&mut self) {
        if self.users_list.is_empty() {
            return;
        }
        let selected = self.user_table_state.selected().unwrap_or(0);
        if selected < self.users_list.len() - 1 {
            self.user_table_state.select(Some(selected + 1));
        }
    }

    /// Get user display name by ID
    pub fn get_user_display_name(&self, user_id: &str) -> Option<String> {
        self.users_list.iter().find(|u| u.id == user_id).map(|u| {
            u.display_name
                .clone()
                .or_else(|| Some(u.username.clone()))
                .unwrap_or_else(|| u.username.clone())
        })
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

    // Render modal if open
    match state.modal {
        DeviceModal::Add => render_add_modal(f, theme, state, area),
        DeviceModal::Edit => render_edit_modal(f, theme, state, area),
        DeviceModal::Delete => render_delete_modal(f, theme, state, area),
        DeviceModal::SelectOwner => render_select_owner_modal(f, theme, state, area),
        DeviceModal::None => {}
    }
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

    let title = if state.loading {
        format!(" Devices (Loading...) ")
    } else {
        format!(" Devices ({}) (↑↓ to navigate) ", state.devices.len())
    };

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
            .title(title)
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
    let content = if state.loading {
        vec![
            Line::from(""),
            Line::from(""),
            Line::from(vec![Span::styled("  Loading devices...", theme.info())]),
        ]
    } else if let Some(device) = state.selected_device() {
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
                Span::raw(
                    device
                        .owner_user_id
                        .as_ref()
                        .and_then(|id| state.get_user_display_name(id))
                        .unwrap_or_else(|| "-".to_string()),
                ),
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
            Line::from(""),
            Line::from(""),
            Line::from(vec![Span::styled("Actions:", theme.title())]),
            Line::from(vec![
                Span::styled("  ", theme.muted()),
                Span::styled("a", theme.primary()),
                Span::styled(" - Add  ", theme.normal()),
                Span::styled("e", theme.primary()),
                Span::styled(" - Edit  ", theme.normal()),
                Span::styled("d", theme.primary()),
                Span::styled(" - Delete", theme.normal()),
            ]),
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

/// Render the add device modal
fn render_add_modal(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &DevicesViewState,
    area: Rect,
) {
    // Calculate centered popup area
    let popup_width = 70.min(area.width - 4);
    let popup_height = 26.min(area.height - 4);
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the area
    f.render_widget(Clear, popup_area);

    // Build form fields
    let name_focused = state.focused_field == 0;
    let type_focused = state.focused_field == 1;
    let tenant_focused = state.focused_field == 2;
    let mac_focused = state.focused_field == 3;
    let model_focused = state.focused_field == 4;
    let owner_focused = state.focused_field == 5;

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("Device Name: ", theme.muted()),
            Span::raw(state.input_name.value()),
            if name_focused { Span::raw(" █") } else { Span::raw("") },
        ]),
        Line::from(Span::styled("  (e.g., Alice Desk Phone)", theme.muted())),
        Line::from(""),
        Line::from(vec![
            Span::styled("Device Type: ", theme.muted()),
            Span::raw(state.input_device_type.value()),
            if type_focused { Span::raw(" █") } else { Span::raw("") },
        ]),
        Line::from(Span::styled("  (desk_phone, softphone, webrtc, conference_room, ata, gateway, mobile, other)", theme.muted())),
        Line::from(""),
        Line::from(vec![
            Span::styled("Tenant ID: ", theme.muted()),
            Span::raw(state.input_tenant_id.value()),
            if tenant_focused { Span::raw(" █") } else { Span::raw("") },
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("MAC Address: ", theme.muted()),
            Span::raw(state.input_mac_address.value()),
            if mac_focused { Span::raw(" █") } else { Span::raw("") },
        ]),
        Line::from(Span::styled("  (Optional, e.g., 00:11:22:33:44:55)", theme.muted())),
        Line::from(""),
        Line::from(vec![
            Span::styled("Model: ", theme.muted()),
            Span::raw(state.input_model.value()),
            if model_focused { Span::raw(" █") } else { Span::raw("") },
        ]),
        Line::from(Span::styled("  (Optional, e.g., Yealink T46S)", theme.muted())),
        Line::from(""),
        Line::from(vec![
            Span::styled("Owner User ID: ", theme.muted()),
            Span::raw(if state.input_owner_user_id.value().is_empty() {
                "(none)"
            } else {
                state.input_owner_user_id.value()
            }),
            if owner_focused { Span::raw(" █") } else { Span::raw("") },
        ]),
        Line::from(Span::styled("  (Optional, Tab here and press 's' to select from user list)", theme.muted())),
        Line::from(""),
        Line::from(Span::styled("Enter to submit • Esc to cancel • Tab to navigate", theme.muted())),
    ];

    // Add error if present
    let lines = if let Some(err) = &state.error {
        let mut lines = lines;
        lines.insert(lines.len() - 1, Line::from(""));
        lines.insert(lines.len() - 1, Line::from(Span::styled(err, theme.error())));
        lines
    } else {
        lines
    };

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .title("Add Device")
                .borders(Borders::ALL)
                .border_style(theme.primary()),
        );

    f.render_widget(paragraph, popup_area);
}

/// Render the edit device modal
fn render_edit_modal(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &DevicesViewState,
    area: Rect,
) {
    // Calculate centered popup area
    let popup_width = 70.min(area.width - 4);
    let popup_height = 26.min(area.height - 4);
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the area
    f.render_widget(Clear, popup_area);

    // Build form fields
    let name_focused = state.focused_field == 0;
    let type_focused = state.focused_field == 1;
    let tenant_focused = state.focused_field == 2;
    let mac_focused = state.focused_field == 3;
    let model_focused = state.focused_field == 4;
    let owner_focused = state.focused_field == 5;

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("Device Name: ", theme.muted()),
            Span::raw(state.input_name.value()),
            if name_focused { Span::raw(" █") } else { Span::raw("") },
        ]),
        Line::from(Span::styled("  (e.g., Alice Desk Phone)", theme.muted())),
        Line::from(""),
        Line::from(vec![
            Span::styled("Device Type: ", theme.muted()),
            Span::raw(state.input_device_type.value()),
            if type_focused { Span::raw(" █") } else { Span::raw("") },
        ]),
        Line::from(Span::styled("  (desk_phone, softphone, webrtc, conference_room, ata, gateway, mobile, other)", theme.muted())),
        Line::from(""),
        Line::from(vec![
            Span::styled("Tenant ID: ", theme.muted()),
            Span::raw(state.input_tenant_id.value()),
            if tenant_focused { Span::raw(" █") } else { Span::raw("") },
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("MAC Address: ", theme.muted()),
            Span::raw(state.input_mac_address.value()),
            if mac_focused { Span::raw(" █") } else { Span::raw("") },
        ]),
        Line::from(Span::styled("  (Optional, e.g., 00:11:22:33:44:55)", theme.muted())),
        Line::from(""),
        Line::from(vec![
            Span::styled("Model: ", theme.muted()),
            Span::raw(state.input_model.value()),
            if model_focused { Span::raw(" █") } else { Span::raw("") },
        ]),
        Line::from(Span::styled("  (Optional, e.g., Yealink T46S)", theme.muted())),
        Line::from(""),
        Line::from(vec![
            Span::styled("Owner User ID: ", theme.muted()),
            Span::raw(if state.input_owner_user_id.value().is_empty() {
                "(none)"
            } else {
                state.input_owner_user_id.value()
            }),
            if owner_focused { Span::raw(" █") } else { Span::raw("") },
        ]),
        Line::from(Span::styled("  (Optional, Tab here and press 's' to select from user list)", theme.muted())),
        Line::from(""),
        Line::from(Span::styled("Enter to submit • Esc to cancel • Tab to navigate", theme.muted())),
    ];

    // Add error if present
    let lines = if let Some(err) = &state.error {
        let mut lines = lines;
        lines.insert(lines.len() - 1, Line::from(""));
        lines.insert(lines.len() - 1, Line::from(Span::styled(err, theme.error())));
        lines
    } else {
        lines
    };

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .title("Edit Device")
                .borders(Borders::ALL)
                .border_style(theme.primary()),
        );

    f.render_widget(paragraph, popup_area);
}

/// Render the delete device modal
fn render_delete_modal(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &DevicesViewState,
    area: Rect,
) {
    // Calculate centered popup area
    let popup_width = 50.min(area.width - 4);
    let popup_height = 8.min(area.height - 4);
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the area
    f.render_widget(Clear, popup_area);

    let device_name = state
        .selected_device()
        .map(|d| d.name.as_str())
        .unwrap_or("");

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("Are you sure you want to delete device "),
            Span::styled(device_name, theme.error()),
            Span::raw("?"),
        ]),
        Line::from(""),
        Line::from(Span::styled("Enter to confirm • Esc to cancel", theme.muted())),
    ];

    // Add error if present
    let lines = if let Some(err) = &state.error {
        let mut lines = lines;
        lines.insert(lines.len() - 1, Line::from(""));
        lines.insert(lines.len() - 1, Line::from(Span::styled(err, theme.error())));
        lines
    } else {
        lines
    };

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .title("Delete Device")
                .borders(Borders::ALL)
                .border_style(theme.error()),
        );

    f.render_widget(paragraph, popup_area);
}

/// Render the user selection modal for assigning device owner
fn render_select_owner_modal(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &mut DevicesViewState,
    area: Rect,
) {
    // Calculate centered popup area
    let popup_width = 80.min(area.width - 4);
    let popup_height = 25.min(area.height - 4);
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the area
    f.render_widget(Clear, popup_area);

    // Create table header
    let header = Row::new(vec!["Username", "Display Name", "Email", "Role"])
        .style(theme.title())
        .bottom_margin(1);

    // Create table rows from users list
    let rows: Vec<Row> = state
        .users_list
        .iter()
        .map(|user| {
            Row::new(vec![
                user.username.clone(),
                user.display_name.clone().unwrap_or_else(|| "(none)".to_string()),
                user.email.clone().unwrap_or_else(|| "(none)".to_string()),
                user.role.clone(),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(30),
            Constraint::Percentage(20),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title("Select Device Owner")
            .borders(Borders::ALL)
            .border_style(theme.primary()),
    )
    .row_highlight_style(theme.selected())
    .highlight_symbol(">> ");

    f.render_stateful_widget(table, popup_area, &mut state.user_table_state);

    // Show instructions at the bottom
    let instructions_area = Rect::new(
        popup_area.x + 2,
        popup_area.y + popup_area.height - 2,
        popup_area.width - 4,
        1,
    );
    let instructions = Paragraph::new(Line::from(Span::styled(
        "↑/↓ Navigate • Enter to select • Esc to cancel",
        theme.muted(),
    )));
    f.render_widget(instructions, instructions_area);
}

/// Fetch devices from the API
pub async fn fetch_devices(api_client: &crate::api::ApiClient, state: &mut DevicesViewState) {
    state.loading = true;
    state.error = None;

    // Fetch users list for owner name lookup
    if let Ok(users) = api_client.list_users().await {
        state.users_list = users;
    }

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

/// Handle keyboard events for device modal
pub async fn handle_device_action(
    state: &mut DevicesViewState,
    api_client: &crate::api::ApiClient,
    event: &crate::tui::event::Event,
) -> bool {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tui_input::backend::crossterm::EventHandler;

    // Only handle events when modal is open
    if state.modal == DeviceModal::None {
        return false;
    }

    match event {
        crate::tui::event::Event::Key(KeyEvent { code, modifiers, .. }) => {
            match code {
                KeyCode::Esc if *modifiers == KeyModifiers::NONE => {
                    // If in SelectOwner modal, return to previous modal instead of closing
                    if state.modal == DeviceModal::SelectOwner {
                        if let Some(prev) = state.previous_modal.take() {
                            state.modal = prev;
                        } else {
                            state.close_modal();
                        }
                    } else {
                        state.close_modal();
                    }
                    return true;
                }
                KeyCode::Tab if *modifiers == KeyModifiers::NONE => {
                    if state.modal != DeviceModal::Delete && state.modal != DeviceModal::SelectOwner {
                        state.focused_field = (state.focused_field + 1) % 6;
                    }
                    return true;
                }
                KeyCode::BackTab => {
                    if state.modal != DeviceModal::Delete && state.modal != DeviceModal::SelectOwner {
                        state.focused_field = (state.focused_field + 5) % 6;
                    }
                    return true;
                }
                KeyCode::Enter if *modifiers == KeyModifiers::NONE => {
                    match state.modal {
                        DeviceModal::Add => {
                            create_device(api_client, state).await;
                        }
                        DeviceModal::Edit => {
                            update_device(api_client, state).await;
                        }
                        DeviceModal::Delete => {
                            delete_device(api_client, state).await;
                        }
                        DeviceModal::SelectOwner => {
                            // Select the currently highlighted user and set it as owner
                            if let Some(user) = state.selected_user() {
                                state.input_owner_user_id = tui_input::Input::new(user.id.clone());
                            }
                            // Return to the previous modal (Add or Edit) instead of closing
                            if let Some(prev) = state.previous_modal.take() {
                                state.modal = prev;
                            } else {
                                state.close_modal();
                            }
                        }
                        DeviceModal::None => {}
                    }
                    return true;
                }
                KeyCode::Up if *modifiers == KeyModifiers::NONE => {
                    if state.modal == DeviceModal::SelectOwner {
                        state.select_user_previous();
                        return true;
                    }
                    return false;
                }
                KeyCode::Down if *modifiers == KeyModifiers::NONE => {
                    if state.modal == DeviceModal::SelectOwner {
                        state.select_user_next();
                        return true;
                    }
                    return false;
                }
                _ => {
                    // Special handling for 's' key on owner field (field 5)
                    if let KeyCode::Char('s') | KeyCode::Char('S') = code {
                        if *modifiers == KeyModifiers::NONE && (state.modal == DeviceModal::Add || state.modal == DeviceModal::Edit) && state.focused_field == 5 {
                            // Open user selection modal
                            select_owner(api_client, state).await;
                            return true;
                        }
                    }

                    // Handle text input for the focused field (not for Delete modal or SelectOwner modal)
                    if state.modal != DeviceModal::Delete && state.modal != DeviceModal::SelectOwner {
                        let input = match state.focused_field {
                            0 => &mut state.input_name,
                            1 => &mut state.input_device_type,
                            2 => &mut state.input_tenant_id,
                            3 => &mut state.input_mac_address,
                            4 => &mut state.input_model,
                            5 => &mut state.input_owner_user_id,
                            _ => return false,
                        };

                        if let crate::tui::event::Event::Key(key_event) = event {
                            input.handle_event(&crossterm::event::Event::Key(*key_event));
                        }
                    }
                    return true;
                }
            }
        }
        _ => false,
    }
}

/// Create a new device
pub async fn create_device(api_client: &crate::api::ApiClient, state: &mut DevicesViewState) {
    let name = state.input_name.value().to_string();
    let device_type = state.input_device_type.value().to_string();
    let tenant_id = state.input_tenant_id.value().to_string();
    let mac_address = state.input_mac_address.value().to_string();
    let model = state.input_model.value().to_string();
    let owner_user_id = state.input_owner_user_id.value().to_string();

    if name.is_empty() {
        state.error = Some("Device name is required".to_string());
        return;
    }

    if device_type.is_empty() {
        state.error = Some("Device type is required".to_string());
        return;
    }

    // Validate device type (must match server's parse_device_type expectations)
    if !matches!(
        device_type.as_str(),
        "desk_phone" | "softphone" | "webrtc" | "conference_room" | "ata" | "gateway" | "mobile" | "other"
    ) {
        state.error = Some("Invalid device type (use: desk_phone, softphone, webrtc, conference_room, ata, gateway, mobile, other)".to_string());
        return;
    }

    #[derive(serde::Serialize)]
    struct CreateDeviceRequest {
        name: String,
        device_type: String,
        tenant_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        mac_address: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        owner_user_id: Option<String>,
    }

    let request = CreateDeviceRequest {
        name,
        device_type,
        tenant_id,
        mac_address: if mac_address.is_empty() {
            None
        } else {
            Some(mac_address)
        },
        model: if model.is_empty() { None } else { Some(model) },
        owner_user_id: if owner_user_id.is_empty() {
            None
        } else {
            Some(owner_user_id)
        },
    };

    match api_client.create_device(&request).await {
        Ok(_) => {
            state.close_modal();
            // Refresh device list
            fetch_devices(api_client, state).await;
        }
        Err(e) => {
            state.error = Some(format!("Failed to create device: {}", e));
        }
    }
}

/// Update an existing device
pub async fn update_device(api_client: &crate::api::ApiClient, state: &mut DevicesViewState) {
    let Some(device) = state.selected_device() else {
        state.error = Some("No device selected".to_string());
        return;
    };

    let device_id = device.id.clone();
    let name = state.input_name.value().to_string();
    let device_type = state.input_device_type.value().to_string();
    let mac_address = state.input_mac_address.value().to_string();
    let model = state.input_model.value().to_string();
    let owner_user_id = state.input_owner_user_id.value().to_string();

    if name.is_empty() {
        state.error = Some("Device name is required".to_string());
        return;
    }

    if device_type.is_empty() {
        state.error = Some("Device type is required".to_string());
        return;
    }

    // Validate device type (must match server's parse_device_type expectations)
    if !matches!(
        device_type.as_str(),
        "desk_phone" | "softphone" | "webrtc" | "conference_room" | "ata" | "gateway" | "mobile" | "other"
    ) {
        state.error = Some("Invalid device type (use: desk_phone, softphone, webrtc, conference_room, ata, gateway, mobile, other)".to_string());
        return;
    }

    #[derive(serde::Serialize)]
    struct UpdateDeviceRequest {
        name: String,
        device_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        mac_address: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        owner_user_id: Option<String>,
    }

    let request = UpdateDeviceRequest {
        name,
        device_type,
        mac_address: if mac_address.is_empty() {
            None
        } else {
            Some(mac_address)
        },
        model: if model.is_empty() { None } else { Some(model) },
        owner_user_id: if owner_user_id.is_empty() {
            None
        } else {
            Some(owner_user_id)
        },
    };

    match api_client.update_device(&device_id, &request).await {
        Ok(_) => {
            state.close_modal();
            // Refresh device list
            fetch_devices(api_client, state).await;
        }
        Err(e) => {
            state.error = Some(format!("Failed to update device: {}", e));
        }
    }
}

/// Delete a device
pub async fn delete_device(api_client: &crate::api::ApiClient, state: &mut DevicesViewState) {
    let Some(device) = state.selected_device() else {
        state.error = Some("No device selected".to_string());
        return;
    };

    let device_id = device.id.clone();

    match api_client.delete_device(&device_id).await {
        Ok(_) => {
            state.close_modal();
            // Refresh device list
            fetch_devices(api_client, state).await;
        }
        Err(e) => {
            state.error = Some(format!("Failed to delete device: {}", e));
        }
    }
}

/// Open the user selection modal for device owner assignment
pub async fn select_owner(api_client: &crate::api::ApiClient, state: &mut DevicesViewState) {
    // Fetch all users from the API
    match api_client.list_users().await {
        Ok(users) => {
            state.users_list = users;
            state.open_select_owner_modal();
        }
        Err(e) => {
            state.error = Some(format!("Failed to fetch users: {}", e));
        }
    }
}
