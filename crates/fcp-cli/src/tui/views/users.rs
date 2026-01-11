//! Users view showing user list and details

use crate::api::{CreateUserRequest, UpdateUserRequest, UserResponse};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState},
    Frame,
};
use tui_input::Input;

/// Modal type for user operations
#[derive(Debug, Clone, PartialEq)]
pub enum UserModal {
    None,
    Add,
    Edit,
    Delete,
}

/// State for the users view
pub struct UsersViewState {
    /// List of users
    pub users: Vec<UserResponse>,
    /// Table state for selection
    pub table_state: TableState,
    /// Last error message
    pub error: Option<String>,
    /// Loading state
    pub loading: bool,
    /// Active modal
    pub modal: UserModal,
    /// Input field: username
    pub input_username: Input,
    /// Input field: display name
    pub input_display_name: Input,
    /// Input field: password
    pub input_password: Input,
    /// Whether user is enabled (for add/edit)
    pub input_enabled: bool,

    // ===== Enterprise input fields =====
    /// Input field: email
    pub input_email: Input,
    /// Input field: phone number
    pub input_phone_number: Input,
    /// Input field: mobile number
    pub input_mobile_number: Input,
    /// Input field: PIN (voicemail)
    pub input_pin: Input,
    /// Input field: extension
    pub input_extension: Input,
    /// Input field: department
    pub input_department: Input,
    /// Input field: job title
    pub input_job_title: Input,
    /// Input field: location
    pub input_location: Input,
    /// Input field: language
    pub input_language: Input,
    /// Input field: timezone
    pub input_timezone: Input,
    /// Input field: account status
    pub input_account_status: Input,
    /// Input field: emergency contact name
    pub input_emergency_contact_name: Input,
    /// Input field: emergency contact phone
    pub input_emergency_contact_phone: Input,

    /// Which input field is focused (0-15)
    pub focused_field: usize,
    /// Total number of input fields
    pub total_fields: usize,
}

impl Default for UsersViewState {
    fn default() -> Self {
        let mut table_state = TableState::default();
        table_state.select(Some(0));

        Self {
            users: Vec::new(),
            table_state,
            error: None,
            loading: false,
            modal: UserModal::None,
            input_username: Input::default(),
            input_display_name: Input::default(),
            input_password: Input::default(),
            input_enabled: true,
            // Enterprise fields
            input_email: Input::default(),
            input_phone_number: Input::default(),
            input_mobile_number: Input::default(),
            input_pin: Input::default(),
            input_extension: Input::default(),
            input_department: Input::default(),
            input_job_title: Input::default(),
            input_location: Input::default(),
            input_language: Input::new("en-US".to_string()),
            input_timezone: Input::new("UTC".to_string()),
            input_account_status: Input::new("active".to_string()),
            input_emergency_contact_name: Input::default(),
            input_emergency_contact_phone: Input::default(),
            focused_field: 0,
            total_fields: 16, // 0-15: username, display_name, password, email, phone, mobile, pin, ext, dept, title, loc, lang, tz, status, emerg_name, emerg_phone
        }
    }
}

impl UsersViewState {
    /// Get currently selected user
    pub fn selected_user(&self) -> Option<&UserResponse> {
        self.table_state.selected().and_then(|i| self.users.get(i))
    }

    /// Move selection up
    pub fn select_previous(&mut self) {
        if self.users.is_empty() {
            return;
        }

        let selected = self.table_state.selected().unwrap_or(0);
        if selected > 0 {
            self.table_state.select(Some(selected - 1));
        }
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        if self.users.is_empty() {
            return;
        }

        let selected = self.table_state.selected().unwrap_or(0);
        if selected < self.users.len() - 1 {
            self.table_state.select(Some(selected + 1));
        }
    }

    /// Update users list
    pub fn set_users(&mut self, users: Vec<UserResponse>) {
        self.users = users;
        // Reset selection to first item if we had no selection or it's out of bounds
        if self.table_state.selected().is_none()
            || self.table_state.selected().unwrap() >= self.users.len() {
            self.table_state.select(if self.users.is_empty() { None } else { Some(0) });
        }
    }

    /// Open add user modal
    pub fn open_add_modal(&mut self) {
        self.modal = UserModal::Add;
        self.clear_inputs();
        self.focused_field = 0;
    }

    /// Open edit user modal
    pub fn open_edit_modal(&mut self) {
        // Clone user data to avoid borrow checker issues
        if let Some(user) = self.selected_user().cloned() {
            self.modal = UserModal::Edit;
            self.input_username = Input::new(user.username);
            self.input_display_name = Input::new(user.display_name.unwrap_or_default());
            self.input_password = Input::default(); // Don't show existing password
            self.input_enabled = user.enabled;

            // Enterprise fields
            self.input_email = Input::new(user.email.unwrap_or_default());
            self.input_phone_number = Input::new(user.phone_number.unwrap_or_default());
            self.input_mobile_number = Input::new(user.mobile_number.unwrap_or_default());
            self.input_pin = Input::default(); // Don't show existing PIN
            self.input_extension = Input::new(user.extension.unwrap_or_default());
            self.input_department = Input::new(user.department.unwrap_or_default());
            self.input_job_title = Input::new(user.job_title.unwrap_or_default());
            self.input_location = Input::new(user.location.unwrap_or_default());
            self.input_language = Input::new(user.language);
            self.input_timezone = Input::new(user.timezone);
            self.input_account_status = Input::new(user.account_status);
            self.input_emergency_contact_name = Input::new(user.emergency_contact_name.unwrap_or_default());
            self.input_emergency_contact_phone = Input::new(user.emergency_contact_phone.unwrap_or_default());

            self.focused_field = 1; // Focus on display name for edit
        }
    }

    /// Open delete user modal
    pub fn open_delete_modal(&mut self) {
        if self.selected_user().is_some() {
            self.modal = UserModal::Delete;
        }
    }

    /// Close modal
    pub fn close_modal(&mut self) {
        self.modal = UserModal::None;
        self.clear_inputs();
    }

    /// Clear input fields
    fn clear_inputs(&mut self) {
        self.input_username = Input::default();
        self.input_display_name = Input::default();
        self.input_password = Input::default();
        self.input_enabled = true;
        // Enterprise fields
        self.input_email = Input::default();
        self.input_phone_number = Input::default();
        self.input_mobile_number = Input::default();
        self.input_pin = Input::default();
        self.input_extension = Input::default();
        self.input_department = Input::default();
        self.input_job_title = Input::default();
        self.input_location = Input::default();
        self.input_language = Input::new("en-US".to_string());
        self.input_timezone = Input::new("UTC".to_string());
        self.input_account_status = Input::new("active".to_string());
        self.input_emergency_contact_name = Input::default();
        self.input_emergency_contact_phone = Input::default();
    }

    /// Move focus to next field
    pub fn next_field(&mut self) {
        self.focused_field = (self.focused_field + 1) % self.total_fields;
    }

    /// Move focus to previous field
    pub fn prev_field(&mut self) {
        self.focused_field = if self.focused_field == 0 {
            self.total_fields - 1
        } else {
            self.focused_field - 1
        };
    }

    /// Get currently focused input
    pub fn focused_input(&mut self) -> &mut Input {
        match self.focused_field {
            0 => &mut self.input_username,
            1 => &mut self.input_display_name,
            2 => &mut self.input_password,
            3 => &mut self.input_email,
            4 => &mut self.input_phone_number,
            5 => &mut self.input_mobile_number,
            6 => &mut self.input_pin,
            7 => &mut self.input_extension,
            8 => &mut self.input_department,
            9 => &mut self.input_job_title,
            10 => &mut self.input_location,
            11 => &mut self.input_language,
            12 => &mut self.input_timezone,
            13 => &mut self.input_account_status,
            14 => &mut self.input_emergency_contact_name,
            15 => &mut self.input_emergency_contact_phone,
            _ => &mut self.input_username,
        }
    }
}

/// Render the users view
pub fn render(f: &mut Frame, theme: &crate::tui::theme::Theme, state: &mut UsersViewState, area: Rect) {
    // Split into table and details panel
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(60), // Users table
            Constraint::Percentage(40), // Details panel
        ])
        .split(area);

    render_users_table(f, theme, state, chunks[0]);
    render_details_panel(f, theme, state, chunks[1]);

    // Render modal on top if active
    match state.modal {
        UserModal::Add => render_add_modal(f, theme, state, area),
        UserModal::Edit => render_edit_modal(f, theme, state, area),
        UserModal::Delete => render_delete_modal(f, theme, state, area),
        UserModal::None => {}
    }
}

/// Render the users table
fn render_users_table(f: &mut Frame, theme: &crate::tui::theme::Theme, state: &mut UsersViewState, area: Rect) {
    let header = Row::new(vec!["Username", "Display Name", "Tenant", "Enabled"])
        .style(theme.title())
        .bottom_margin(1);

    let rows: Vec<Row> = state
        .users
        .iter()
        .map(|user| {
            Row::new(vec![
                Cell::from(user.username.clone()),
                Cell::from(user.display_name.clone().unwrap_or_else(|| "-".to_string())),
                Cell::from(user.tenant_id.clone()),
                Cell::from(if user.enabled { "✓" } else { "✗" }),
            ])
        })
        .collect();

    let widths = [
        Constraint::Percentage(30),
        Constraint::Percentage(30),
        Constraint::Percentage(30),
        Constraint::Percentage(10),
    ];

    let title = if state.loading {
        " Users (Loading...) "
    } else {
        format!(" Users ({}) ", state.users.len()).leak() as &str
    };

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(theme.border())
                .title_style(theme.title()),
        )
        .row_highlight_style(theme.selected())
        .highlight_symbol(">> ");

    f.render_stateful_widget(table, area, &mut state.table_state);

    // Show error if any
    if let Some(error) = &state.error {
        let error_text = vec![Line::from(vec![
            Span::styled("Error: ", theme.error()),
            Span::raw(error),
        ])];

        let error_para = Paragraph::new(error_text)
            .block(Block::default().borders(Borders::ALL).border_style(theme.error()));

        // Show error in a small area at the bottom
        let error_area = Rect {
            x: area.x,
            y: area.y + area.height.saturating_sub(4),
            width: area.width,
            height: 3,
        };

        f.render_widget(error_para, error_area);
    }
}

/// Render the details panel for selected user
fn render_details_panel(f: &mut Frame, theme: &crate::tui::theme::Theme, state: &UsersViewState, area: Rect) {
    let details_text = if let Some(user) = state.selected_user() {
        let mut lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("Username: ", theme.muted()),
                Span::styled(&user.username, theme.normal()),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Display Name: ", theme.muted()),
                Span::styled(
                    user.display_name.as_deref().unwrap_or("-"),
                    theme.normal(),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Enabled: ", theme.muted()),
                Span::styled(
                    if user.enabled { "Yes" } else { "No" },
                    if user.enabled {
                        theme.success()
                    } else {
                        theme.error()
                    },
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Status: ", theme.muted()),
                Span::styled(&user.account_status, theme.info()),
            ]),
        ];

        // Enterprise fields section
        if user.email.is_some() || user.extension.is_some() || user.department.is_some() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled("── Contact Info ──", theme.title())]));

            if let Some(email) = &user.email {
                lines.push(Line::from(vec![
                    Span::styled("Email: ", theme.muted()),
                    Span::styled(email, theme.normal()),
                ]));
            }
            if let Some(phone) = &user.phone_number {
                lines.push(Line::from(vec![
                    Span::styled("Phone: ", theme.muted()),
                    Span::styled(phone, theme.normal()),
                ]));
            }
            if let Some(mobile) = &user.mobile_number {
                lines.push(Line::from(vec![
                    Span::styled("Mobile: ", theme.muted()),
                    Span::styled(mobile, theme.normal()),
                ]));
            }
            if let Some(ext) = &user.extension {
                lines.push(Line::from(vec![
                    Span::styled("Extension: ", theme.muted()),
                    Span::styled(ext, theme.normal()),
                ]));
            }
        }

        // Job info section
        if user.department.is_some() || user.job_title.is_some() || user.location.is_some() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled("── Job Info ──", theme.title())]));

            if let Some(dept) = &user.department {
                lines.push(Line::from(vec![
                    Span::styled("Department: ", theme.muted()),
                    Span::styled(dept, theme.normal()),
                ]));
            }
            if let Some(title) = &user.job_title {
                lines.push(Line::from(vec![
                    Span::styled("Title: ", theme.muted()),
                    Span::styled(title, theme.normal()),
                ]));
            }
            if let Some(loc) = &user.location {
                lines.push(Line::from(vec![
                    Span::styled("Location: ", theme.muted()),
                    Span::styled(loc, theme.normal()),
                ]));
            }
        }

        // Preferences section
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled("── Preferences ──", theme.title())]));
        lines.push(Line::from(vec![
            Span::styled("Language: ", theme.muted()),
            Span::styled(&user.language, theme.normal()),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Timezone: ", theme.muted()),
            Span::styled(&user.timezone, theme.normal()),
        ]));

        // Emergency contact section
        if user.emergency_contact_name.is_some() || user.emergency_contact_phone.is_some() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled("── Emergency Contact ──", theme.title())]));

            if let Some(name) = &user.emergency_contact_name {
                lines.push(Line::from(vec![
                    Span::styled("Name: ", theme.muted()),
                    Span::styled(name, theme.normal()),
                ]));
            }
            if let Some(phone) = &user.emergency_contact_phone {
                lines.push(Line::from(vec![
                    Span::styled("Phone: ", theme.muted()),
                    Span::styled(phone, theme.normal()),
                ]));
            }
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
    } else if state.users.is_empty() {
        vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "  No users to display",
                theme.muted(),
            )]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "  Press 'a' to add a user",
                theme.info(),
            )]),
            Line::from(vec![Span::styled(
                "  Press 'r' to refresh",
                theme.info(),
            )]),
        ]
    } else {
        vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "  Select a user to view details",
                theme.muted(),
            )]),
            Line::from(""),
            Line::from(""),
            Line::from(vec![Span::styled("  Actions:", theme.title())]),
            Line::from(vec![
                Span::styled("    ", theme.muted()),
                Span::styled("a", theme.primary()),
                Span::styled(" - Add new user", theme.normal()),
            ]),
            Line::from(vec![
                Span::styled("    ", theme.muted()),
                Span::styled("e", theme.primary()),
                Span::styled(" - Edit selected user", theme.normal()),
            ]),
            Line::from(vec![
                Span::styled("    ", theme.muted()),
                Span::styled("d", theme.primary()),
                Span::styled(" - Delete selected user", theme.normal()),
            ]),
        ]
    };

    let details = Paragraph::new(details_text).block(
        Block::default()
            .title(" Details ")
            .borders(Borders::ALL)
            .border_style(theme.border())
            .title_style(theme.title()),
    );

    f.render_widget(details, area);
}

/// Handle navigation events for users view
pub fn handle_navigation(state: &mut UsersViewState, event: &crate::tui::event::Event) {
    if event.is_up() {
        state.select_previous();
    } else if event.is_down() {
        state.select_next();
    }
}

/// Fetch users data
pub async fn fetch_users(
    api_client: &crate::api::ApiClient,
    state: &mut UsersViewState,
) {
    state.loading = true;
    state.error = None;

    match api_client.list_users().await {
        Ok(users) => {
            state.set_users(users);
        }
        Err(e) => {
            state.error = Some(format!("Failed to fetch users: {}", e));
        }
    }

    state.loading = false;
}

/// Render add user modal
fn render_add_modal(f: &mut Frame, theme: &crate::tui::theme::Theme, state: &UsersViewState, area: Rect) {
    let popup_area = centered_rect(70, 80, area);

    let title_text = " Add New User (Tab/Shift+Tab: navigate, Enter: save, Esc: cancel) ";
    let password_masked = "*".repeat(state.input_password.value().len());
    let pin_masked = "*".repeat(state.input_pin.value().len());

    let fields = vec![
        // Core fields
        ("Username:", state.input_username.value(), state.focused_field == 0),
        ("Display Name:", state.input_display_name.value(), state.focused_field == 1),
        ("Password:", password_masked.as_str(), state.focused_field == 2),
        // Enterprise fields
        ("Email:", state.input_email.value(), state.focused_field == 3),
        ("Phone Number:", state.input_phone_number.value(), state.focused_field == 4),
        ("Mobile Number:", state.input_mobile_number.value(), state.focused_field == 5),
        ("PIN (Voicemail):", pin_masked.as_str(), state.focused_field == 6),
        ("Extension:", state.input_extension.value(), state.focused_field == 7),
        ("Department:", state.input_department.value(), state.focused_field == 8),
        ("Job Title:", state.input_job_title.value(), state.focused_field == 9),
        ("Location:", state.input_location.value(), state.focused_field == 10),
        ("Language:", state.input_language.value(), state.focused_field == 11),
        ("Timezone:", state.input_timezone.value(), state.focused_field == 12),
        ("Account Status:", state.input_account_status.value(), state.focused_field == 13),
        ("Emergency Name:", state.input_emergency_contact_name.value(), state.focused_field == 14),
        ("Emergency Phone:", state.input_emergency_contact_phone.value(), state.focused_field == 15),
    ];

    render_form_modal(f, theme, popup_area, title_text, &fields);
}

/// Render edit user modal
fn render_edit_modal(f: &mut Frame, theme: &crate::tui::theme::Theme, state: &UsersViewState, area: Rect) {
    let popup_area = centered_rect(70, 80, area);

    let title_text = " Edit User (Tab/Shift+Tab: navigate, Enter: save, Esc: cancel) ";
    let password_masked = "*".repeat(state.input_password.value().len());
    let pin_masked = "*".repeat(state.input_pin.value().len());

    let fields = vec![
        // Core fields
        ("Username:", state.input_username.value(), false), // Username not editable
        ("Display Name:", state.input_display_name.value(), state.focused_field == 1),
        ("New Password:", password_masked.as_str(), state.focused_field == 2),
        // Enterprise fields
        ("Email:", state.input_email.value(), state.focused_field == 3),
        ("Phone Number:", state.input_phone_number.value(), state.focused_field == 4),
        ("Mobile Number:", state.input_mobile_number.value(), state.focused_field == 5),
        ("New PIN:", pin_masked.as_str(), state.focused_field == 6),
        ("Extension:", state.input_extension.value(), state.focused_field == 7),
        ("Department:", state.input_department.value(), state.focused_field == 8),
        ("Job Title:", state.input_job_title.value(), state.focused_field == 9),
        ("Location:", state.input_location.value(), state.focused_field == 10),
        ("Language:", state.input_language.value(), state.focused_field == 11),
        ("Timezone:", state.input_timezone.value(), state.focused_field == 12),
        ("Account Status:", state.input_account_status.value(), state.focused_field == 13),
        ("Emergency Name:", state.input_emergency_contact_name.value(), state.focused_field == 14),
        ("Emergency Phone:", state.input_emergency_contact_phone.value(), state.focused_field == 15),
    ];

    render_form_modal(f, theme, popup_area, title_text, &fields);
}

/// Render delete confirmation modal
fn render_delete_modal(f: &mut Frame, theme: &crate::tui::theme::Theme, state: &UsersViewState, area: Rect) {
    let popup_area = centered_rect(50, 20, area);
    
    let username = state.selected_user().map(|u| u.username.as_str()).unwrap_or("unknown");
    
    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("Are you sure you want to delete user ", theme.normal()),
            Span::styled(username, theme.error()),
            Span::styled("?", theme.normal()),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Press ", theme.normal()),
            Span::styled("Enter", theme.error()),
            Span::styled(" to confirm, ", theme.normal()),
            Span::styled("Esc", theme.primary()),
            Span::styled(" to cancel", theme.normal()),
        ]),
    ];
    
    f.render_widget(Clear, popup_area);
    let block = Block::default()
        .title(" Confirm Delete ")
        .borders(Borders::ALL)
        .border_style(theme.error());
    let paragraph = Paragraph::new(text).block(block);
    f.render_widget(paragraph, popup_area);
}

/// Helper to render a form modal
fn render_form_modal(f: &mut Frame, theme: &crate::tui::theme::Theme, area: Rect, title: &str, fields: &[(&str, &str, bool)]) {
    f.render_widget(Clear, area);
    
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(theme.border());
    
    let inner = block.inner(area);
    f.render_widget(block, area);
    
    // Render form fields
    let mut y = inner.y + 1;
    for (label, value, focused) in fields {
        let label_line = Line::from(vec![
            Span::styled(*label, theme.muted()),
        ]);
        f.render_widget(Paragraph::new(label_line), Rect::new(inner.x + 2, y, inner.width - 4, 1));
        y += 1;
        
        let value_style = if *focused { theme.selected() } else { theme.normal() };
        let value_line = Line::from(vec![
            Span::styled(format!(" {} ", value), value_style),
        ]);
        f.render_widget(Paragraph::new(value_line), Rect::new(inner.x + 2, y, inner.width - 4, 1));
        y += 3;
    }
}

/// Create a centered rectangle
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Handle keyboard events for user management
pub async fn handle_user_action(
    state: &mut UsersViewState,
    api_client: &crate::api::ApiClient,
    event: &crate::tui::event::Event,
) -> bool {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    
    // Check if we're in a modal
    if state.modal != UserModal::None {
        match event {
            crate::tui::event::Event::Key(KeyEvent { code, modifiers, .. }) => {
                match code {
                    KeyCode::Esc if *modifiers == KeyModifiers::NONE => {
                        state.close_modal();
                        return true;
                    }
                    KeyCode::Tab if *modifiers == KeyModifiers::NONE => {
                        if state.modal != UserModal::Delete {
                            state.next_field();
                        }
                        return true;
                    }
                    KeyCode::BackTab => {
                        if state.modal != UserModal::Delete {
                            state.prev_field();
                        }
                        return true;
                    }
                    KeyCode::Enter if *modifiers == KeyModifiers::NONE => {
                        match state.modal {
                            UserModal::Add => {
                                create_user(api_client, state).await;
                            }
                            UserModal::Edit => {
                                update_user(api_client, state).await;
                            }
                            UserModal::Delete => {
                                delete_user(api_client, state).await;
                            }
                            UserModal::None => {}
                        }
                        return true;
                    }
                    KeyCode::Char(_) | KeyCode::Backspace => {
                        if state.modal != UserModal::Delete {
                            let key_event = crossterm::event::KeyEvent::new(*code, *modifiers);
                            if let Some(req) = tui_input::backend::crossterm::to_input_request(&crossterm::event::Event::Key(key_event)) {
                                state.focused_input().handle(req);
                            }
                        }
                        return true;
                    }
                    _ => return true,
                }
            }
            _ => return true,
        }
    }
    
    false
}

/// Create a new user
async fn create_user(api_client: &crate::api::ApiClient, state: &mut UsersViewState) {
    let username = state.input_username.value().to_string();
    let password = state.input_password.value().to_string();
    let display_name = state.input_display_name.value().to_string();

    if username.is_empty() || password.is_empty() {
        state.error = Some("Username and password are required".to_string());
        return;
    }

    // Helper function to convert empty strings to None
    let optional = |s: &str| if s.is_empty() { None } else { Some(s.to_string()) };

    let request = CreateUserRequest {
        username,
        password,
        display_name: optional(&display_name),
        tenant_id: None,
        enabled: Some(state.input_enabled),
        // Enterprise fields
        email: optional(state.input_email.value()),
        phone_number: optional(state.input_phone_number.value()),
        mobile_number: optional(state.input_mobile_number.value()),
        pin: optional(state.input_pin.value()),
        extension: optional(state.input_extension.value()),
        department: optional(state.input_department.value()),
        job_title: optional(state.input_job_title.value()),
        location: optional(state.input_location.value()),
        language: optional(state.input_language.value()),
        timezone: optional(state.input_timezone.value()),
        account_status: optional(state.input_account_status.value()),
        emergency_contact_name: optional(state.input_emergency_contact_name.value()),
        emergency_contact_phone: optional(state.input_emergency_contact_phone.value()),
    };

    match api_client.create_user(&request).await {
        Ok(_) => {
            state.close_modal();
            // Refresh user list
            fetch_users(api_client, state).await;
        }
        Err(e) => {
            state.error = Some(format!("Failed to create user: {}", e));
        }
    }
}

/// Update an existing user
async fn update_user(api_client: &crate::api::ApiClient, state: &mut UsersViewState) {
    let username = state.input_username.value().to_string();
    let display_name = state.input_display_name.value().to_string();
    let password = state.input_password.value().to_string();
    let pin = state.input_pin.value().to_string();

    // Helper function to convert empty strings to None
    let optional = |s: &str| if s.is_empty() { None } else { Some(s.to_string()) };

    let request = UpdateUserRequest {
        display_name: optional(&display_name),
        enabled: Some(state.input_enabled),
        // Enterprise fields
        email: optional(state.input_email.value()),
        phone_number: optional(state.input_phone_number.value()),
        mobile_number: optional(state.input_mobile_number.value()),
        pin: optional(&pin),
        extension: optional(state.input_extension.value()),
        department: optional(state.input_department.value()),
        job_title: optional(state.input_job_title.value()),
        location: optional(state.input_location.value()),
        language: optional(state.input_language.value()),
        timezone: optional(state.input_timezone.value()),
        account_status: optional(state.input_account_status.value()),
        emergency_contact_name: optional(state.input_emergency_contact_name.value()),
        emergency_contact_phone: optional(state.input_emergency_contact_phone.value()),
    };
    
    match api_client.update_user(&username, &request).await {
        Ok(_) => {
            // Update password if provided
            if !password.is_empty() {
                let _ = api_client.update_password(&username, &crate::api::UpdatePasswordRequest {
                    password,
                }).await;
            }
            
            state.close_modal();
            // Refresh user list
            fetch_users(api_client, state).await;
        }
        Err(e) => {
            state.error = Some(format!("Failed to update user: {}", e));
        }
    }
}

/// Delete a user
async fn delete_user(api_client: &crate::api::ApiClient, state: &mut UsersViewState) {
    if let Some(user) = state.selected_user() {
        let username = user.username.clone();
        
        match api_client.delete_user(&username).await {
            Ok(_) => {
                state.close_modal();
                // Refresh user list
                fetch_users(api_client, state).await;
            }
            Err(e) => {
                state.error = Some(format!("Failed to delete user: {}", e));
            }
        }
    }
}
