//! Phone numbers view showing phone numbers and details

use crate::api::PhoneNumberResponse;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};

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
}

/// Render the phone numbers view
pub fn render(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &mut PhoneNumbersViewState,
    area: Rect,
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
    render_details_panel(f, theme, state, chunks[1]);
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
            .title("Phone Numbers (↑↓ to navigate)")
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
) {
    let content = if let Some(phone) = state.selected_phone_number() {
        vec![
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
            Line::from(""),
            Line::from(vec![
                Span::styled("Carrier: ", theme.muted()),
                Span::raw(phone.carrier.as_deref().unwrap_or("-")),
            ]),
            Line::from(vec![
                Span::styled("LRN: ", theme.muted()),
                Span::raw(phone.lrn.as_deref().unwrap_or("-")),
            ]),
            Line::from(vec![
                Span::styled("Rate Center: ", theme.muted()),
                Span::raw(phone.rate_center.as_deref().unwrap_or("-")),
            ]),
        ]
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
