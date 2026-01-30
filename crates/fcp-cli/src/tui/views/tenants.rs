//! Tenants view showing tenant list and details

use crate::api::{ApiClient, CreateTenantRequest, TenantResponse, UpdateTenantRequest};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState},
    Frame,
};
use tui_input::Input;

/// Modal type for tenant operations
#[derive(Debug, Clone, PartialEq)]
pub enum TenantModal {
    None,
    Add,
    Edit,
    Delete,
}

/// Field being edited in add/edit modal
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TenantField {
    Name,
    Description,
    Status,
    ContactEmail,
    ContactPhone,
    MaxUsers,
    MaxPhoneNumbers,
    MaxConcurrentCalls,
    BillingId,
    Metadata,
}

impl TenantField {
    pub fn next(self) -> Self {
        match self {
            TenantField::Name => TenantField::Description,
            TenantField::Description => TenantField::Status,
            TenantField::Status => TenantField::ContactEmail,
            TenantField::ContactEmail => TenantField::ContactPhone,
            TenantField::ContactPhone => TenantField::MaxUsers,
            TenantField::MaxUsers => TenantField::MaxPhoneNumbers,
            TenantField::MaxPhoneNumbers => TenantField::MaxConcurrentCalls,
            TenantField::MaxConcurrentCalls => TenantField::BillingId,
            TenantField::BillingId => TenantField::Metadata,
            TenantField::Metadata => TenantField::Name,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            TenantField::Name => TenantField::Metadata,
            TenantField::Description => TenantField::Name,
            TenantField::Status => TenantField::Description,
            TenantField::ContactEmail => TenantField::Status,
            TenantField::ContactPhone => TenantField::ContactEmail,
            TenantField::MaxUsers => TenantField::ContactPhone,
            TenantField::MaxPhoneNumbers => TenantField::MaxUsers,
            TenantField::MaxConcurrentCalls => TenantField::MaxPhoneNumbers,
            TenantField::BillingId => TenantField::MaxConcurrentCalls,
            TenantField::Metadata => TenantField::BillingId,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TenantField::Name => "Name",
            TenantField::Description => "Description",
            TenantField::Status => "Status",
            TenantField::ContactEmail => "Contact Email",
            TenantField::ContactPhone => "Contact Phone",
            TenantField::MaxUsers => "Max Users",
            TenantField::MaxPhoneNumbers => "Max Phone Numbers",
            TenantField::MaxConcurrentCalls => "Max Concurrent Calls",
            TenantField::BillingId => "Billing ID",
            TenantField::Metadata => "Metadata (JSON)",
        }
    }
}

/// State for the tenants view
pub struct TenantsState {
    pub tenants: Vec<TenantResponse>,
    pub table_state: TableState,
    pub modal: TenantModal,
    pub current_field: TenantField,

    // Form inputs
    pub name_input: Input,
    pub description_input: Input,
    pub status_input: Input,
    pub contact_email_input: Input,
    pub contact_phone_input: Input,
    pub max_users_input: Input,
    pub max_phone_numbers_input: Input,
    pub max_concurrent_calls_input: Input,
    pub billing_id_input: Input,
    pub metadata_input: Input,

    pub message: Option<String>,
    pub error: Option<String>,
}

impl TenantsState {
    pub fn new() -> Self {
        Self {
            tenants: Vec::new(),
            table_state: TableState::default(),
            modal: TenantModal::None,
            current_field: TenantField::Name,
            name_input: Input::default(),
            description_input: Input::default(),
            status_input: Input::default().with_value("active".to_string()),
            contact_email_input: Input::default(),
            contact_phone_input: Input::default(),
            max_users_input: Input::default(),
            max_phone_numbers_input: Input::default(),
            max_concurrent_calls_input: Input::default(),
            billing_id_input: Input::default(),
            metadata_input: Input::default(),
            message: None,
            error: None,
        }
    }

    pub fn selected_tenant(&self) -> Option<&TenantResponse> {
        self.table_state
            .selected()
            .and_then(|i| self.tenants.get(i))
    }

    pub fn next_tenant(&mut self) {
        let i = match self.table_state.selected() {
            Some(i) => {
                if i >= self.tenants.len().saturating_sub(1) {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.table_state.select(Some(i));
    }

    pub fn previous_tenant(&mut self) {
        let i = match self.table_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.tenants.len().saturating_sub(1)
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.table_state.select(Some(i));
    }

    pub fn open_add_modal(&mut self) {
        self.modal = TenantModal::Add;
        self.current_field = TenantField::Name;
        self.clear_form();
    }

    pub fn open_edit_modal(&mut self) {
        if let Some(tenant) = self.selected_tenant().cloned() {
            self.modal = TenantModal::Edit;
            self.current_field = TenantField::Name;
            self.name_input = Input::default().with_value(tenant.name.clone());
            self.description_input =
                Input::default().with_value(tenant.description.clone().unwrap_or_default());
            self.status_input = Input::default().with_value(tenant.status.clone());
            self.contact_email_input =
                Input::default().with_value(tenant.contact_email.clone().unwrap_or_default());
            self.contact_phone_input =
                Input::default().with_value(tenant.contact_phone.clone().unwrap_or_default());
            self.max_users_input = Input::default()
                .with_value(tenant.max_users.map(|v| v.to_string()).unwrap_or_default());
            self.max_phone_numbers_input = Input::default().with_value(
                tenant
                    .max_phone_numbers
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
            );
            self.max_concurrent_calls_input = Input::default().with_value(
                tenant
                    .max_concurrent_calls
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
            );
            self.billing_id_input =
                Input::default().with_value(tenant.billing_id.clone().unwrap_or_default());
            self.metadata_input =
                Input::default().with_value(tenant.metadata.clone().unwrap_or_default());
        }
    }

    pub fn open_delete_modal(&mut self) {
        self.modal = TenantModal::Delete;
    }

    pub fn close_modal(&mut self) {
        self.modal = TenantModal::None;
        self.clear_form();
    }

    fn clear_form(&mut self) {
        self.name_input = Input::default();
        self.description_input = Input::default();
        self.status_input = Input::default().with_value("active".to_string());
        self.contact_email_input = Input::default();
        self.contact_phone_input = Input::default();
        self.max_users_input = Input::default();
        self.max_phone_numbers_input = Input::default();
        self.max_concurrent_calls_input = Input::default();
        self.billing_id_input = Input::default();
        self.metadata_input = Input::default();
    }

    pub fn get_current_input_mut(&mut self) -> &mut Input {
        match self.current_field {
            TenantField::Name => &mut self.name_input,
            TenantField::Description => &mut self.description_input,
            TenantField::Status => &mut self.status_input,
            TenantField::ContactEmail => &mut self.contact_email_input,
            TenantField::ContactPhone => &mut self.contact_phone_input,
            TenantField::MaxUsers => &mut self.max_users_input,
            TenantField::MaxPhoneNumbers => &mut self.max_phone_numbers_input,
            TenantField::MaxConcurrentCalls => &mut self.max_concurrent_calls_input,
            TenantField::BillingId => &mut self.billing_id_input,
            TenantField::Metadata => &mut self.metadata_input,
        }
    }

    pub fn next_field(&mut self) {
        self.current_field = self.current_field.next();
    }

    pub fn previous_field(&mut self) {
        self.current_field = self.current_field.previous();
    }

    pub fn build_create_request(&self) -> CreateTenantRequest {
        let max_users = self.max_users_input.value().parse().ok();
        let max_phone_numbers = self.max_phone_numbers_input.value().parse().ok();
        let max_concurrent_calls = self.max_concurrent_calls_input.value().parse().ok();

        CreateTenantRequest {
            name: self.name_input.value().to_string(),
            description: if self.description_input.value().is_empty() {
                None
            } else {
                Some(self.description_input.value().to_string())
            },
            status: if self.status_input.value().is_empty() {
                None
            } else {
                Some(self.status_input.value().to_string())
            },
            contact_email: if self.contact_email_input.value().is_empty() {
                None
            } else {
                Some(self.contact_email_input.value().to_string())
            },
            contact_phone: if self.contact_phone_input.value().is_empty() {
                None
            } else {
                Some(self.contact_phone_input.value().to_string())
            },
            max_users,
            max_phone_numbers,
            max_concurrent_calls,
            billing_id: if self.billing_id_input.value().is_empty() {
                None
            } else {
                Some(self.billing_id_input.value().to_string())
            },
            trial_expires_at: None,
            metadata: if self.metadata_input.value().is_empty() {
                None
            } else {
                Some(self.metadata_input.value().to_string())
            },
        }
    }

    pub fn build_update_request(&self) -> UpdateTenantRequest {
        let max_users = self.max_users_input.value().parse().ok();
        let max_phone_numbers = self.max_phone_numbers_input.value().parse().ok();
        let max_concurrent_calls = self.max_concurrent_calls_input.value().parse().ok();

        UpdateTenantRequest {
            name: if self.name_input.value().is_empty() {
                None
            } else {
                Some(self.name_input.value().to_string())
            },
            description: if self.description_input.value().is_empty() {
                None
            } else {
                Some(self.description_input.value().to_string())
            },
            status: if self.status_input.value().is_empty() {
                None
            } else {
                Some(self.status_input.value().to_string())
            },
            contact_email: if self.contact_email_input.value().is_empty() {
                None
            } else {
                Some(self.contact_email_input.value().to_string())
            },
            contact_phone: if self.contact_phone_input.value().is_empty() {
                None
            } else {
                Some(self.contact_phone_input.value().to_string())
            },
            max_users,
            max_phone_numbers,
            max_concurrent_calls,
            billing_id: if self.billing_id_input.value().is_empty() {
                None
            } else {
                Some(self.billing_id_input.value().to_string())
            },
            trial_expires_at: None,
            metadata: if self.metadata_input.value().is_empty() {
                None
            } else {
                Some(self.metadata_input.value().to_string())
            },
        }
    }
}

/// Render the tenants view
pub fn render(f: &mut Frame, area: Rect, state: &mut TenantsState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(area);

    // Main content area
    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(chunks[0]);

    // Tenant list table
    render_tenant_list(f, content_chunks[0], state);

    // Details panel
    render_details(f, content_chunks[1], state);

    // Status bar
    render_status_bar(f, chunks[1], state);

    // Modals
    match state.modal {
        TenantModal::Add | TenantModal::Edit => render_form_modal(f, area, state),
        TenantModal::Delete => render_delete_modal(f, area, state),
        TenantModal::None => {}
    }
}

fn render_tenant_list(f: &mut Frame, area: Rect, state: &mut TenantsState) {
    let header_cells = ["Name", "Status", "Max Users", "Max Phone#", "Max Calls"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow)));
    let header = Row::new(header_cells).style(Style::default()).height(1);

    let rows = state.tenants.iter().map(|tenant| {
        let status_color = match tenant.status.as_str() {
            "active" => Color::Green,
            "trial" => Color::Blue,
            "suspended" => Color::Yellow,
            "closed" => Color::Red,
            _ => Color::Gray,
        };

        Row::new(vec![
            Cell::from(tenant.name.clone()),
            Cell::from(tenant.status.clone()).style(Style::default().fg(status_color)),
            Cell::from(
                tenant
                    .max_users
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "∞".to_string()),
            ),
            Cell::from(
                tenant
                    .max_phone_numbers
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "∞".to_string()),
            ),
            Cell::from(
                tenant
                    .max_concurrent_calls
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "∞".to_string()),
            ),
        ])
    });

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(30),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title("Tenants (↑/↓: navigate, a: add, e: edit, d: delete)")
            .borders(Borders::ALL),
    )
    .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    f.render_stateful_widget(table, area, &mut state.table_state);
}

fn render_details(f: &mut Frame, area: Rect, state: &TenantsState) {
    let details = if let Some(tenant) = state.selected_tenant() {
        let created_at = chrono::DateTime::from_timestamp(tenant.created_at as i64, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "Unknown".to_string());

        let updated_at = chrono::DateTime::from_timestamp(tenant.updated_at as i64, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "Unknown".to_string());

        vec![
            Line::from(vec![
                Span::styled("ID: ", Style::default().fg(Color::Cyan)),
                Span::raw(&tenant.id),
            ]),
            Line::from(vec![
                Span::styled("Name: ", Style::default().fg(Color::Cyan)),
                Span::raw(&tenant.name),
            ]),
            Line::from(vec![
                Span::styled("Status: ", Style::default().fg(Color::Cyan)),
                Span::raw(&tenant.status),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Description: ", Style::default().fg(Color::Cyan)),
                Span::raw(tenant.description.as_deref().unwrap_or("None")),
            ]),
            Line::from(vec![
                Span::styled("Contact Email: ", Style::default().fg(Color::Cyan)),
                Span::raw(tenant.contact_email.as_deref().unwrap_or("None")),
            ]),
            Line::from(vec![
                Span::styled("Contact Phone: ", Style::default().fg(Color::Cyan)),
                Span::raw(tenant.contact_phone.as_deref().unwrap_or("None")),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Max Users: ", Style::default().fg(Color::Cyan)),
                Span::raw(
                    tenant
                        .max_users
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "Unlimited".to_string()),
                ),
            ]),
            Line::from(vec![
                Span::styled("Max Phone Numbers: ", Style::default().fg(Color::Cyan)),
                Span::raw(
                    tenant
                        .max_phone_numbers
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "Unlimited".to_string()),
                ),
            ]),
            Line::from(vec![
                Span::styled("Max Concurrent Calls: ", Style::default().fg(Color::Cyan)),
                Span::raw(
                    tenant
                        .max_concurrent_calls
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "Unlimited".to_string()),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Billing ID: ", Style::default().fg(Color::Cyan)),
                Span::raw(tenant.billing_id.as_deref().unwrap_or("None")),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Created: ", Style::default().fg(Color::Cyan)),
                Span::raw(created_at),
            ]),
            Line::from(vec![
                Span::styled("Updated: ", Style::default().fg(Color::Cyan)),
                Span::raw(updated_at),
            ]),
        ]
    } else {
        vec![Line::from("No tenant selected")]
    };

    let paragraph =
        Paragraph::new(details).block(Block::default().title("Details").borders(Borders::ALL));

    f.render_widget(paragraph, area);
}

fn render_status_bar(f: &mut Frame, area: Rect, state: &TenantsState) {
    let text = if let Some(ref msg) = state.message {
        Line::from(vec![Span::styled(msg, Style::default().fg(Color::Green))])
    } else if let Some(ref err) = state.error {
        Line::from(vec![Span::styled(err, Style::default().fg(Color::Red))])
    } else {
        Line::from(vec![Span::raw(format!(
            "Total tenants: {}",
            state.tenants.len()
        ))])
    };

    let paragraph = Paragraph::new(text).block(Block::default().borders(Borders::ALL));

    f.render_widget(paragraph, area);
}

fn render_form_modal(f: &mut Frame, area: Rect, state: &TenantsState) {
    let modal_title = match state.modal {
        TenantModal::Add => "Add Tenant",
        TenantModal::Edit => "Edit Tenant",
        _ => "",
    };

    // Create a centered modal area
    let modal_area = centered_rect(80, 80, area);

    // Clear the background
    f.render_widget(Clear, modal_area);

    // Modal block
    let block = Block::default()
        .title(format!(
            "{} (Tab/Shift+Tab: next/prev field, Enter: submit, Esc: cancel)",
            modal_title
        ))
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    // Split the inner area into form fields
    let field_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Name
            Constraint::Length(3), // Description
            Constraint::Length(3), // Status
            Constraint::Length(3), // Contact Email
            Constraint::Length(3), // Contact Phone
            Constraint::Length(3), // Max Users
            Constraint::Length(3), // Max Phone Numbers
            Constraint::Length(3), // Max Concurrent Calls
            Constraint::Length(3), // Billing ID
            Constraint::Length(3), // Metadata
        ])
        .split(inner);

    render_field(
        f,
        field_chunks[0],
        "Name",
        &state.name_input,
        state.current_field == TenantField::Name,
    );
    render_field(
        f,
        field_chunks[1],
        "Description",
        &state.description_input,
        state.current_field == TenantField::Description,
    );
    render_field(
        f,
        field_chunks[2],
        "Status (active/trial/suspended/closed)",
        &state.status_input,
        state.current_field == TenantField::Status,
    );
    render_field(
        f,
        field_chunks[3],
        "Contact Email",
        &state.contact_email_input,
        state.current_field == TenantField::ContactEmail,
    );
    render_field(
        f,
        field_chunks[4],
        "Contact Phone",
        &state.contact_phone_input,
        state.current_field == TenantField::ContactPhone,
    );
    render_field(
        f,
        field_chunks[5],
        "Max Users (empty = unlimited)",
        &state.max_users_input,
        state.current_field == TenantField::MaxUsers,
    );
    render_field(
        f,
        field_chunks[6],
        "Max Phone Numbers (empty = unlimited)",
        &state.max_phone_numbers_input,
        state.current_field == TenantField::MaxPhoneNumbers,
    );
    render_field(
        f,
        field_chunks[7],
        "Max Concurrent Calls (empty = unlimited)",
        &state.max_concurrent_calls_input,
        state.current_field == TenantField::MaxConcurrentCalls,
    );
    render_field(
        f,
        field_chunks[8],
        "Billing ID",
        &state.billing_id_input,
        state.current_field == TenantField::BillingId,
    );
    render_field(
        f,
        field_chunks[9],
        "Metadata (JSON)",
        &state.metadata_input,
        state.current_field == TenantField::Metadata,
    );
}

fn render_field(f: &mut Frame, area: Rect, label: &str, input: &Input, is_focused: bool) {
    let style = if is_focused {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    let block = Block::default()
        .title(label)
        .borders(Borders::ALL)
        .style(style);

    let width = block.inner(area).width.max(1) as usize;
    let scroll = input.visual_scroll(width);

    let paragraph = Paragraph::new(input.value())
        .style(Style::default())
        .block(block)
        .scroll((0, scroll as u16));

    f.render_widget(paragraph, area);

    if is_focused {
        let cursor_x = area.x + (input.visual_cursor().max(scroll) - scroll) as u16 + 1;
        let cursor_y = area.y + 1;
        f.set_cursor_position((cursor_x, cursor_y));
    }
}

fn render_delete_modal(f: &mut Frame, area: Rect, state: &TenantsState) {
    let modal_area = centered_rect(50, 20, area);

    f.render_widget(Clear, modal_area);

    let tenant_name = state
        .selected_tenant()
        .map(|t| t.name.as_str())
        .unwrap_or("unknown");

    let text = vec![
        Line::from(""),
        Line::from(vec![Span::raw("Are you sure you want to delete tenant")]),
        Line::from(vec![Span::styled(
            tenant_name,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![Span::raw("Press Enter to confirm, Esc to cancel")]),
    ];

    let paragraph = Paragraph::new(text)
        .block(
            Block::default()
                .title("Confirm Delete")
                .borders(Borders::ALL)
                .style(Style::default().bg(Color::Black).fg(Color::Red)),
        )
        .style(Style::default());

    f.render_widget(paragraph, modal_area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Fetch tenants from the API
pub async fn fetch_tenants(client: &ApiClient, state: &mut TenantsState) {
    state.error = None;
    state.message = None;

    match client.list_tenants().await {
        Ok(tenants) => {
            state.tenants = tenants;
            // Select first tenant if none selected and list is not empty
            if state.table_state.selected().is_none() && !state.tenants.is_empty() {
                state.table_state.select(Some(0));
            }
        }
        Err(e) => {
            state.error = Some(format!("Failed to fetch tenants: {}", e));
        }
    }
}

/// Handle tenant-specific actions (keyboard events in the tenants view)
pub async fn handle_tenant_action(
    state: &mut TenantsState,
    client: &ApiClient,
    event: &crate::tui::event::Event,
) -> bool {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    match state.modal {
        TenantModal::Add | TenantModal::Edit => {
            // Handle form modal events
            match event {
                crate::tui::event::Event::Key(KeyEvent {
                    code: KeyCode::Esc, ..
                }) => {
                    state.close_modal();
                    true
                }
                crate::tui::event::Event::Key(KeyEvent {
                    code: KeyCode::Tab,
                    modifiers: KeyModifiers::NONE,
                    ..
                }) => {
                    state.next_field();
                    true
                }
                crate::tui::event::Event::Key(KeyEvent {
                    code: KeyCode::BackTab,
                    ..
                }) => {
                    state.previous_field();
                    true
                }
                crate::tui::event::Event::Key(KeyEvent {
                    code: KeyCode::Enter,
                    ..
                }) => {
                    // Submit the form
                    if state.modal == TenantModal::Add {
                        let req = state.build_create_request();
                        match client.create_tenant(&req).await {
                            Ok(tenant) => {
                                state.message =
                                    Some(format!("Tenant '{}' created successfully", tenant.name));
                                state.close_modal();
                                fetch_tenants(client, state).await;
                            }
                            Err(e) => {
                                state.error = Some(format!("Failed to create tenant: {}", e));
                            }
                        }
                    } else if state.modal == TenantModal::Edit {
                        if let Some(tenant) = state.selected_tenant() {
                            let tenant_id = tenant.id.clone();
                            let req = state.build_update_request();
                            match client.update_tenant(&tenant_id, &req).await {
                                Ok(tenant) => {
                                    state.message = Some(format!(
                                        "Tenant '{}' updated successfully",
                                        tenant.name
                                    ));
                                    state.close_modal();
                                    fetch_tenants(client, state).await;
                                }
                                Err(e) => {
                                    state.error = Some(format!("Failed to update tenant: {}", e));
                                }
                            }
                        }
                    }
                    true
                }
                crate::tui::event::Event::Key(KeyEvent {
                    code: KeyCode::Char(c),
                    modifiers,
                    ..
                }) if *modifiers == KeyModifiers::NONE || *modifiers == KeyModifiers::SHIFT => {
                    // Type character into current field (accept both lowercase and uppercase)
                    let input = state.get_current_input_mut();
                    let mut value = input.value().to_string();
                    value.push(*c);
                    *input = Input::default().with_value(value);
                    true
                }
                crate::tui::event::Event::Key(KeyEvent {
                    code: KeyCode::Backspace,
                    ..
                }) => {
                    let input = state.get_current_input_mut();
                    let mut value = input.value().to_string();
                    value.pop();
                    *input = Input::default().with_value(value);
                    true
                }
                crate::tui::event::Event::Key(KeyEvent {
                    code: KeyCode::Delete,
                    ..
                }) => {
                    // Delete doesn't make sense without cursor position, so ignore
                    true
                }
                crate::tui::event::Event::Key(KeyEvent {
                    code: KeyCode::Left,
                    ..
                }) => {
                    // Cursor movement doesn't make sense without tracking cursor, so ignore
                    true
                }
                crate::tui::event::Event::Key(KeyEvent {
                    code: KeyCode::Right,
                    ..
                }) => {
                    // Cursor movement doesn't make sense without tracking cursor, so ignore
                    true
                }
                crate::tui::event::Event::Key(KeyEvent {
                    code: KeyCode::Home,
                    ..
                }) => {
                    // Cursor movement doesn't make sense without tracking cursor, so ignore
                    true
                }
                crate::tui::event::Event::Key(KeyEvent {
                    code: KeyCode::End, ..
                }) => {
                    // Cursor movement doesn't make sense without tracking cursor, so ignore
                    true
                }
                _ => false,
            }
        }
        TenantModal::Delete => {
            // Handle delete confirmation modal
            match event {
                crate::tui::event::Event::Key(KeyEvent {
                    code: KeyCode::Esc, ..
                }) => {
                    state.close_modal();
                    true
                }
                crate::tui::event::Event::Key(KeyEvent {
                    code: KeyCode::Enter,
                    ..
                }) => {
                    // Confirm deletion
                    if let Some(tenant) = state.selected_tenant() {
                        let tenant_id = tenant.id.clone();
                        let tenant_name = tenant.name.clone();
                        match client.delete_tenant(&tenant_id).await {
                            Ok(_) => {
                                state.message =
                                    Some(format!("Tenant '{}' deleted successfully", tenant_name));
                                state.close_modal();
                                fetch_tenants(client, state).await;
                            }
                            Err(e) => {
                                state.error = Some(format!("Failed to delete tenant: {}", e));
                            }
                        }
                    }
                    true
                }
                _ => false,
            }
        }
        TenantModal::None => {
            // Handle action keys when no modal is open
            // Note: Up/Down navigation is handled in the main event loop (mod.rs)
            match event {
                crate::tui::event::Event::Key(KeyEvent {
                    code: KeyCode::Char('a'),
                    ..
                }) => {
                    state.open_add_modal();
                    true
                }
                crate::tui::event::Event::Key(KeyEvent {
                    code: KeyCode::Char('e'),
                    ..
                }) => {
                    if state.selected_tenant().is_some() {
                        state.open_edit_modal();
                    }
                    true
                }
                crate::tui::event::Event::Key(KeyEvent {
                    code: KeyCode::Char('d'),
                    ..
                }) => {
                    if state.selected_tenant().is_some() {
                        state.open_delete_modal();
                    }
                    true
                }
                crate::tui::event::Event::Key(KeyEvent {
                    code: KeyCode::Char('r'),
                    ..
                }) => {
                    // Refresh the list
                    fetch_tenants(client, state).await;
                    true
                }
                _ => false,
            }
        }
    }
}
