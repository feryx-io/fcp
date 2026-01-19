//! Phone numbers view showing phone numbers and details

use crate::api::PhoneNumberResponse;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState},
    Frame,
};
use tui_input::Input;

/// Modal type for phone number operations
#[derive(Debug, Clone, PartialEq)]
pub enum PhoneNumberModal {
    None,
    Add,
    Edit,
    Delete,
}

/// State for the phone numbers view
pub struct PhoneNumbersViewState {
    /// List of phone numbers
    pub phone_numbers: Vec<PhoneNumberResponse>,
    /// Table state for selection
    pub table_state: TableState,
    /// Last error message
    pub error: Option<String>,
    /// Loading state
    pub loading: bool,
    /// Active modal
    pub modal: PhoneNumberModal,
    /// Input field: E.164 phone number
    pub input_e164: Input,
    /// Input field: tenant ID (defaults to nil UUID)
    pub input_tenant_id: Input,
    /// Input field: CNAM (caller ID name)
    pub input_cnam: Input,
    /// Input field: E911 callback number
    pub input_e911_callback: Input,
    /// Input field: E911 address line 1
    pub input_e911_address1: Input,
    /// Input field: E911 address line 2
    pub input_e911_address2: Input,
    /// Input field: E911 city
    pub input_e911_city: Input,
    /// Input field: E911 state
    pub input_e911_state: Input,
    /// Input field: E911 postal code
    pub input_e911_postal: Input,
    /// Input field: Recording enabled (0=disabled, 1=enabled)
    pub input_recording_enabled: usize,
    /// Input field: Block anonymous (0=no, 1=yes)
    pub input_block_anonymous: usize,
    /// Which input field is focused (0-10)
    pub focused_field: usize,
}

impl Default for PhoneNumbersViewState {
    fn default() -> Self {
        let mut table_state = TableState::default();
        table_state.select(Some(0));

        Self {
            phone_numbers: Vec::new(),
            table_state,
            error: None,
            loading: false,
            modal: PhoneNumberModal::None,
            input_e164: Input::default(),
            input_tenant_id: Input::new("00000000-0000-0000-0000-000000000000".to_string()),
            input_cnam: Input::default(),
            input_e911_callback: Input::default(),
            input_e911_address1: Input::default(),
            input_e911_address2: Input::default(),
            input_e911_city: Input::default(),
            input_e911_state: Input::default(),
            input_e911_postal: Input::default(),
            input_recording_enabled: 0,
            input_block_anonymous: 0,
            focused_field: 0,
        }
    }
}

impl PhoneNumbersViewState {
    /// Get currently selected phone number
    pub fn selected_phone_number(&self) -> Option<&PhoneNumberResponse> {
        self.table_state
            .selected()
            .and_then(|i| self.phone_numbers.get(i))
    }

    /// Move selection up
    pub fn select_previous(&mut self) {
        if self.phone_numbers.is_empty() {
            return;
        }

        let selected = self.table_state.selected().unwrap_or(0);
        if selected > 0 {
            self.table_state.select(Some(selected - 1));
        }
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        if self.phone_numbers.is_empty() {
            return;
        }

        let selected = self.table_state.selected().unwrap_or(0);
        if selected < self.phone_numbers.len() - 1 {
            self.table_state.select(Some(selected + 1));
        }
    }

    /// Update phone numbers list
    pub fn set_phone_numbers(&mut self, phone_numbers: Vec<PhoneNumberResponse>) {
        self.phone_numbers = phone_numbers;
        // Reset selection to first item if we had no selection or it's out of bounds
        if self.table_state.selected().is_none()
            || self.table_state.selected().unwrap() >= self.phone_numbers.len()
        {
            self.table_state.select(if self.phone_numbers.is_empty() {
                None
            } else {
                Some(0)
            });
        }
    }

    /// Open add phone number modal
    pub fn open_add_modal(&mut self) {
        self.modal = PhoneNumberModal::Add;
        self.clear_inputs();
        self.focused_field = 0;
        self.error = None;
    }

    /// Open edit phone number modal
    pub fn open_edit_modal(&mut self) {
        // Get the data we need before mutating self
        let phone_data = self.selected_phone_number().map(|phone| {
            (
                phone.e164.clone(),
                phone.cnam.clone(),
                phone.e911_callback.clone(),
                phone.e911_address.clone(),
                phone.recording_enabled,
                phone.block_anonymous,
            )
        });

        if let Some((e164, cnam, e911_callback, e911_address, recording, block_anon)) = phone_data {
            self.modal = PhoneNumberModal::Edit;
            self.input_e164 = Input::new(e164);
            self.input_tenant_id = Input::new("00000000-0000-0000-0000-000000000000".to_string());
            self.input_cnam = Input::new(cnam.unwrap_or_default());
            self.input_e911_callback = Input::new(e911_callback.unwrap_or_default());

            // Populate E911 address fields
            if let Some(addr) = e911_address {
                self.input_e911_address1 = Input::new(addr.address1);
                self.input_e911_address2 = Input::new(addr.address2.unwrap_or_default());
                self.input_e911_city = Input::new(addr.city);
                self.input_e911_state = Input::new(addr.state);
                self.input_e911_postal = Input::new(addr.postal_code);
            } else {
                self.input_e911_address1 = Input::default();
                self.input_e911_address2 = Input::default();
                self.input_e911_city = Input::default();
                self.input_e911_state = Input::default();
                self.input_e911_postal = Input::default();
            }

            self.input_recording_enabled = if recording { 1 } else { 0 };
            self.input_block_anonymous = if block_anon { 1 } else { 0 };
            self.focused_field = 0;
            self.error = None;
        }
    }

    /// Open delete phone number modal
    pub fn open_delete_modal(&mut self) {
        if self.selected_phone_number().is_some() {
            self.modal = PhoneNumberModal::Delete;
            self.error = None;
        }
    }

    /// Close modal
    pub fn close_modal(&mut self) {
        self.modal = PhoneNumberModal::None;
        self.clear_inputs();
        self.error = None;
    }

    /// Clear input fields
    fn clear_inputs(&mut self) {
        self.input_e164 = Input::default();
        self.input_tenant_id = Input::new("00000000-0000-0000-0000-000000000000".to_string());
        self.input_cnam = Input::default();
        self.input_e911_callback = Input::default();
        self.input_e911_address1 = Input::default();
        self.input_e911_address2 = Input::default();
        self.input_e911_city = Input::default();
        self.input_e911_state = Input::default();
        self.input_e911_postal = Input::default();
        self.input_recording_enabled = 0;
        self.input_block_anonymous = 0;
    }
}

/// Render the phone numbers view
pub fn render(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &mut PhoneNumbersViewState,
    area: Rect,
    deployment_mode: fcp_core::DeploymentMode,
) {
    // Split into table and details panel
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(55), // Phone numbers table
            Constraint::Percentage(45), // Details panel
        ])
        .split(area);

    render_phone_numbers_table(f, theme, state, chunks[0]);
    render_details_panel(f, theme, state, chunks[1], deployment_mode);

    // Render modal if open
    match state.modal {
        PhoneNumberModal::Add => render_add_modal(f, theme, state, area),
        PhoneNumberModal::Edit => render_edit_modal(f, theme, state, area),
        PhoneNumberModal::Delete => render_delete_modal(f, theme, state, area),
        PhoneNumberModal::None => {}
    }
}

/// Render the phone numbers table
fn render_phone_numbers_table(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &mut PhoneNumbersViewState,
    area: Rect,
) {
    let header = Row::new(vec!["Phone Number", "State", "CNAM", "E911", "Recording"])
        .style(theme.title())
        .bottom_margin(1);

    let rows: Vec<Row> = state
        .phone_numbers
        .iter()
        .map(|phone| {
            // Format E911 status
            let e911 = if phone.e911_address.is_some() {
                "✓"
            } else {
                "✗"
            };

            // Format recording status
            let recording = if phone.recording_enabled { "✓" } else { "✗" };

            // Format CNAM
            let cnam = phone.cnam.as_deref().unwrap_or("-");

            Row::new(vec![
                Cell::from(phone.e164.clone()),
                Cell::from(phone.state.clone()),
                Cell::from(cnam),
                Cell::from(e911),
                Cell::from(recording),
            ])
        })
        .collect();

    let title = if state.loading {
        format!(" Phone Numbers (Loading...) ")
    } else {
        format!(" Phone Numbers ({}) (↑↓ to navigate) ", state.phone_numbers.len())
    };

    let table = Table::new(
        rows,
        [
            Constraint::Length(16), // Phone number
            Constraint::Length(12), // State
            Constraint::Length(20), // CNAM
            Constraint::Length(6),  // E911
            Constraint::Length(10), // Recording
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
    state: &PhoneNumbersViewState,
    area: Rect,
    deployment_mode: fcp_core::DeploymentMode,
) {
    let content = if state.loading {
        vec![
            Line::from(""),
            Line::from(""),
            Line::from(vec![Span::styled("  Loading phone numbers...", theme.info())]),
        ]
    } else if let Some(phone) = state.selected_phone_number() {
        let mut lines = vec![
            Line::from(vec![
                Span::styled("Phone Number: ", theme.muted()),
                Span::raw(&phone.e164),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("ID: ", theme.muted()),
                Span::raw(format_short_id(&phone.id)),
            ]),
            Line::from(vec![
                Span::styled("State: ", theme.muted()),
                Span::raw(&phone.state),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Country Code: ", theme.muted()),
                Span::raw(&phone.country_code),
            ]),
            Line::from(vec![
                Span::styled("NPA-NXX-Line: ", theme.muted()),
                Span::raw(format!("{}-{}-{}", phone.npa, phone.nxx, phone.line)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("CNAM: ", theme.muted()),
                Span::raw(phone.cnam.as_deref().unwrap_or("-")),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("E911 Address: ", theme.muted()),
                Span::raw(if phone.e911_address.is_some() {
                    "Configured"
                } else {
                    "Not configured"
                }),
            ]),
            if let Some(e911) = &phone.e911_address {
                Line::from(vec![
                    Span::raw("  "),
                    Span::raw(format!("{}", e911.address1)),
                ])
            } else {
                Line::from("")
            },
            if let Some(e911) = &phone.e911_address {
                if let Some(addr2) = &e911.address2 {
                    Line::from(vec![Span::raw("  "), Span::raw(addr2)])
                } else {
                    Line::from("")
                }
            } else {
                Line::from("")
            },
            if let Some(e911) = &phone.e911_address {
                Line::from(vec![
                    Span::raw("  "),
                    Span::raw(format!("{}, {} {}", e911.city, e911.state, e911.postal_code)),
                ])
            } else {
                Line::from("")
            },
            Line::from(""),
            Line::from(vec![
                Span::styled("E911 Callback: ", theme.muted()),
                Span::raw(phone.e911_callback.as_deref().unwrap_or("-")),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Recording: ", theme.muted()),
                Span::raw(if phone.recording_enabled {
                    "Enabled"
                } else {
                    "Disabled"
                }),
            ]),
            Line::from(vec![
                Span::styled("Block Anonymous: ", theme.muted()),
                Span::raw(if phone.block_anonymous {
                    "Yes"
                } else {
                    "No"
                }),
            ]),
        ];

        // Only show carrier data in Carrier/ServiceProvider modes
        if deployment_mode.has_carrier_features() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("Carrier: ", theme.muted()),
                Span::raw(phone.carrier.as_deref().unwrap_or("-")),
            ]));
            lines.push(Line::from(vec![
                Span::styled("LRN: ", theme.muted()),
                Span::raw(phone.lrn.as_deref().unwrap_or("-")),
            ]));
            lines.push(Line::from(vec![
                Span::styled("Rate Center: ", theme.muted()),
                Span::raw(phone.rate_center.as_deref().unwrap_or("-")),
            ]));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled("Actions:", theme.title())]));
        lines.push(Line::from(vec![
            Span::styled("  ", theme.muted()),
            Span::styled("a", theme.primary()),
            Span::styled(" - Add  ", theme.normal()),
            Span::styled("e", theme.primary()),
            Span::styled(" - Edit  ", theme.normal()),
            Span::styled("d", theme.primary()),
            Span::styled(" - Delete", theme.normal()),
        ]));

        lines
    } else {
        vec![Line::from("No phone number selected")]
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

/// Render the add phone number modal
fn render_add_modal(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &PhoneNumbersViewState,
    area: Rect,
) {
    // Calculate centered popup area
    let popup_width = 60.min(area.width - 4);
    let popup_height = 14.min(area.height - 4);
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the area
    f.render_widget(Clear, popup_area);

    // Build form fields
    let e164_focused = state.focused_field == 0;
    let tenant_focused = state.focused_field == 1;
    let cnam_focused = state.focused_field == 2;

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("E.164 Number: ", theme.muted()),
            Span::raw(state.input_e164.value()),
            if e164_focused { Span::raw(" █") } else { Span::raw("") },
        ]),
        Line::from(Span::styled("  (e.g., +15551234567)", theme.muted())),
        Line::from(""),
        Line::from(vec![
            Span::styled("Tenant ID: ", theme.muted()),
            Span::raw(state.input_tenant_id.value()),
            if tenant_focused { Span::raw(" █") } else { Span::raw("") },
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("CNAM: ", theme.muted()),
            Span::raw(state.input_cnam.value()),
            if cnam_focused { Span::raw(" █") } else { Span::raw("") },
        ]),
        Line::from(Span::styled("  (Optional caller ID name)", theme.muted())),
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
                .title("Add Phone Number")
                .borders(Borders::ALL)
                .border_style(theme.primary()),
        );

    f.render_widget(paragraph, popup_area);
}

/// Render the edit phone number modal
fn render_edit_modal(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &PhoneNumbersViewState,
    area: Rect,
) {
    // Calculate centered popup area - larger for more fields
    let popup_width = 70.min(area.width - 4);
    let popup_height = 30.min(area.height - 4);
    let popup_x = (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the area
    f.render_widget(Clear, popup_area);

    // Build form fields
    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("E.164 Number: ", theme.muted()),
            Span::raw(state.input_e164.value()),
        ]),
        Line::from(Span::styled("  (Read-only)", theme.muted())),
        Line::from(""),
        Line::from(vec![
            Span::styled("CNAM: ", theme.muted()),
            Span::raw(state.input_cnam.value()),
            if state.focused_field == 0 { Span::raw(" █") } else { Span::raw("") },
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("E911 Callback: ", theme.muted()),
            Span::raw(state.input_e911_callback.value()),
            if state.focused_field == 1 { Span::raw(" █") } else { Span::raw("") },
        ]),
        Line::from(""),
        Line::from(Span::styled("E911 Address:", theme.title())),
        Line::from(vec![
            Span::styled("  Address 1: ", theme.muted()),
            Span::raw(state.input_e911_address1.value()),
            if state.focused_field == 2 { Span::raw(" █") } else { Span::raw("") },
        ]),
        Line::from(vec![
            Span::styled("  Address 2: ", theme.muted()),
            Span::raw(state.input_e911_address2.value()),
            if state.focused_field == 3 { Span::raw(" █") } else { Span::raw("") },
        ]),
        Line::from(vec![
            Span::styled("  City: ", theme.muted()),
            Span::raw(state.input_e911_city.value()),
            if state.focused_field == 4 { Span::raw(" █") } else { Span::raw("") },
        ]),
        Line::from(vec![
            Span::styled("  State: ", theme.muted()),
            Span::raw(state.input_e911_state.value()),
            if state.focused_field == 5 { Span::raw(" █") } else { Span::raw("") },
        ]),
        Line::from(vec![
            Span::styled("  Postal Code: ", theme.muted()),
            Span::raw(state.input_e911_postal.value()),
            if state.focused_field == 6 { Span::raw(" █") } else { Span::raw("") },
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Recording Enabled: ", theme.muted()),
            Span::raw(if state.input_recording_enabled == 1 { "Yes" } else { "No" }),
            if state.focused_field == 7 {
                Span::styled(" ◄ Space to toggle", theme.primary())
            } else {
                Span::raw("")
            },
        ]),
        Line::from(vec![
            Span::styled("Block Anonymous: ", theme.muted()),
            Span::raw(if state.input_block_anonymous == 1 { "Yes" } else { "No" }),
            if state.focused_field == 8 {
                Span::styled(" ◄ Space to toggle", theme.primary())
            } else {
                Span::raw("")
            },
        ]),
        Line::from(""),
        Line::from(Span::styled("Enter to submit • Esc to cancel • Tab to navigate • Space to toggle", theme.muted())),
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
                .title("Edit Phone Number")
                .borders(Borders::ALL)
                .border_style(theme.primary()),
        );

    f.render_widget(paragraph, popup_area);
}

/// Render the delete phone number modal
fn render_delete_modal(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &PhoneNumbersViewState,
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

    let phone_number = state
        .selected_phone_number()
        .map(|p| p.e164.as_str())
        .unwrap_or("");

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("Are you sure you want to delete phone number "),
            Span::styled(phone_number, theme.error()),
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
                .title("Delete Phone Number")
                .borders(Borders::ALL)
                .border_style(theme.error()),
        );

    f.render_widget(paragraph, popup_area);
}

/// Fetch phone numbers from the API
pub async fn fetch_phone_numbers(api_client: &crate::api::ApiClient, state: &mut PhoneNumbersViewState) {
    state.loading = true;
    state.error = None;

    match api_client.list_phone_numbers(None).await {
        Ok(phone_numbers) => {
            state.set_phone_numbers(phone_numbers);
            state.loading = false;
        }
        Err(e) => {
            state.error = Some(format!("Failed to fetch phone numbers: {}", e));
            state.loading = false;
        }
    }
}

/// Handle navigation events
pub fn handle_navigation(state: &mut PhoneNumbersViewState, event: &crate::tui::event::Event) {
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

/// Handle keyboard events for phone number modal
pub async fn handle_phone_number_action(
    state: &mut PhoneNumbersViewState,
    api_client: &crate::api::ApiClient,
    event: &crate::tui::event::Event,
) -> bool {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tui_input::backend::crossterm::EventHandler;

    // Only handle events when modal is open
    if state.modal == PhoneNumberModal::None {
        return false;
    }

    match event {
        crate::tui::event::Event::Key(KeyEvent { code, modifiers, .. }) => {
            match code {
                KeyCode::Esc if *modifiers == KeyModifiers::NONE => {
                    state.close_modal();
                    return true;
                }
                KeyCode::Tab if *modifiers == KeyModifiers::NONE => {
                    if state.modal != PhoneNumberModal::Delete {
                        let max_field = if state.modal == PhoneNumberModal::Add { 3 } else { 9 };
                        state.focused_field = (state.focused_field + 1) % max_field;
                    }
                    return true;
                }
                KeyCode::BackTab => {
                    if state.modal != PhoneNumberModal::Delete {
                        let max_field = if state.modal == PhoneNumberModal::Add { 3 } else { 9 };
                        state.focused_field = (state.focused_field + max_field - 1) % max_field;
                    }
                    return true;
                }
                KeyCode::Char(' ') if *modifiers == KeyModifiers::NONE => {
                    // Handle space key for boolean toggles in edit mode
                    if state.modal == PhoneNumberModal::Edit {
                        match state.focused_field {
                            7 => {
                                state.input_recording_enabled = 1 - state.input_recording_enabled;
                                return true;
                            }
                            8 => {
                                state.input_block_anonymous = 1 - state.input_block_anonymous;
                                return true;
                            }
                            _ => {}
                        }
                    }
                }
                KeyCode::Enter if *modifiers == KeyModifiers::NONE => {
                    match state.modal {
                        PhoneNumberModal::Add => {
                            create_phone_number(api_client, state).await;
                        }
                        PhoneNumberModal::Edit => {
                            update_phone_number(api_client, state).await;
                        }
                        PhoneNumberModal::Delete => {
                            delete_phone_number(api_client, state).await;
                        }
                        PhoneNumberModal::None => {}
                    }
                    return true;
                }
                _ => {}
            }

            // Handle text input for the focused field (not for Delete modal, not for boolean fields)
            if state.modal != PhoneNumberModal::Delete {
                let is_boolean_field = state.modal == PhoneNumberModal::Edit
                    && (state.focused_field == 7 || state.focused_field == 8);

                if !is_boolean_field {
                    let input = match (state.modal.clone(), state.focused_field) {
                        (PhoneNumberModal::Add, 0) => Some(&mut state.input_e164),
                        (PhoneNumberModal::Add, 1) => Some(&mut state.input_tenant_id),
                        (PhoneNumberModal::Add, 2) => Some(&mut state.input_cnam),
                        (PhoneNumberModal::Edit, 0) => Some(&mut state.input_cnam),
                        (PhoneNumberModal::Edit, 1) => Some(&mut state.input_e911_callback),
                        (PhoneNumberModal::Edit, 2) => Some(&mut state.input_e911_address1),
                        (PhoneNumberModal::Edit, 3) => Some(&mut state.input_e911_address2),
                        (PhoneNumberModal::Edit, 4) => Some(&mut state.input_e911_city),
                        (PhoneNumberModal::Edit, 5) => Some(&mut state.input_e911_state),
                        (PhoneNumberModal::Edit, 6) => Some(&mut state.input_e911_postal),
                        _ => None,
                    };

                    if let Some(input) = input {
                        if let crate::tui::event::Event::Key(key_event) = event {
                            input.handle_event(&crossterm::event::Event::Key(*key_event));
                        }
                    }
                }
            }
            return true;
        }
        _ => false,
    }
}

/// Create a new phone number
pub async fn create_phone_number(api_client: &crate::api::ApiClient, state: &mut PhoneNumbersViewState) {
    let e164 = state.input_e164.value().to_string();
    let tenant_id = state.input_tenant_id.value().to_string();
    let cnam = state.input_cnam.value().to_string();

    if e164.is_empty() {
        state.error = Some("E.164 number is required".to_string());
        return;
    }

    if !e164.starts_with('+') {
        state.error = Some("E.164 number must start with +".to_string());
        return;
    }

    #[derive(serde::Serialize)]
    struct CreatePhoneNumberRequest {
        e164: String,
        tenant_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cnam: Option<String>,
    }

    let request = CreatePhoneNumberRequest {
        e164,
        tenant_id,
        cnam: if cnam.is_empty() { None } else { Some(cnam) },
    };

    match api_client.create_phone_number(&request).await {
        Ok(_) => {
            state.close_modal();
            // Refresh phone number list
            fetch_phone_numbers(api_client, state).await;
        }
        Err(e) => {
            state.error = Some(format!("Failed to create phone number: {}", e));
        }
    }
}

/// Update an existing phone number
pub async fn update_phone_number(api_client: &crate::api::ApiClient, state: &mut PhoneNumbersViewState) {
    let Some(phone) = state.selected_phone_number() else {
        state.error = Some("No phone number selected".to_string());
        return;
    };

    let phone_id = phone.id.clone();
    let cnam = state.input_cnam.value().to_string();
    let e911_callback = state.input_e911_callback.value().to_string();
    let e911_address1 = state.input_e911_address1.value().to_string();
    let e911_address2 = state.input_e911_address2.value().to_string();
    let e911_city = state.input_e911_city.value().to_string();
    let e911_state = state.input_e911_state.value().to_string();
    let e911_postal = state.input_e911_postal.value().to_string();

    #[derive(serde::Serialize)]
    struct E911Address {
        address1: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        address2: Option<String>,
        city: String,
        state: String,
        postal_code: String,
        country: String,
        validated: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        validated_at: Option<String>,
    }

    #[derive(serde::Serialize)]
    struct UpdatePhoneNumberRequest {
        #[serde(skip_serializing_if = "Option::is_none")]
        cnam: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        e911_callback: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        e911_address: Option<E911Address>,
        #[serde(skip_serializing_if = "Option::is_none")]
        recording_enabled: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        block_anonymous: Option<bool>,
    }

    // Build E911 address if we have the required fields
    let e911_address = if !e911_address1.is_empty() && !e911_city.is_empty() && !e911_state.is_empty() && !e911_postal.is_empty() {
        Some(E911Address {
            address1: e911_address1,
            address2: if e911_address2.is_empty() { None } else { Some(e911_address2) },
            city: e911_city,
            state: e911_state,
            postal_code: e911_postal,
            country: "US".to_string(),
            validated: false, // User-entered addresses are not pre-validated
            validated_at: None,
        })
    } else {
        None
    };

    let request = UpdatePhoneNumberRequest {
        cnam: if cnam.is_empty() { None } else { Some(cnam) },
        e911_callback: if e911_callback.is_empty() { None } else { Some(e911_callback) },
        e911_address,
        recording_enabled: Some(state.input_recording_enabled == 1),
        block_anonymous: Some(state.input_block_anonymous == 1),
    };

    match api_client.update_phone_number(&phone_id, &request).await {
        Ok(_) => {
            state.close_modal();
            // Refresh phone number list
            fetch_phone_numbers(api_client, state).await;
        }
        Err(e) => {
            state.error = Some(format!("Failed to update phone number: {}", e));
        }
    }
}

/// Delete a phone number
pub async fn delete_phone_number(api_client: &crate::api::ApiClient, state: &mut PhoneNumbersViewState) {
    let Some(phone) = state.selected_phone_number() else {
        state.error = Some("No phone number selected".to_string());
        return;
    };

    let phone_id = phone.id.clone();

    match api_client.delete_phone_number(&phone_id).await {
        Ok(_) => {
            state.close_modal();
            // Refresh phone number list
            fetch_phone_numbers(api_client, state).await;
        }
        Err(e) => {
            state.error = Some(format!("Failed to delete phone number: {}", e));
        }
    }
}
