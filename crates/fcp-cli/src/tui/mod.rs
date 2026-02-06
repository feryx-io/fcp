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
        Tab::Tenants => views::tenants::render(f, area, &mut app.tenants_view),
        Tab::Users => views::users::render(f, &app.theme, &mut app.users_view, area),
        Tab::PhoneNumbers => views::phone_numbers::render(
            f,
            &app.theme,
            &mut app.phone_numbers_view,
            area,
            app.deployment_mode,
        ),
        Tab::Devices => views::devices::render(f, &app.theme, &mut app.devices_view, area),
        Tab::LineBindings => {
            views::line_bindings::render(f, area, &app.line_bindings_view, &app.theme)
        }
        Tab::Calls => views::calls::render(f, &app.theme, &mut app.calls_view, area),
        Tab::Registrations => {
            views::registrations::render(f, &app.theme, &mut app.registrations_view, area)
        }
        Tab::Debug => views::debug::render(f, &app.theme, &mut app.debug_view, area),
    }
}

/// Render a placeholder for views not yet implemented
fn render_placeholder(f: &mut ratatui::Frame, app: &App, area: ratatui::layout::Rect, title: &str) {
    use ratatui::{
        text::{Line, Span},
        widgets::{Block, Borders, Paragraph},
    };

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
    use event::EventResult;

    // Step 1: Delegate to the active view's handle_event() first
    let ctx = app.view_context();
    let result = match app.active_tab {
        Tab::Dashboard => views::dashboard::handle_event(&app.api_client, &event, &ctx).await,
        Tab::Tenants => {
            views::tenants::handle_event(&mut app.tenants_view, &app.api_client, &event, &ctx)
                .await
        }
        Tab::Users => {
            views::users::handle_event(&mut app.users_view, &app.api_client, &event, &ctx).await
        }
        Tab::PhoneNumbers => {
            views::phone_numbers::handle_event(
                &mut app.phone_numbers_view,
                &app.api_client,
                &event,
                &ctx,
            )
            .await
        }
        Tab::Devices => {
            views::devices::handle_event(&mut app.devices_view, &app.api_client, &event, &ctx)
                .await
        }
        Tab::LineBindings => {
            views::line_bindings::handle_event(
                &mut app.line_bindings_view,
                &app.api_client,
                &event,
                &ctx,
            )
            .await
        }
        Tab::Calls => {
            views::calls::handle_event(
                &mut app.calls_view,
                &app.api_client,
                &event,
                &ctx,
                app.events_url.as_deref(),
            )
            .await
        }
        Tab::Registrations => {
            views::registrations::handle_event(
                &mut app.registrations_view,
                &app.api_client,
                &event,
                &ctx,
            )
            .await
        }
        Tab::Debug => {
            views::debug::handle_event(&mut app.debug_view, &app.api_client, &event, &ctx).await
        }
    };

    // Step 2: Handle result
    match result {
        EventResult::Error(msg) => {
            app.set_error(msg);
            return Ok(());
        }
        EventResult::Consumed => {
            return Ok(());
        }
        EventResult::NotConsumed => {
            // Fall through to global event handling
        }
    }

    // Step 3: Handle global events (not consumed by the view)
    match event {
        Event::Key(_) if event.is_quit() => {
            app.quit();
        }
        Event::Key(KeyEvent {
            code: KeyCode::Esc, ..
        }) => {
            if app.error.is_some() {
                app.clear_error();
            }
        }
        Event::Key(KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::NONE,
            ..
        }) if matches!(c, '1'..='9') => {
            if let Some(tab) = Tab::from_index((c as usize) - ('1' as usize)) {
                app.set_tab(tab);
                check_and_load_tab_data(app).await;
            }
        }
        Event::Key(_) if event.is_shift_left() => {
            app.prev_tab();
            check_and_load_tab_data(app).await;
        }
        Event::Key(_) if event.is_shift_right() => {
            app.next_tab();
            check_and_load_tab_data(app).await;
        }
        Event::Key(_) if event.is_left() => {
            app.prev_tab();
            check_and_load_tab_data(app).await;
        }
        Event::Key(_) if event.is_right() => {
            app.next_tab();
            check_and_load_tab_data(app).await;
        }
        Event::Key(_) if event.is_help() => {
            // TODO: Show help overlay
        }
        Event::Tick => {
            // Periodic update - use view-specific refresh intervals
            let time_since = app.time_since_update();
            let interval = match app.active_tab {
                Tab::Dashboard => views::dashboard::refresh_interval(),
                Tab::Calls => views::calls::refresh_interval(),
                Tab::Registrations => views::registrations::refresh_interval(),
                Tab::Debug => views::debug::refresh_interval(),
                Tab::Users => views::users::refresh_interval(),
                Tab::Tenants => views::tenants::refresh_interval(),
                Tab::PhoneNumbers => views::phone_numbers::refresh_interval(),
                Tab::Devices => views::devices::refresh_interval(),
                Tab::LineBindings => views::line_bindings::refresh_interval(),
            };

            if time_since > interval {
                app.mark_update();
                refresh_active_tab(app).await;
            }
        }
        Event::Resize(_, _) => {
            // Terminal was resized, will redraw automatically
        }
        _ => {}
    }

    Ok(())
}

/// Refresh data for the active tab
async fn refresh_active_tab(app: &mut App) {
    match app.active_tab {
        Tab::Dashboard => {}
        Tab::Tenants => {
            views::tenants::fetch_data(&app.api_client, &mut app.tenants_view).await;
        }
        Tab::Users => {
            views::users::fetch_data(&app.api_client, &mut app.users_view).await;
        }
        Tab::Calls => {
            views::calls::fetch_data(&app.api_client, &mut app.calls_view, app.events_url.as_deref()).await;
        }
        Tab::Registrations => {
            views::registrations::fetch_data(&app.api_client, &mut app.registrations_view).await;
        }
        Tab::Debug => {
            views::debug::fetch_data(&app.api_client, &mut app.debug_view).await;
        }
        Tab::PhoneNumbers => {
            views::phone_numbers::fetch_data(&app.api_client, &mut app.phone_numbers_view).await;
        }
        Tab::Devices => {
            views::devices::fetch_data(&app.api_client, &mut app.devices_view).await;
        }
        Tab::LineBindings => {
            if let Err(e) =
                views::line_bindings::fetch_data(&app.api_client, &mut app.line_bindings_view).await
            {
                app.error = Some(format!("Failed to fetch line bindings: {}", e));
            }
        }
    }
}

/// Check if the current tab needs data loading and load it
async fn check_and_load_tab_data(app: &mut App) {
    let needs_load = match app.active_tab {
        Tab::Dashboard => views::dashboard::needs_data(),
        Tab::Tenants => views::tenants::needs_data(&app.tenants_view),
        Tab::Users => views::users::needs_data(&app.users_view),
        Tab::Calls => views::calls::needs_data(&app.calls_view),
        Tab::Registrations => views::registrations::needs_data(&app.registrations_view),
        Tab::Debug => views::debug::needs_data(&app.debug_view),
        Tab::PhoneNumbers => views::phone_numbers::needs_data(&app.phone_numbers_view),
        Tab::Devices => views::devices::needs_data(&app.devices_view),
        Tab::LineBindings => views::line_bindings::needs_data(&app.line_bindings_view),
    };

    if needs_load {
        refresh_active_tab(app).await;
    }
}
