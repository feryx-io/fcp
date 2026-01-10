//! Debug view showing internal server state

use crate::api::{DebugCallResponse, DebugSummaryResponse, DialogResponse, RateLimitResponse, SubscriptionResponse};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Tabs},
    Frame,
};

/// Debug sub-view categories
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugSubView {
    Summary,
    Dialogs,
    Subscriptions,
    RateLimits,
    Calls,
}

impl DebugSubView {
    /// Get all available sub-views
    pub fn all() -> &'static [DebugSubView] {
        &[
            DebugSubView::Summary,
            DebugSubView::Dialogs,
            DebugSubView::Subscriptions,
            DebugSubView::RateLimits,
            DebugSubView::Calls,
        ]
    }

    /// Get sub-view title
    pub fn title(&self) -> &'static str {
        match self {
            DebugSubView::Summary => "Summary",
            DebugSubView::Dialogs => "Dialogs",
            DebugSubView::Subscriptions => "Subscriptions",
            DebugSubView::RateLimits => "Rate Limits",
            DebugSubView::Calls => "Calls",
        }
    }

    /// Get next sub-view
    pub fn next(&self) -> DebugSubView {
        let views = Self::all();
        let current_idx = views.iter().position(|v| v == self).unwrap();
        views[(current_idx + 1) % views.len()]
    }

    /// Get previous sub-view
    pub fn prev(&self) -> DebugSubView {
        let views = Self::all();
        let current_idx = views.iter().position(|v| v == self).unwrap();
        views[(current_idx + views.len() - 1) % views.len()]
    }
}

/// State for the debug view
pub struct DebugViewState {
    /// Current sub-view
    pub active_subview: DebugSubView,
    /// Summary data
    pub summary: Option<DebugSummaryResponse>,
    /// Dialogs list
    pub dialogs: Vec<DialogResponse>,
    /// Subscriptions list
    pub subscriptions: Vec<SubscriptionResponse>,
    /// Rate limits list
    pub rate_limits: Vec<RateLimitResponse>,
    /// Debug calls list
    pub debug_calls: Vec<DebugCallResponse>,
    /// Table state for selection
    pub table_state: TableState,
    /// Last error message
    pub error: Option<String>,
    /// Loading state
    pub loading: bool,
}

impl Default for DebugViewState {
    fn default() -> Self {
        let mut table_state = TableState::default();
        table_state.select(Some(0));

        Self {
            active_subview: DebugSubView::Summary,
            summary: None,
            dialogs: Vec::new(),
            subscriptions: Vec::new(),
            rate_limits: Vec::new(),
            debug_calls: Vec::new(),
            table_state,
            error: None,
            loading: false,
        }
    }
}

impl DebugViewState {
    /// Navigate to next sub-view
    pub fn next_subview(&mut self) {
        self.active_subview = self.active_subview.next();
        self.table_state.select(Some(0));
    }

    /// Navigate to previous sub-view
    pub fn prev_subview(&mut self) {
        self.active_subview = self.active_subview.prev();
        self.table_state.select(Some(0));
    }

    /// Move selection up
    pub fn select_previous(&mut self) {
        let len = match self.active_subview {
            DebugSubView::Summary => 0,
            DebugSubView::Dialogs => self.dialogs.len(),
            DebugSubView::Subscriptions => self.subscriptions.len(),
            DebugSubView::RateLimits => self.rate_limits.len(),
            DebugSubView::Calls => self.debug_calls.len(),
        };

        if len == 0 {
            return;
        }

        let selected = self.table_state.selected().unwrap_or(0);
        if selected > 0 {
            self.table_state.select(Some(selected - 1));
        }
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        let len = match self.active_subview {
            DebugSubView::Summary => 0,
            DebugSubView::Dialogs => self.dialogs.len(),
            DebugSubView::Subscriptions => self.subscriptions.len(),
            DebugSubView::RateLimits => self.rate_limits.len(),
            DebugSubView::Calls => self.debug_calls.len(),
        };

        if len == 0 {
            return;
        }

        let selected = self.table_state.selected().unwrap_or(0);
        if selected < len - 1 {
            self.table_state.select(Some(selected + 1));
        }
    }
}

/// Render the debug view
pub fn render(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &mut DebugViewState,
    area: Rect,
) {
    // Split into sub-view tabs and content
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Sub-view tabs
            Constraint::Min(10),   // Content area
        ])
        .split(area);

    render_subview_tabs(f, theme, state, chunks[0]);
    render_subview_content(f, theme, state, chunks[1]);
}

/// Render sub-view tabs
fn render_subview_tabs(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &DebugViewState,
    area: Rect,
) {
    let titles: Vec<String> = DebugSubView::all()
        .iter()
        .map(|v| v.title().to_string())
        .collect();

    let selected_index = DebugSubView::all()
        .iter()
        .position(|v| v == &state.active_subview)
        .unwrap_or(0);

    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.border())
                .title(" Debug Info "),
        )
        .select(selected_index)
        .style(theme.normal())
        .highlight_style(theme.selected());

    f.render_widget(tabs, area);
}

/// Render sub-view content
fn render_subview_content(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &mut DebugViewState,
    area: Rect,
) {
    match state.active_subview {
        DebugSubView::Summary => render_summary(f, theme, state, area),
        DebugSubView::Dialogs => render_dialogs(f, theme, state, area),
        DebugSubView::Subscriptions => render_subscriptions(f, theme, state, area),
        DebugSubView::RateLimits => render_rate_limits(f, theme, state, area),
        DebugSubView::Calls => render_debug_calls(f, theme, state, area),
    }
}

/// Render summary sub-view
fn render_summary(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &DebugViewState,
    area: Rect,
) {
    let summary_text = if let Some(summary) = &state.summary {
        vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("Server State Overview", theme.title()),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Active Dialogs: ", theme.muted()),
                Span::styled(summary.active_dialogs.to_string(), theme.info()),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Active Subscriptions: ", theme.muted()),
                Span::styled(summary.active_subscriptions.to_string(), theme.info()),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Active Calls: ", theme.muted()),
                Span::styled(summary.active_calls.to_string(), theme.info()),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Rate Limit Entries: ", theme.muted()),
                Span::styled(summary.rate_limit_entries.to_string(), theme.info()),
            ]),
            Line::from(""),
            Line::from(""),
            Line::from(vec![Span::styled(
                "Use Tab or 1-5 to switch between debug views",
                theme.muted(),
            )]),
        ]
    } else if state.loading {
        vec![
            Line::from(""),
            Line::from(vec![Span::styled("Loading debug summary...", theme.info())]),
        ]
    } else {
        vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "No debug data available",
                theme.muted(),
            )]),
            Line::from(""),
            Line::from(vec![Span::styled("Press 'r' to refresh", theme.info())]),
        ]
    };

    let paragraph = Paragraph::new(summary_text).block(
        Block::default()
            .title(" Summary ")
            .borders(Borders::ALL)
            .border_style(theme.border())
            .title_style(theme.title()),
    );

    f.render_widget(paragraph, area);
}

/// Render dialogs sub-view
fn render_dialogs(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &mut DebugViewState,
    area: Rect,
) {
    if state.dialogs.is_empty() {
        let text = vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "No active dialogs",
                theme.muted(),
            )]),
        ];
        let paragraph = Paragraph::new(text).block(
            Block::default()
                .title(" Dialogs ")
                .borders(Borders::ALL)
                .border_style(theme.border()),
        );
        f.render_widget(paragraph, area);
        return;
    }

    let header = Row::new(vec!["Call-ID", "State", "Local", "Remote", "L-CSeq", "R-CSeq"])
        .style(theme.title())
        .bottom_margin(1);

    let rows: Vec<Row> = state
        .dialogs
        .iter()
        .map(|dialog| {
            let call_id = if dialog.call_id.len() > 20 {
                format!("{}...", &dialog.call_id[..17])
            } else {
                dialog.call_id.clone()
            };

            Row::new(vec![
                Cell::from(call_id),
                Cell::from(dialog.state.clone()),
                Cell::from(dialog.local_uri.clone()),
                Cell::from(dialog.remote_uri.clone()),
                Cell::from(dialog.local_cseq.to_string()),
                Cell::from(dialog.remote_cseq.to_string()),
            ])
        })
        .collect();

    let widths = [
        Constraint::Percentage(20),
        Constraint::Percentage(15),
        Constraint::Percentage(25),
        Constraint::Percentage(25),
        Constraint::Percentage(7),
        Constraint::Percentage(8),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .title(format!(" Dialogs ({}) ", state.dialogs.len()))
                .borders(Borders::ALL)
                .border_style(theme.border())
                .title_style(theme.title()),
        )
        .row_highlight_style(theme.selected())
        .highlight_symbol(">> ");

    f.render_stateful_widget(table, area, &mut state.table_state);
}

/// Render subscriptions sub-view
fn render_subscriptions(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &mut DebugViewState,
    area: Rect,
) {
    if state.subscriptions.is_empty() {
        let text = vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "No active subscriptions",
                theme.muted(),
            )]),
        ];
        let paragraph = Paragraph::new(text).block(
            Block::default()
                .title(" Subscriptions ")
                .borders(Borders::ALL)
                .border_style(theme.border()),
        );
        f.render_widget(paragraph, area);
        return;
    }

    let header = Row::new(vec!["Call-ID", "Event", "State", "Contact", "Expires At"])
        .style(theme.title())
        .bottom_margin(1);

    let rows: Vec<Row> = state
        .subscriptions
        .iter()
        .map(|sub| {
            let call_id = if sub.call_id.len() > 15 {
                format!("{}...", &sub.call_id[..12])
            } else {
                sub.call_id.clone()
            };

            let contact = if sub.contact.len() > 25 {
                format!("{}...", &sub.contact[..22])
            } else {
                sub.contact.clone()
            };

            Row::new(vec![
                Cell::from(call_id),
                Cell::from(sub.event.clone()),
                Cell::from(sub.state.clone()),
                Cell::from(contact),
                Cell::from(sub.expires_at.clone()),
            ])
        })
        .collect();

    let widths = [
        Constraint::Percentage(20),
        Constraint::Percentage(15),
        Constraint::Percentage(15),
        Constraint::Percentage(30),
        Constraint::Percentage(20),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .title(format!(" Subscriptions ({}) ", state.subscriptions.len()))
                .borders(Borders::ALL)
                .border_style(theme.border())
                .title_style(theme.title()),
        )
        .row_highlight_style(theme.selected())
        .highlight_symbol(">> ");

    f.render_stateful_widget(table, area, &mut state.table_state);
}

/// Render rate limits sub-view
fn render_rate_limits(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &mut DebugViewState,
    area: Rect,
) {
    if state.rate_limits.is_empty() {
        let text = vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "No rate limit entries",
                theme.muted(),
            )]),
        ];
        let paragraph = Paragraph::new(text).block(
            Block::default()
                .title(" Rate Limits ")
                .borders(Borders::ALL)
                .border_style(theme.border()),
        );
        f.render_widget(paragraph, area);
        return;
    }

    let header = Row::new(vec!["Dialog ID", "Window Start", "Request Counts"])
        .style(theme.title())
        .bottom_margin(1);

    let rows: Vec<Row> = state
        .rate_limits
        .iter()
        .map(|rl| {
            let dialog_id = if rl.dialog_id.len() > 25 {
                format!("{}...", &rl.dialog_id[..22])
            } else {
                rl.dialog_id.clone()
            };

            // Format counts as "METHOD:count, METHOD:count"
            let counts = format_counts(&rl.counts);

            Row::new(vec![
                Cell::from(dialog_id),
                Cell::from(rl.window_start.clone()),
                Cell::from(counts),
            ])
        })
        .collect();

    let widths = [
        Constraint::Percentage(30),
        Constraint::Percentage(25),
        Constraint::Percentage(45),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .title(format!(" Rate Limits ({}) ", state.rate_limits.len()))
                .borders(Borders::ALL)
                .border_style(theme.border())
                .title_style(theme.title()),
        )
        .row_highlight_style(theme.selected())
        .highlight_symbol(">> ");

    f.render_stateful_widget(table, area, &mut state.table_state);
}

/// Render debug calls sub-view
fn render_debug_calls(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &mut DebugViewState,
    area: Rect,
) {
    if state.debug_calls.is_empty() {
        let text = vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "No debug call information",
                theme.muted(),
            )]),
        ];
        let paragraph = Paragraph::new(text).block(
            Block::default()
                .title(" Debug Calls ")
                .borders(Borders::ALL)
                .border_style(theme.border()),
        );
        f.render_widget(paragraph, area);
        return;
    }

    let header = Row::new(vec![
        "Call ID",
        "State",
        "Media Session",
        "A-Leg Dialog",
        "B-Leg Dialog",
    ])
    .style(theme.title())
    .bottom_margin(1);

    let rows: Vec<Row> = state
        .debug_calls
        .iter()
        .map(|call| {
            let call_id = if call.call_id.len() > 15 {
                format!("{}...", &call.call_id[..12])
            } else {
                call.call_id.clone()
            };

            let media_session = call
                .media_session_id
                .as_ref()
                .map(|s| {
                    if s.len() > 12 {
                        format!("{}...", &s[..9])
                    } else {
                        s.clone()
                    }
                })
                .unwrap_or_else(|| "-".to_string());

            let a_leg = call
                .a_leg_dialog_id
                .as_ref()
                .map(|s| {
                    if s.len() > 12 {
                        format!("{}...", &s[..9])
                    } else {
                        s.clone()
                    }
                })
                .unwrap_or_else(|| "-".to_string());

            let b_leg = call
                .b_leg_dialog_id
                .as_ref()
                .map(|s| {
                    if s.len() > 12 {
                        format!("{}...", &s[..9])
                    } else {
                        s.clone()
                    }
                })
                .unwrap_or_else(|| "-".to_string());

            Row::new(vec![
                Cell::from(call_id),
                Cell::from(call.state.clone()),
                Cell::from(media_session),
                Cell::from(a_leg),
                Cell::from(b_leg),
            ])
        })
        .collect();

    let widths = [
        Constraint::Percentage(18),
        Constraint::Percentage(15),
        Constraint::Percentage(22),
        Constraint::Percentage(22),
        Constraint::Percentage(23),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .title(format!(" Debug Calls ({}) ", state.debug_calls.len()))
                .borders(Borders::ALL)
                .border_style(theme.border())
                .title_style(theme.title()),
        )
        .row_highlight_style(theme.selected())
        .highlight_symbol(">> ");

    f.render_stateful_widget(table, area, &mut state.table_state);
}

/// Format request counts as "METHOD:count, METHOD:count"
fn format_counts(counts: &std::collections::HashMap<String, u32>) -> String {
    if counts.is_empty() {
        return "-".to_string();
    }

    let mut pairs: Vec<_> = counts.iter().collect();
    pairs.sort_by_key(|(k, _)| *k);

    pairs
        .iter()
        .map(|(method, count)| format!("{}:{}", method, count))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Handle navigation events for debug view
pub fn handle_navigation(state: &mut DebugViewState, event: &crate::tui::event::Event) {
    if event.is_up() {
        state.select_previous();
    } else if event.is_down() {
        state.select_next();
    }
}

/// Handle tab navigation for debug sub-views
pub fn handle_tab_navigation(state: &mut DebugViewState, event: &crate::tui::event::Event) {
    if event.is_left() {
        state.prev_subview();
    } else if event.is_right() {
        state.next_subview();
    } else if event.is_tab() {
        // Tab key also moves to next sub-view
        state.next_subview();
    }
}

/// Fetch all debug data
pub async fn fetch_debug_data(api_client: &crate::api::ApiClient, state: &mut DebugViewState) {
    state.loading = true;
    state.error = None;

    // Fetch summary
    match api_client.get_debug_summary().await {
        Ok(summary) => {
            state.summary = Some(summary);
        }
        Err(e) => {
            state.error = Some(format!("Failed to fetch debug summary: {}", e));
        }
    }

    // Fetch dialogs
    match api_client.list_dialogs().await {
        Ok(dialogs) => {
            state.dialogs = dialogs;
        }
        Err(_) => {
            // Silently skip if dialogs endpoint fails
        }
    }

    // Fetch subscriptions
    match api_client.list_subscriptions().await {
        Ok(subscriptions) => {
            state.subscriptions = subscriptions;
        }
        Err(_) => {
            // Silently skip if subscriptions endpoint fails
        }
    }

    // Fetch rate limits
    match api_client.list_rate_limits().await {
        Ok(rate_limits) => {
            state.rate_limits = rate_limits;
        }
        Err(_) => {
            // Silently skip if rate limits endpoint fails
        }
    }

    // Fetch debug calls
    match api_client.list_debug_calls().await {
        Ok(debug_calls) => {
            state.debug_calls = debug_calls;
        }
        Err(_) => {
            // Silently skip if debug calls endpoint fails
        }
    }

    state.loading = false;
}
