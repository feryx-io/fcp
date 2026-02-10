//! Event handling for the TUI

use anyhow::Result;
use crossterm::event::{self, Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers};
use std::time::{Duration, Instant};

use super::app::CurrentUser;
use fcp_core::DeploymentMode;

/// Result of a view handling an event
pub enum EventResult {
    /// The event was consumed by the view
    Consumed,
    /// The event was not consumed (should be handled globally)
    NotConsumed,
    /// The event was consumed but produced an error
    Error(String),
}

/// Context passed to views for permission checks
pub struct ViewContext {
    pub current_user: CurrentUser,
    pub deployment_mode: DeploymentMode,
}

/// Events that can occur in the TUI
#[derive(Debug, Clone)]
pub enum Event {
    /// A key was pressed
    Key(KeyEvent),
    /// Tick event for periodic updates
    Tick,
    /// Terminal was resized
    #[allow(dead_code)]
    Resize(u16, u16),
}

/// Event handler for the TUI
pub struct EventHandler {
    /// Last tick time
    last_tick: Instant,
    /// Tick rate
    tick_rate: Duration,
}

impl EventHandler {
    /// Create a new event handler with the given tick rate
    pub fn new(tick_rate: Duration) -> Self {
        Self {
            last_tick: Instant::now(),
            tick_rate,
        }
    }

    /// Wait for the next event
    pub fn next(&mut self) -> Result<Event> {
        let timeout = self
            .tick_rate
            .checked_sub(self.last_tick.elapsed())
            .unwrap_or(Duration::from_secs(0));

        if event::poll(timeout)? {
            match event::read()? {
                CrosstermEvent::Key(key) => Ok(Event::Key(key)),
                CrosstermEvent::Resize(w, h) => Ok(Event::Resize(w, h)),
                _ => Ok(Event::Tick), // Ignore mouse and other events for now
            }
        } else {
            // Timeout elapsed, emit tick
            self.last_tick = Instant::now();
            Ok(Event::Tick)
        }
    }
}

/// Key binding helpers
impl Event {
    /// Check if this is a quit event (q or Ctrl+C)
    pub fn is_quit(&self) -> bool {
        matches!(
            self,
            Event::Key(KeyEvent {
                code: KeyCode::Char('q'),
                modifiers: KeyModifiers::NONE,
                ..
            }) | Event::Key(KeyEvent {
                code: KeyCode::Char('c'),
                modifiers: KeyModifiers::CONTROL,
                ..
            })
        )
    }

    /// Check if this is a help event (?)
    pub fn is_help(&self) -> bool {
        matches!(
            self,
            Event::Key(KeyEvent {
                code: KeyCode::Char('?'),
                modifiers: KeyModifiers::NONE,
                ..
            })
        )
    }

    /// Check if this is a refresh event (r)
    pub fn is_refresh(&self) -> bool {
        matches!(
            self,
            Event::Key(KeyEvent {
                code: KeyCode::Char('r'),
                modifiers: KeyModifiers::NONE,
                ..
            })
        )
    }

    /// Check if this is a left navigation event (h or Left arrow)
    pub fn is_left(&self) -> bool {
        matches!(
            self,
            Event::Key(KeyEvent {
                code: KeyCode::Char('h') | KeyCode::Left,
                modifiers: KeyModifiers::NONE,
                ..
            })
        )
    }

    /// Check if this is a right navigation event (l or Right arrow)
    pub fn is_right(&self) -> bool {
        matches!(
            self,
            Event::Key(KeyEvent {
                code: KeyCode::Char('l') | KeyCode::Right,
                modifiers: KeyModifiers::NONE,
                ..
            })
        )
    }

    /// Check if this is a shift+left navigation event (H or Shift+Left arrow)
    pub fn is_shift_left(&self) -> bool {
        matches!(
            self,
            Event::Key(KeyEvent {
                code: KeyCode::Char('H') | KeyCode::Left,
                modifiers: KeyModifiers::SHIFT,
                ..
            })
        )
    }

    /// Check if this is a shift+right navigation event (L or Shift+Right arrow)
    pub fn is_shift_right(&self) -> bool {
        matches!(
            self,
            Event::Key(KeyEvent {
                code: KeyCode::Char('L') | KeyCode::Right,
                modifiers: KeyModifiers::SHIFT,
                ..
            })
        )
    }

    /// Check if this is an up navigation event (k or Up arrow)
    pub fn is_up(&self) -> bool {
        matches!(
            self,
            Event::Key(KeyEvent {
                code: KeyCode::Char('k') | KeyCode::Up,
                modifiers: KeyModifiers::NONE,
                ..
            })
        )
    }

    /// Check if this is a down navigation event (j or Down arrow)
    pub fn is_down(&self) -> bool {
        matches!(
            self,
            Event::Key(KeyEvent {
                code: KeyCode::Char('j') | KeyCode::Down,
                modifiers: KeyModifiers::NONE,
                ..
            })
        )
    }

    /// Check if this is an enter event
    #[allow(dead_code)]
    pub fn is_enter(&self) -> bool {
        matches!(
            self,
            Event::Key(KeyEvent {
                code: KeyCode::Enter,
                ..
            })
        )
    }

    /// Check if this is a tab event
    pub fn is_tab(&self) -> bool {
        matches!(
            self,
            Event::Key(KeyEvent {
                code: KeyCode::Tab,
                modifiers: KeyModifiers::NONE,
                ..
            })
        )
    }
}
