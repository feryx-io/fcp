//! Calls view showing active calls and details

use crate::api::CallResponse;
use crate::tui::event::{Event, EventResult, ViewContext};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};
use std::time::Duration;

/// State for the calls view
pub struct CallsViewState {
    /// List of calls
    pub calls: Vec<CallResponse>,
    /// Table state for selection
    pub table_state: TableState,
    /// Last error message
    pub error: Option<String>,
    /// Loading state
    pub loading: bool,
}

impl Default for CallsViewState {
    fn default() -> Self {
        let mut table_state = TableState::default();
        table_state.select(Some(0));

        Self {
            calls: Vec::new(),
            table_state,
            error: None,
            loading: false,
        }
    }
}

impl CallsViewState {
    /// Get currently selected call
    pub fn selected_call(&self) -> Option<&CallResponse> {
        self.table_state.selected().and_then(|i| self.calls.get(i))
    }

    /// Move selection up
    pub fn select_previous(&mut self) {
        if self.calls.is_empty() {
            return;
        }

        let selected = self.table_state.selected().unwrap_or(0);
        if selected > 0 {
            self.table_state.select(Some(selected - 1));
        }
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        if self.calls.is_empty() {
            return;
        }

        let selected = self.table_state.selected().unwrap_or(0);
        if selected < self.calls.len() - 1 {
            self.table_state.select(Some(selected + 1));
        }
    }

    /// Update calls list
    pub fn set_calls(&mut self, calls: Vec<CallResponse>) {
        self.calls = calls;
        // Reset selection to first item if we had no selection or it's out of bounds
        if self.table_state.selected().is_none()
            || self.table_state.selected().unwrap() >= self.calls.len()
        {
            self.table_state
                .select(if self.calls.is_empty() { None } else { Some(0) });
        }
    }
}

/// Render the calls view
pub fn render(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &mut CallsViewState,
    area: Rect,
) {
    // Split into table and details panel
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(60), // Calls table
            Constraint::Percentage(40), // Details panel
        ])
        .split(area);

    render_calls_table(f, theme, state, chunks[0]);
    render_details_panel(f, theme, state, chunks[1]);
}

/// Render the calls table
fn render_calls_table(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &mut CallsViewState,
    area: Rect,
) {
    // Check if any calls have node_id (aggregated view)
    let has_node_info = state.calls.iter().any(|c| c.node_id.is_some());

    let header = if has_node_info {
        Row::new(vec!["Call ID", "Node", "From → To", "State", "Duration"])
            .style(theme.title())
            .bottom_margin(1)
    } else {
        Row::new(vec!["Call ID", "From → To", "State", "Duration", "Media"])
            .style(theme.title())
            .bottom_margin(1)
    };

    let rows: Vec<Row> = state
        .calls
        .iter()
        .map(|call| {
            // Format call ID (show first 8 chars)
            let short_id = if call.id.len() > 8 {
                format!("{}...", &call.id[..8])
            } else {
                call.id.clone()
            };

            // Format from → to with display names if available
            let from = call
                .from_display_name
                .as_ref()
                .map(|n| format!("{} <{}>", n, call.from_uri))
                .unwrap_or_else(|| call.from_uri.clone());
            let to = call
                .to_display_name
                .as_ref()
                .map(|n| format!("{} <{}>", n, call.to_uri))
                .unwrap_or_else(|| call.to_uri.clone());
            let direction = format!("{} → {}", from, to);

            // Format duration
            let duration = if let Some(secs) = call.duration_seconds {
                format_duration(secs)
            } else {
                "-".to_string()
            };

            if has_node_info {
                // Show node ID (short form)
                let node = call.node_id.as_ref()
                    .map(|n| if n.len() > 12 { format!("{}...", &n[..12]) } else { n.clone() })
                    .unwrap_or_else(|| "-".to_string());
                Row::new(vec![
                    Cell::from(short_id),
                    Cell::from(node),
                    Cell::from(direction),
                    Cell::from(call.state.clone()),
                    Cell::from(duration),
                ])
            } else {
                // Media status
                let media = if call.media_anchored { "✓" } else { "✗" };
                Row::new(vec![
                    Cell::from(short_id),
                    Cell::from(direction),
                    Cell::from(call.state.clone()),
                    Cell::from(duration),
                    Cell::from(media),
                ])
            }
        })
        .collect();

    let widths = if has_node_info {
        [
            Constraint::Percentage(12), // Call ID
            Constraint::Percentage(15), // Node
            Constraint::Percentage(43), // From → To
            Constraint::Percentage(15), // State
            Constraint::Percentage(15), // Duration
        ]
    } else {
        [
            Constraint::Percentage(15), // Call ID
            Constraint::Percentage(45), // From → To
            Constraint::Percentage(15), // State
            Constraint::Percentage(15), // Duration
            Constraint::Percentage(10), // Media
        ]
    };

    let title = if state.loading {
        " Active Calls (Loading...) "
    } else {
        format!(" Active Calls ({}) ", state.calls.len()).leak() as &str
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

/// Render the details panel for selected call
fn render_details_panel(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &CallsViewState,
    area: Rect,
) {
    let details_text = if let Some(call) = state.selected_call() {
        let mut lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("Call ID: ", theme.muted()),
                Span::styled(&call.id, theme.normal()),
            ]),
        ];

        // Show node ID if present (aggregated view)
        if let Some(ref node_id) = call.node_id {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("Node: ", theme.muted()),
                Span::styled(node_id, theme.info()),
            ]));
        }

        lines.extend(vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("State: ", theme.muted()),
                Span::styled(
                    &call.state,
                    match call.state.as_str() {
                        "answered" | "established" => theme.success(),
                        "ended" | "terminated" => theme.muted(),
                        "failed" => theme.error(),
                        "ringing" => theme.info(),
                        _ => theme.normal(),
                    },
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("From: ", theme.muted()),
                Span::styled(&call.from_uri, theme.normal()),
            ]),
            Line::from(vec![
                Span::styled("  Display: ", theme.muted()),
                Span::styled(
                    call.from_display_name.as_deref().unwrap_or("-"),
                    theme.normal(),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("To: ", theme.muted()),
                Span::styled(&call.to_uri, theme.normal()),
            ]),
            Line::from(vec![
                Span::styled("  Display: ", theme.muted()),
                Span::styled(
                    call.to_display_name.as_deref().unwrap_or("-"),
                    theme.normal(),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Duration: ", theme.muted()),
                Span::styled(
                    if let Some(secs) = call.duration_seconds {
                        format_duration(secs)
                    } else {
                        "-".to_string()
                    },
                    theme.normal(),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Media Anchored: ", theme.muted()),
                Span::styled(
                    if call.media_anchored { "Yes" } else { "No" },
                    if call.media_anchored {
                        theme.success()
                    } else {
                        theme.muted()
                    },
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Created: ", theme.muted()),
                Span::styled(&call.created_at, theme.normal()),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Established: ", theme.muted()),
                Span::styled(
                    call.established_at.as_deref().unwrap_or("-"),
                    theme.normal(),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Ended: ", theme.muted()),
                Span::styled(call.ended_at.as_deref().unwrap_or("-"), theme.normal()),
            ]),
        ]);
        lines
    } else if state.calls.is_empty() {
        vec![
            Line::from(""),
            Line::from(vec![Span::styled("  No active calls", theme.muted())]),
            Line::from(""),
            Line::from(vec![Span::styled("  Press 'r' to refresh", theme.info())]),
        ]
    } else {
        vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "  Select a call to view details",
                theme.muted(),
            )]),
        ]
    };

    // Add failure reason if present
    let mut final_text = details_text;
    if let Some(call) = state.selected_call() {
        if let Some(reason) = &call.failure_reason {
            final_text.push(Line::from(""));
            final_text.push(Line::from(vec![
                Span::styled("Failure Reason: ", theme.error()),
                Span::styled(reason, theme.error()),
            ]));
        }
    }

    let details = Paragraph::new(final_text).block(
        Block::default()
            .title(" Call Details ")
            .borders(Borders::ALL)
            .border_style(theme.border())
            .title_style(theme.title()),
    );

    f.render_widget(details, area);
}

/// Format duration in HH:MM:SS or MM:SS
fn format_duration(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;

    if hours > 0 {
        format!("{:02}:{:02}:{:02}", hours, minutes, secs)
    } else {
        format!("{:02}:{:02}", minutes, secs)
    }
}

/// Handle all events for the calls view
pub async fn handle_event(
    state: &mut CallsViewState,
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

/// Check if the calls view needs initial data loading
pub fn needs_data(state: &CallsViewState) -> bool {
    state.calls.is_empty() && !state.loading
}

/// Refresh interval for calls (fast refresh for live durations)
pub fn refresh_interval() -> Duration {
    Duration::from_secs(2)
}

/// Fetch calls data
/// If events_url is provided, fetches aggregated calls from the events server.
/// Otherwise, falls back to the local call-manager API.
pub async fn fetch_data(
    api_client: &crate::api::ApiClient,
    state: &mut CallsViewState,
    events_url: Option<&str>,
) {
    state.loading = true;
    state.error = None;

    // Try events server first if URL is configured
    if let Some(url) = events_url {
        match api_client.list_aggregated_calls(url, None).await {
            Ok(aggregated_calls) => {
                // Convert aggregated calls to CallResponse format
                let calls: Vec<CallResponse> = aggregated_calls
                    .into_iter()
                    .map(|c| c.to_call_response())
                    .collect();
                state.set_calls(calls);
                state.loading = false;
                return;
            }
            Err(e) => {
                // Log warning but fall back to regular API
                tracing::warn!("Failed to fetch from events server, falling back: {}", e);
            }
        }
    }

    // Fall back to local call-manager API
    match api_client.list_calls().await {
        Ok(calls) => {
            state.set_calls(calls);
        }
        Err(e) => {
            state.error = Some(format!("Failed to fetch calls: {}", e));
        }
    }

    state.loading = false;
}

