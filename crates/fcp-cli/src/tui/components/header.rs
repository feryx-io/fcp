//! Header component showing context, status, and tabs

use crate::tui::app::App;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Tabs},
    Frame,
};

/// Render the header component
pub fn render(f: &mut Frame, app: &App, area: Rect) {
    // Split header into two rows: status bar and tabs
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(3)])
        .split(area);

    render_status_bar(f, app, chunks[0]);
    render_tabs(f, app, chunks[1]);
}

/// Render the status bar showing context, URL, mode, role, and clock
fn render_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let now = chrono::Local::now();
    let time_str = now.format("%H:%M:%S").to_string();

    let status_line = Line::from(vec![
        Span::styled("Context: ", app.theme.muted()),
        Span::styled(&app.context.name, app.theme.primary()),
        Span::raw(" | "),
        Span::styled("Mode: ", app.theme.muted()),
        Span::styled(app.deployment_mode.name(), app.theme.info()),
        Span::raw(" | "),
        Span::styled("User: ", app.theme.muted()),
        Span::styled(&app.current_user.username, app.theme.success()),
        Span::raw(" ("),
        Span::styled(app.current_user.role.name(), app.theme.warning()),
        Span::raw(") | "),
        Span::styled(&time_str, app.theme.info()),
    ]);

    let status = Paragraph::new(status_line)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border()),
        )
        .alignment(Alignment::Left);

    f.render_widget(status, area);
}

/// Render the tabs bar
fn render_tabs(f: &mut Frame, app: &App, area: Rect) {
    // Get only tabs accessible to current user
    let accessible_tabs = app.accessible_tabs();

    let tab_titles: Vec<_> = accessible_tabs.iter().map(|t| t.title()).collect();

    let selected_idx = accessible_tabs
        .iter()
        .position(|t| t == &app.active_tab)
        .unwrap_or(0);

    let tabs = Tabs::new(tab_titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border()),
        )
        .select(selected_idx)
        .style(app.theme.normal())
        .highlight_style(app.theme.selected());

    f.render_widget(tabs, area);
}
