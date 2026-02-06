//! Registrations view showing SIP registrations and details

use crate::api::RegistrationResponse;
use crate::tui::event::{Event, EventResult, ViewContext};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};
use std::time::Duration;

/// State for the registrations view
pub struct RegistrationsViewState {
    /// List of registrations
    pub registrations: Vec<RegistrationResponse>,
    /// Table state for selection
    pub table_state: TableState,
    /// Last error message
    pub error: Option<String>,
    /// Loading state
    pub loading: bool,
    /// AOR filter (if any)
    pub aor_filter: Option<String>,
}

impl Default for RegistrationsViewState {
    fn default() -> Self {
        let mut table_state = TableState::default();
        table_state.select(Some(0));

        Self {
            registrations: Vec::new(),
            table_state,
            error: None,
            loading: false,
            aor_filter: None,
        }
    }
}

impl RegistrationsViewState {
    /// Get currently selected registration
    pub fn selected_registration(&self) -> Option<&RegistrationResponse> {
        self.table_state
            .selected()
            .and_then(|i| self.registrations.get(i))
    }

    /// Move selection up
    pub fn select_previous(&mut self) {
        if self.registrations.is_empty() {
            return;
        }

        let selected = self.table_state.selected().unwrap_or(0);
        if selected > 0 {
            self.table_state.select(Some(selected - 1));
        }
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        if self.registrations.is_empty() {
            return;
        }

        let selected = self.table_state.selected().unwrap_or(0);
        if selected < self.registrations.len() - 1 {
            self.table_state.select(Some(selected + 1));
        }
    }

    /// Update registrations list
    pub fn set_registrations(&mut self, registrations: Vec<RegistrationResponse>) {
        self.registrations = registrations;
        // Reset selection to first item if we had no selection or it's out of bounds
        if self.table_state.selected().is_none()
            || self.table_state.selected().unwrap() >= self.registrations.len()
        {
            self.table_state.select(if self.registrations.is_empty() {
                None
            } else {
                Some(0)
            });
        }
    }
}

/// Render the registrations view
pub fn render(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &mut RegistrationsViewState,
    area: Rect,
) {
    // Split into table and details panel
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(60), // Registrations table
            Constraint::Percentage(40), // Details panel
        ])
        .split(area);

    render_registrations_table(f, theme, state, chunks[0]);
    render_details_panel(f, theme, state, chunks[1]);
}

/// Render the registrations table
fn render_registrations_table(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &mut RegistrationsViewState,
    area: Rect,
) {
    let header = Row::new(vec!["AOR", "Contact", "Expires In", "User Agent", "Q"])
        .style(theme.title())
        .bottom_margin(1);

    let rows: Vec<Row> = state
        .registrations
        .iter()
        .map(|reg| {
            // Format expires in (countdown)
            let expires_in = format_expires_in(reg.expires_in_seconds);

            // Format contact (truncate if too long)
            let contact = if reg.contact.len() > 30 {
                format!("{}...", &reg.contact[..27])
            } else {
                reg.contact.clone()
            };

            // Format user agent
            let user_agent = reg
                .user_agent
                .as_ref()
                .map(|ua| {
                    if ua.len() > 20 {
                        format!("{}...", &ua[..17])
                    } else {
                        ua.clone()
                    }
                })
                .unwrap_or_else(|| "-".to_string());

            // Format Q value
            let q_value = reg
                .q
                .map(|q| format!("{:.2}", q))
                .unwrap_or_else(|| "1.00".to_string());

            Row::new(vec![
                Cell::from(reg.aor.clone()),
                Cell::from(contact),
                Cell::from(expires_in),
                Cell::from(user_agent),
                Cell::from(q_value),
            ])
        })
        .collect();

    let widths = [
        Constraint::Percentage(25), // AOR
        Constraint::Percentage(25), // Contact
        Constraint::Percentage(15), // Expires In
        Constraint::Percentage(25), // User Agent
        Constraint::Percentage(10), // Q
    ];

    let title = if state.loading {
        " Registrations (Loading...) "
    } else if let Some(aor) = &state.aor_filter {
        format!(
            " Registrations for {} ({}) ",
            aor,
            state.registrations.len()
        )
        .leak() as &str
    } else {
        format!(" Registrations ({}) ", state.registrations.len()).leak() as &str
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

        let error_para = Paragraph::new(error_text).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.error()),
        );

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

/// Render the details panel for selected registration
fn render_details_panel(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &RegistrationsViewState,
    area: Rect,
) {
    let details_text = if let Some(reg) = state.selected_registration() {
        // Calculate time until expiration
        let now = std::time::SystemTime::now();
        let time_until_expiry = reg
            .expires_at
            .duration_since(now)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Calculate how long ago registration was made
        let time_since_registration = now
            .duration_since(reg.registered_at)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("AOR: ", theme.muted()),
                Span::styled(reg.aor.clone(), theme.normal()),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Contact: ", theme.muted()),
                Span::styled(reg.contact.clone(), theme.normal()),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Expires In: ", theme.muted()),
                Span::styled(
                    format_expires_in(time_until_expiry),
                    if time_until_expiry < 60 {
                        theme.error() // Expiring soon
                    } else if time_until_expiry < 300 {
                        theme.info() // Warning
                    } else {
                        theme.success() // Good
                    },
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Registered: ", theme.muted()),
                Span::styled(
                    format!("{} ago", format_duration(time_since_registration)),
                    theme.normal(),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("User Agent: ", theme.muted()),
                Span::styled(
                    reg.user_agent
                        .as_ref()
                        .map(|s| s.clone())
                        .unwrap_or_else(|| "-".to_string()),
                    theme.normal(),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Q-Value: ", theme.muted()),
                Span::styled(
                    reg.q
                        .map(|q| format!("{:.2}", q))
                        .unwrap_or_else(|| "1.00".to_string()),
                    theme.normal(),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Call-ID: ", theme.muted()),
                Span::styled(reg.call_id.clone(), theme.normal()),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("CSeq: ", theme.muted()),
                Span::styled(reg.cseq.to_string(), theme.normal()),
            ]),
        ]
    } else if state.registrations.is_empty() {
        vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "  No active registrations",
                theme.muted(),
            )]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "  Users must register to appear here",
                theme.info(),
            )]),
            Line::from(""),
            Line::from(vec![Span::styled("  Press 'r' to refresh", theme.info())]),
        ]
    } else {
        vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "  Select a registration to view details",
                theme.muted(),
            )]),
        ]
    };

    let details = Paragraph::new(details_text).block(
        Block::default()
            .title(" Registration Details ")
            .borders(Borders::ALL)
            .border_style(theme.border())
            .title_style(theme.title()),
    );

    f.render_widget(details, area);
}

/// Format expiration time as human-readable countdown
fn format_expires_in(seconds: u64) -> String {
    if seconds == 0 {
        return "Expired".to_string();
    }

    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;

    if hours > 0 {
        format!("{}h {}m", hours, minutes)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, secs)
    } else {
        format!("{}s", secs)
    }
}

/// Format duration as human-readable string
fn format_duration(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;

    if hours > 0 {
        format!("{}h {}m", hours, minutes)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, secs)
    } else {
        format!("{}s", secs)
    }
}

/// Handle all events for the registrations view
pub async fn handle_event(
    state: &mut RegistrationsViewState,
    api_client: &crate::api::ApiClient,
    event: &Event,
    _ctx: &ViewContext,
    events_url: Option<&str>,
) -> EventResult {
    // Navigation
    if event.is_up() {
        state.select_previous();
        return EventResult::Consumed;
    }
    if event.is_down() {
        state.select_next();
        return EventResult::Consumed;
    }

    // Refresh
    if event.is_refresh() {
        fetch_data(api_client, state, events_url).await;
        return EventResult::Consumed;
    }

    EventResult::NotConsumed
}

/// Check if the registrations view needs initial data loading
pub fn needs_data(state: &RegistrationsViewState) -> bool {
    state.registrations.is_empty() && !state.loading
}

/// Refresh interval for registrations (medium refresh for expiration countdown)
pub fn refresh_interval() -> Duration {
    Duration::from_secs(3)
}


/// Fetch registrations data
/// If events_url is provided, fetches aggregated registrations from the events server.
/// Otherwise, falls back to the local call-manager API.
pub async fn fetch_data(
    api_client: &crate::api::ApiClient,
    state: &mut RegistrationsViewState,
    events_url: Option<&str>,
) {
    state.loading = true;
    state.error = None;

    // Try events server first if URL is configured
    if let Some(url) = events_url {
        match api_client.list_aggregated_registrations(url).await {
            Ok(aggregated_regs) => {
                // Convert aggregated registrations to RegistrationResponse format
                let registrations: Vec<crate::api::RegistrationResponse> = aggregated_regs
                    .into_iter()
                    .map(|r| r.to_registration_response())
                    .collect();
                if registrations.is_empty() {
                    state.error = Some("No active registrations".to_string());
                }
                state.set_registrations(registrations);
                state.loading = false;
                return;
            }
            Err(e) => {
                // Show error to user instead of silently falling back
                state.error = Some(format!("Events server error: {}", e));
                state.loading = false;
                return;
            }
        }
    }

    // Fall back to local call-manager API
    match api_client.list_all_registrations().await {
        Ok(registrations) => {
            if registrations.is_empty() {
                state.error = Some("No active registrations".to_string());
            }
            state.set_registrations(registrations);
        }
        Err(e) => {
            state.error = Some(format!("Failed to fetch registrations: {}", e));
            state.set_registrations(Vec::new());
        }
    }

    state.loading = false;
}
