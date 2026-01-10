//! Registrations view showing SIP registrations and details

use crate::api::RegistrationResponse;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};

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
        format!(" Registrations for {} ({}) ", aor, state.registrations.len()).leak() as &str
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
            Line::from(vec![Span::styled(
                "  Press 'r' to refresh",
                theme.info(),
            )]),
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

/// Handle navigation events for registrations view
pub fn handle_navigation(state: &mut RegistrationsViewState, event: &crate::tui::event::Event) {
    if event.is_up() {
        state.select_previous();
    } else if event.is_down() {
        state.select_next();
    }
}

/// Fetch registrations data for all users
pub async fn fetch_registrations(
    api_client: &crate::api::ApiClient,
    state: &mut RegistrationsViewState,
) {
    state.loading = true;
    state.error = None;

    let mut all_registrations = Vec::new();
    let mut attempted_aors: Vec<String> = Vec::new();

    // Extract server hostname from API client URL
    let server_host = extract_host_from_url(api_client.base_url());

    // Build list of domains to try
    let mut domains = vec![
        server_host.clone(),
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        // Common local IPs that might be used for SIP when API is on localhost
        "192.168.1.1".to_string(),
        "192.168.1.100".to_string(),
        "192.168.1.225".to_string(), // Current server IP
        "192.168.0.1".to_string(),
        "10.0.0.1".to_string(),
    ];

    domains.dedup(); // Remove duplicates

    // Strategy 1: Try to fetch users and construct AORs
    if let Ok(users) = api_client.list_users().await {
        for user in &users {
            // Try each domain pattern for each user
            let mut user_domains = domains.clone();
            user_domains.insert(0, user.tenant_id.clone()); // Try tenant_id first

            for domain in user_domains {
                let aor = format!("sip:{}@{}", user.username, domain);
                attempted_aors.push(aor.clone());
                if let Ok(mut regs) = api_client.list_registrations(&aor).await {
                    if !regs.is_empty() {
                        all_registrations.append(&mut regs);
                        break; // Found registrations for this user, move to next
                    }
                }
            }
        }
    }

    // Strategy 2: Try common test user AORs if no users found
    if all_registrations.is_empty() {
        let mut test_aors = Vec::new();
        let test_users = vec!["alice", "bob", "charlie", "dave"];

        for user in &test_users {
            for domain in &domains {
                test_aors.push(format!("sip:{}@{}", user, domain));
                test_aors.push(format!("{}@{}", user, domain));
            }
        }

        for aor in test_aors {
            attempted_aors.push(aor.clone());
            match api_client.list_registrations(&aor).await {
                Ok(mut regs) => {
                    if !regs.is_empty() {
                        all_registrations.append(&mut regs);
                    }
                }
                Err(e) => {
                    // Store first error for debugging
                    if state.error.is_none() {
                        state.error = Some(format!(
                            "API: {} | Error: {} (tried {} AORs)",
                            api_client.base_url(),
                            e,
                            attempted_aors.len()
                        ));
                    }
                }
            }
        }
    }

    // If we found registrations, clear any error
    if !all_registrations.is_empty() {
        state.error = None;
    } else if state.error.is_none() {
        // Show which domains were attempted
        state.error = Some(format!(
            "No registrations found. API: {} | Tried {} AORs across domains: {}",
            api_client.base_url(),
            attempted_aors.len(),
            domains.join(", ")
        ));
    }

    state.set_registrations(all_registrations);
    state.loading = false;
}

/// Extract hostname from URL (e.g., "http://192.168.1.225:8081" -> "192.168.1.225")
fn extract_host_from_url(url: &str) -> String {
    // Remove protocol prefix
    let without_protocol = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);

    // Extract host (everything before the first colon or slash)
    if let Some(host) = without_protocol.split(&[':', '/'][..]).next() {
        if !host.is_empty() {
            return host.to_string();
        }
    }

    "localhost".to_string()
}
