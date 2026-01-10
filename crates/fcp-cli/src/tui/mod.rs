//! Terminal User Interface module

mod app;
mod components;
mod event;
mod theme;
mod views;

use anyhow::Result;
use app::{App, Tab};
use crossterm::{
    event::{KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use event::{Event, EventHandler};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    Terminal,
};
use std::{io, time::Duration};

use crate::config::ResolvedContext;

/// Run the TUI application
pub async fn run(context: ResolvedContext) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let mut app = App::new(context)?;
    let mut event_handler = EventHandler::new(Duration::from_millis(250)); // 4 updates per second

    // Main loop
    let result = run_app(&mut terminal, &mut app, &mut event_handler).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

/// Main application loop
async fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    event_handler: &mut EventHandler,
) -> Result<()>
where
    <B as ratatui::backend::Backend>::Error: std::error::Error + Send + Sync + 'static,
{
    loop {
        // Draw UI
        terminal.draw(|f| {
            render_ui(f, app);
        })?;

        // Handle events
        let event = event_handler.next()?;
        handle_event(app, event).await?;

        // Check if we should quit
        if app.should_quit {
            break;
        }
    }

    Ok(())
}

/// Render the main UI
fn render_ui(f: &mut ratatui::Frame, app: &mut App) {
    // Create main layout
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6), // Header (status + tabs)
            Constraint::Min(10),   // Content area
            Constraint::Length(3), // Footer
        ])
        .split(f.area());

    // Render components
    components::header::render(f, app, chunks[0]);
    render_content(f, app, chunks[1]);
    components::footer::render(f, app, chunks[2]);
}

/// Render the content area based on active tab
fn render_content(f: &mut ratatui::Frame, app: &mut App, area: ratatui::layout::Rect) {
    match app.active_tab {
        Tab::Dashboard => views::dashboard::render(f, app, area),
        Tab::Users => views::users::render(f, &app.theme, &mut app.users_view, area),
        Tab::Calls => views::calls::render(f, &app.theme, &mut app.calls_view, area),
        Tab::Registrations => {
            views::registrations::render(f, &app.theme, &mut app.registrations_view, area)
        }
        Tab::Debug => views::debug::render(f, &app.theme, &mut app.debug_view, area),
    }
}

/// Render a placeholder for views not yet implemented
fn render_placeholder(
    f: &mut ratatui::Frame,
    app: &App,
    area: ratatui::layout::Rect,
    title: &str,
) {
    use ratatui::{text::{Line, Span}, widgets::{Block, Borders, Paragraph}};

    let text = vec![
        Line::from(""),
        Line::from(vec![Span::styled(
            format!("  {} - Coming Soon  ", title),
            app.theme.title(),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  This view will be implemented in Phase 4",
            app.theme.muted(),
        )]),
    ];

    let paragraph = Paragraph::new(text)
        .block(
            Block::default()
                .title(format!(" {} ", title))
                .borders(Borders::ALL)
                .border_style(app.theme.border())
                .title_style(app.theme.title()),
        )
        .alignment(ratatui::layout::Alignment::Center);

    f.render_widget(paragraph, area);
}

/// Handle an event
async fn handle_event(app: &mut App, event: Event) -> Result<()> {
    // Handle user modal events first - if a modal is open, it gets ALL keyboard events
    if app.active_tab == Tab::Users && app.users_view.modal != crate::tui::views::users::UserModal::None {
        let handled = views::users::handle_user_action(&mut app.users_view, &app.api_client, &event).await;
        if handled {
            return Ok(());
        }
    }

    match event {
        Event::Key(_) if event.is_quit() => {
            app.quit();
        }
        Event::Key(KeyEvent { code: KeyCode::Char(c), modifiers: KeyModifiers::NONE, .. }) if matches!(c, '1'..='5') => {
            // Number keys 1-5 jump directly to main tabs
            if let Some(tab) = Tab::from_index((c as usize) - ('1' as usize)) {
                app.set_tab(tab);
                check_and_load_tab_data(app).await;
            }
        }
        Event::Key(_) if event.is_shift_left() => {
            // Shift+Left always navigates main tabs
            app.prev_tab();
            check_and_load_tab_data(app).await;
        }
        Event::Key(_) if event.is_shift_right() => {
            // Shift+Right always navigates main tabs
            app.next_tab();
            check_and_load_tab_data(app).await;
        }
        Event::Key(_) if event.is_left() => {
            // Special handling for Debug tab - navigate between sub-views
            if app.active_tab == Tab::Debug {
                views::debug::handle_tab_navigation(&mut app.debug_view, &event);
            } else {
                app.prev_tab();
                // Load data when switching to a new tab
                check_and_load_tab_data(app).await;
            }
        }
        Event::Key(_) if event.is_right() => {
            // Special handling for Debug tab - navigate between sub-views
            if app.active_tab == Tab::Debug {
                views::debug::handle_tab_navigation(&mut app.debug_view, &event);
            } else {
                app.next_tab();
                // Load data when switching to a new tab
                check_and_load_tab_data(app).await;
            }
        }
        Event::Key(_) if event.is_refresh() => {
            app.mark_update();
            // Trigger data refresh for current view
            match app.active_tab {
                Tab::Users => {
                    views::users::fetch_users(&app.api_client, &mut app.users_view).await;
                }
                Tab::Calls => {
                    views::calls::fetch_calls(&app.api_client, &mut app.calls_view).await;
                }
                Tab::Registrations => {
                    views::registrations::fetch_registrations(
                        &app.api_client,
                        &mut app.registrations_view,
                    )
                    .await;
                }
                Tab::Debug => {
                    views::debug::fetch_debug_data(&app.api_client, &mut app.debug_view).await;
                }
                _ => {}
            }
        }
        Event::Key(_) if event.is_help() => {
            // TODO: Show help overlay
        }
        Event::Key(_) if event.is_up() || event.is_down() => {
            // Delegate navigation to view-specific handlers
            match app.active_tab {
                Tab::Users => {
                    views::users::handle_navigation(&mut app.users_view, &event);
                }
                Tab::Calls => {
                    views::calls::handle_navigation(&mut app.calls_view, &event);
                }
                Tab::Registrations => {
                    views::registrations::handle_navigation(&mut app.registrations_view, &event);
                }
                Tab::Debug => {
                    views::debug::handle_navigation(&mut app.debug_view, &event);
                }
                _ => {}
            }
        }
        Event::Key(_) if event.is_tab() => {
            // Handle tab within Debug view for sub-view navigation
            if app.active_tab == Tab::Debug {
                views::debug::handle_tab_navigation(&mut app.debug_view, &event);
            }
        }
        Event::Key(_) => {
            // Only handle these keys if no modal is open (modal events handled at top of function)
            if app.active_tab == Tab::Users && app.users_view.modal == crate::tui::views::users::UserModal::None {
                match event {
                    Event::Key(KeyEvent { code: KeyCode::Char('a'), modifiers: KeyModifiers::NONE, .. }) => {
                        app.users_view.open_add_modal();
                    }
                    Event::Key(KeyEvent { code: KeyCode::Char('e'), modifiers: KeyModifiers::NONE, .. }) => {
                        app.users_view.open_edit_modal();
                    }
                    Event::Key(KeyEvent { code: KeyCode::Char('d'), modifiers: KeyModifiers::NONE, .. }) => {
                        app.users_view.open_delete_modal();
                    }
                    _ => {}
                }
            }
        }
        Event::Tick => {
            // Periodic update - auto-refresh data based on view requirements
            let time_since = app.time_since_update();
            let should_refresh = match app.active_tab {
                Tab::Calls => time_since > std::time::Duration::from_secs(2), // Fast refresh for live durations
                Tab::Registrations => time_since > std::time::Duration::from_secs(3), // Medium refresh for expiration countdown
                Tab::Debug => time_since > std::time::Duration::from_secs(4), // Debug refresh
                Tab::Users | Tab::Dashboard => time_since > std::time::Duration::from_secs(5), // Slower refresh
            };

            if should_refresh {
                app.mark_update();
                match app.active_tab {
                    Tab::Users => {
                        views::users::fetch_users(&app.api_client, &mut app.users_view).await;
                    }
                    Tab::Calls => {
                        views::calls::fetch_calls(&app.api_client, &mut app.calls_view).await;
                    }
                    Tab::Registrations => {
                        views::registrations::fetch_registrations(
                            &app.api_client,
                            &mut app.registrations_view,
                        )
                        .await;
                    }
                    Tab::Debug => {
                        views::debug::fetch_debug_data(&app.api_client, &mut app.debug_view)
                            .await;
                    }
                    _ => {}
                }
            }
        }
        Event::Resize(_, _) => {
            // Terminal was resized, will redraw automatically
        }
        _ => {}
    }

    Ok(())
}

/// Check if the current tab needs data loading and load it
async fn check_and_load_tab_data(app: &mut App) {
    match app.active_tab {
        Tab::Users => {
            // Load users if list is empty or not loading
            if app.users_view.users.is_empty() && !app.users_view.loading {
                views::users::fetch_users(&app.api_client, &mut app.users_view).await;
            }
        }
        Tab::Calls => {
            // Load calls if list is empty or not loading
            if app.calls_view.calls.is_empty() && !app.calls_view.loading {
                views::calls::fetch_calls(&app.api_client, &mut app.calls_view).await;
            }
        }
        Tab::Registrations => {
            // Load registrations if list is empty or not loading
            if app.registrations_view.registrations.is_empty()
                && !app.registrations_view.loading
            {
                views::registrations::fetch_registrations(
                    &app.api_client,
                    &mut app.registrations_view,
                )
                .await;
            }
        }
        Tab::Debug => {
            // Load debug data if summary is not available or not loading
            if app.debug_view.summary.is_none() && !app.debug_view.loading {
                views::debug::fetch_debug_data(&app.api_client, &mut app.debug_view).await;
            }
        }
        _ => {}
    }
}
