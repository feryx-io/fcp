//! Footer component showing keybindings

use crate::tui::app::App;
use ratatui::{
    layout::{Alignment, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// Render the footer component
pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let keybindings = vec![
        ("h/l", "Tabs"),
        ("j/k", "Navigate"),
        ("Enter", "Select"),
        ("r", "Refresh"),
        ("?", "Help"),
        ("q", "Quit"),
    ];

    let spans: Vec<_> = keybindings
        .iter()
        .flat_map(|(key, desc)| {
            vec![
                Span::styled(*key, app.theme.primary().add_modifier(ratatui::style::Modifier::BOLD)),
                Span::raw(": "),
                Span::styled(*desc, app.theme.normal()),
                Span::raw("  "),
            ]
        })
        .collect();

    let keybindings_line = Line::from(spans);

    let footer = Paragraph::new(keybindings_line)
        .block(Block::default().borders(Borders::ALL).border_style(app.theme.border()))
        .alignment(Alignment::Center);

    f.render_widget(footer, area);
}
