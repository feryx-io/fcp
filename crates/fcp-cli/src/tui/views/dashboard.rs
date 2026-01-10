//! Dashboard view showing system overview

use crate::tui::app::App;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// Render the dashboard view
pub fn render(f: &mut Frame, app: &App, area: Rect) {
    // Split into sections
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),  // Status section
            Constraint::Min(10),     // Stats section
        ])
        .split(area);

    render_status_section(f, app, chunks[0]);
    render_stats_section(f, app, chunks[1]);
}

/// Render the status section
fn render_status_section(f: &mut Frame, app: &App, area: Rect) {
    let status_text = vec![
        Line::from(vec![
            Span::styled("Server Status: ", app.theme.muted()),
            Span::styled("Connected", app.theme.success()),
        ]),
        Line::from(vec![
            Span::styled("Last Update: ", app.theme.muted()),
            Span::styled(
                format!("{:.1}s ago", app.time_since_update().as_secs_f64()),
                app.theme.normal(),
            ),
        ]),
    ];

    let status = Paragraph::new(status_text)
        .block(
            Block::default()
                .title(" Status ")
                .borders(Borders::ALL)
                .border_style(app.theme.border())
                .title_style(app.theme.title()),
        )
        .alignment(Alignment::Left);

    f.render_widget(status, area);
}

/// Render the stats section
fn render_stats_section(f: &mut Frame, app: &App, area: Rect) {
    let stats_text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Welcome to FCP CLI TUI  ", app.theme.title()),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Use ", app.theme.normal()),
            Span::styled("h/l", app.theme.primary()),
            Span::styled(" or ", app.theme.normal()),
            Span::styled("←/→", app.theme.primary()),
            Span::styled(" to switch tabs", app.theme.normal()),
        ]),
        Line::from(vec![
            Span::styled("  Press ", app.theme.normal()),
            Span::styled("r", app.theme.primary()),
            Span::styled(" to refresh data", app.theme.normal()),
        ]),
        Line::from(vec![
            Span::styled("  Press ", app.theme.normal()),
            Span::styled("q", app.theme.primary()),
            Span::styled(" to quit", app.theme.normal()),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Statistics will appear here once data is loaded", app.theme.muted()),
        ]),
    ];

    let stats = Paragraph::new(stats_text)
        .block(
            Block::default()
                .title(" Dashboard ")
                .borders(Borders::ALL)
                .border_style(app.theme.border())
                .title_style(app.theme.title()),
        )
        .alignment(Alignment::Center);

    f.render_widget(stats, area);
}
