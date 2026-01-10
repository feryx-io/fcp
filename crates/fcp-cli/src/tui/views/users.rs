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
    /// Which input field is focused (0=username, 1=display_name, 2=password)
    pub focused_field: usize,
    /// Whether user is enabled (for add/edit)
    pub input_enabled: bool,
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
            focused_field: 0,
            input_enabled: true,
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
    }

    /// Move focus to next field
    pub fn next_field(&mut self) {
        self.focused_field = (self.focused_field + 1) % 3;
    }

    /// Move focus to previous field
    pub fn prev_field(&mut self) {
        self.focused_field = if self.focused_field == 0 { 2 } else { self.focused_field - 1 };
    }

    /// Get currently focused input
    pub fn focused_input(&mut self) -> &mut Input {
        match self.focused_field {
            0 => &mut self.input_username,
            1 => &mut self.input_display_name,
            2 => &mut self.input_password,
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
        vec![
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
                Span::styled("Tenant ID: ", theme.muted()),
                Span::styled(&user.tenant_id, theme.normal()),
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
                Span::styled("Created: ", theme.muted()),
                Span::styled(format!("{:?}", user.created_at), theme.normal()),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Updated: ", theme.muted()),
                Span::styled(format!("{:?}", user.updated_at), theme.normal()),
            ]),
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
    let popup_area = centered_rect(60, 50, area);

    let title_text = " Add New User (Tab: next field, Enter: save, Esc: cancel) ";
    let password_masked = "*".repeat(state.input_password.value().len());
    let fields = vec![
        ("Username:", state.input_username.value(), state.focused_field == 0),
        ("Display Name:", state.input_display_name.value(), state.focused_field == 1),
        ("Password:", password_masked.as_str(), state.focused_field == 2),
    ];

    render_form_modal(f, theme, popup_area, title_text, &fields);
}

/// Render edit user modal
fn render_edit_modal(f: &mut Frame, theme: &crate::tui::theme::Theme, state: &UsersViewState, area: Rect) {
    let popup_area = centered_rect(60, 50, area);

    let title_text = " Edit User (Tab: next field, Enter: save, Esc: cancel) ";
    let password_masked = "*".repeat(state.input_password.value().len());
    let fields = vec![
        ("Username:", state.input_username.value(), false), // Username not editable
        ("Display Name:", state.input_display_name.value(), state.focused_field == 1),
        ("New Password:", password_masked.as_str(), state.focused_field == 2),
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
            crate::tui::event::Event::Key(KeyEvent { code, modifiers: KeyModifiers::NONE, .. }) => {
                match code {
                    KeyCode::Esc => {
                        state.close_modal();
                        return true;
                    }
                    KeyCode::Tab => {
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
                    KeyCode::Enter => {
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
                            let key_event = crossterm::event::KeyEvent::new(*code, KeyModifiers::NONE);
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
    
    let request = CreateUserRequest {
        username,
        password,
        display_name: if display_name.is_empty() { None } else { Some(display_name) },
        tenant_id: None,
        enabled: Some(state.input_enabled),
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
    
    let request = UpdateUserRequest {
        display_name: if display_name.is_empty() { None } else { Some(display_name) },
        enabled: Some(state.input_enabled),
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
