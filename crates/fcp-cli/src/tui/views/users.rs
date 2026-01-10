//! Users view showing user list and details

use crate::api::UserResponse;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};

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
