//! TUI theme and color scheme

use ratatui::style::{Color, Modifier, Style};

/// Application theme with color definitions
#[derive(Debug, Clone)]
pub struct Theme {
    /// Primary accent color
    pub primary: Color,
    /// Success/positive color
    pub success: Color,
    /// Warning color
    pub warning: Color,
    /// Error/danger color
    pub error: Color,
    /// Info color
    pub info: Color,
    /// Muted/secondary text color
    pub muted: Color,
    /// Border color
    pub border: Color,
    /// Selected item background
    pub selected_bg: Color,
    /// Selected item foreground
    pub selected_fg: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            primary: Color::Cyan,
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,
            info: Color::Blue,
            muted: Color::DarkGray,
            border: Color::Gray,
            selected_bg: Color::Cyan,
            selected_fg: Color::Black,
        }
    }
}

impl Theme {
    /// Get style for normal text
    pub fn normal(&self) -> Style {
        Style::default()
    }

    /// Get style for bold text
    pub fn bold(&self) -> Style {
        Style::default().add_modifier(Modifier::BOLD)
    }

    /// Get style for primary colored text
    pub fn primary(&self) -> Style {
        Style::default().fg(self.primary)
    }

    /// Get style for success colored text
    pub fn success(&self) -> Style {
        Style::default().fg(self.success)
    }

    /// Get style for warning colored text
    pub fn warning(&self) -> Style {
        Style::default().fg(self.warning)
    }

    /// Get style for error colored text
    pub fn error(&self) -> Style {
        Style::default().fg(self.error)
    }

    /// Get style for info colored text
    pub fn info(&self) -> Style {
        Style::default().fg(self.info)
    }

    /// Get style for muted text
    pub fn muted(&self) -> Style {
        Style::default().fg(self.muted)
    }

    /// Get style for borders
    pub fn border(&self) -> Style {
        Style::default().fg(self.border)
    }

    /// Get style for selected items
    pub fn selected(&self) -> Style {
        Style::default()
            .bg(self.selected_bg)
            .fg(self.selected_fg)
            .add_modifier(Modifier::BOLD)
    }

    /// Get style for titles/headers
    pub fn title(&self) -> Style {
        Style::default()
            .fg(self.primary)
            .add_modifier(Modifier::BOLD)
    }
}
