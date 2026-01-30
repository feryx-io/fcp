//! Footer component showing keybindings

use crate::tui::app::{App, Tab};
use ratatui::{
    layout::{Alignment, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// Render the footer component
pub fn render(f: &mut Frame, app: &App, area: Rect) {
    // If there's an error, show it instead of keybindings
    if let Some(error) = &app.error {
        let error_line = Line::from(vec![
            Span::styled(
                "ERROR: ",
                app.theme
                    .error()
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ),
            Span::styled(error, app.theme.error()),
            Span::raw("  "),
            Span::styled("(Press Esc to dismiss)", app.theme.muted()),
        ]);

        let footer = Paragraph::new(error_line)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(app.theme.error()),
            )
            .alignment(Alignment::Left);

        f.render_widget(footer, area);
        return;
    }

    // Otherwise show keybindings
    let keybindings = match app.active_tab {
        Tab::Users => vec![
            ("h/l", "Tabs"),
            ("j/k", "Navigate"),
            ("a", "Add"),
            ("e", "Edit"),
            ("d", "Delete"),
            ("s", "Set Digest"),
            ("c", "Clear Digest"),
            ("r", "Refresh"),
            ("q", "Quit"),
        ],
        _ => vec![
            ("h/l", "Tabs"),
            ("j/k", "Navigate"),
            ("Enter", "Select"),
            ("r", "Refresh"),
            ("?", "Help"),
            ("q", "Quit"),
        ],
    };

    let spans: Vec<_> = keybindings
        .iter()
        .flat_map(|(key, desc)| {
            vec![
                Span::styled(
                    *key,
                    app.theme
                        .primary()
                        .add_modifier(ratatui::style::Modifier::BOLD),
                ),
                Span::raw(": "),
                Span::styled(*desc, app.theme.normal()),
                Span::raw("  "),
            ]
        })
        .collect();

    let keybindings_line = Line::from(spans);

    let footer = Paragraph::new(keybindings_line)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border()),
        )
        .alignment(Alignment::Center);

    f.render_widget(footer, area);
}
