//! Users view showing user list and details

use crate::api::{CreateUserRequest, UpdateUserRequest, UserResponse};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState,
    },
    Frame,
};
use tui_input::Input;

/// Common language codes (BCP 47 format)
const LANGUAGES: &[(&str, &str)] = &[
    ("en-US", "English (United States)"),
    ("en-GB", "English (United Kingdom)"),
    ("en-CA", "English (Canada)"),
    ("en-AU", "English (Australia)"),
    ("es-ES", "Spanish (Spain)"),
    ("es-MX", "Spanish (Mexico)"),
    ("fr-FR", "French (France)"),
    ("fr-CA", "French (Canada)"),
    ("de-DE", "German (Germany)"),
    ("it-IT", "Italian (Italy)"),
    ("pt-BR", "Portuguese (Brazil)"),
    ("pt-PT", "Portuguese (Portugal)"),
    ("nl-NL", "Dutch (Netherlands)"),
    ("pl-PL", "Polish (Poland)"),
    ("ru-RU", "Russian (Russia)"),
    ("ja-JP", "Japanese (Japan)"),
    ("ko-KR", "Korean (Korea)"),
    ("zh-CN", "Chinese (Simplified)"),
    ("zh-TW", "Chinese (Traditional)"),
    ("ar-SA", "Arabic (Saudi Arabia)"),
    ("hi-IN", "Hindi (India)"),
    ("sv-SE", "Swedish (Sweden)"),
    ("da-DK", "Danish (Denmark)"),
    ("no-NO", "Norwegian (Norway)"),
    ("fi-FI", "Finnish (Finland)"),
];

/// Common IANA timezone identifiers
const TIMEZONES: &[&str] = &[
    "UTC",
    "America/New_York",
    "America/Chicago",
    "America/Denver",
    "America/Los_Angeles",
    "America/Phoenix",
    "America/Anchorage",
    "America/Honolulu",
    "America/Toronto",
    "America/Vancouver",
    "America/Mexico_City",
    "America/Sao_Paulo",
    "America/Buenos_Aires",
    "Europe/London",
    "Europe/Paris",
    "Europe/Berlin",
    "Europe/Rome",
    "Europe/Madrid",
    "Europe/Amsterdam",
    "Europe/Brussels",
    "Europe/Vienna",
    "Europe/Stockholm",
    "Europe/Oslo",
    "Europe/Copenhagen",
    "Europe/Helsinki",
    "Europe/Warsaw",
    "Europe/Prague",
    "Europe/Athens",
    "Europe/Istanbul",
    "Europe/Moscow",
    "Asia/Dubai",
    "Asia/Kolkata",
    "Asia/Bangkok",
    "Asia/Singapore",
    "Asia/Hong_Kong",
    "Asia/Shanghai",
    "Asia/Tokyo",
    "Asia/Seoul",
    "Australia/Sydney",
    "Australia/Melbourne",
    "Australia/Brisbane",
    "Australia/Perth",
    "Pacific/Auckland",
    "Pacific/Fiji",
];

/// Modal type for user operations
#[derive(Debug, Clone, PartialEq)]
pub enum UserModal {
    None,
    Add,
    Edit,
    Delete,
    SetDigestPassword,
    RemoveDigestPassword,
}

/// State for the users view
pub struct UsersViewState {
    /// List of users
    pub users: Vec<UserResponse>,
    /// Table state for selection
    pub table_state: TableState,
    /// Last error message
    pub error: Option<String>,
    /// Loading state
    pub loading: bool,
    /// Active modal
    pub modal: UserModal,
    /// Input field: username
    pub input_username: Input,
    /// Input field: display name
    pub input_display_name: Input,
    /// Input field: password
    pub input_password: Input,
    /// Whether user is enabled (for add/edit)
    pub input_enabled: bool,

    // ===== Enterprise input fields =====
    /// Input field: email
    pub input_email: Input,
    /// Input field: phone number
    pub input_phone_number: Input,
    /// Input field: mobile number
    pub input_mobile_number: Input,
    /// Input field: PIN (voicemail)
    pub input_pin: Input,
    /// Input field: extension
    pub input_extension: Input,
    /// Input field: department
    pub input_department: Input,
    /// Input field: job title
    pub input_job_title: Input,
    /// Input field: location
    pub input_location: Input,
    /// Input field: language
    pub input_language: Input,
    /// Input field: timezone
    pub input_timezone: Input,
    /// Input field: account status
    pub input_account_status: Input,
    /// Input field: emergency contact name
    pub input_emergency_contact_name: Input,
    /// Input field: emergency contact phone
    pub input_emergency_contact_phone: Input,

    /// Input field: digest password (for Cisco-style separate SIP credentials)
    pub input_digest_password: Input,

    /// Which input field is focused (0-15)
    pub focused_field: usize,
    /// Total number of input fields
    pub total_fields: usize,
    /// Scroll offset for form modal (which field is at the top)
    pub scroll_offset: usize,

    /// Selection mode: true when user is selecting from language/timezone dropdown
    pub in_selection_mode: bool,
    /// Language selection state
    pub language_list_state: ListState,
    /// Timezone selection state
    pub timezone_list_state: ListState,
}

impl Default for UsersViewState {
    fn default() -> Self {
        let mut table_state = TableState::default();
        table_state.select(Some(0));

        Self {
            users: Vec::new(),
            table_state,
            error: None,
            loading: false,
            modal: UserModal::None,
            input_username: Input::default(),
            input_display_name: Input::default(),
            input_password: Input::default(),
            input_enabled: true,
            // Enterprise fields
            input_email: Input::default(),
            input_phone_number: Input::default(),
            input_mobile_number: Input::default(),
            input_pin: Input::default(),
            input_extension: Input::default(),
            input_department: Input::default(),
            input_job_title: Input::default(),
            input_location: Input::default(),
            input_language: Input::new("en-US".to_string()),
            input_timezone: Input::new("UTC".to_string()),
            input_account_status: Input::new("active".to_string()),
            input_emergency_contact_name: Input::default(),
            input_emergency_contact_phone: Input::default(),
            input_digest_password: Input::default(),
            focused_field: 0,
            total_fields: 16, // 0-15: username, display_name, password, email, phone, mobile, pin, ext, dept, title, loc, lang, tz, status, emerg_name, emerg_phone
            scroll_offset: 0,
            in_selection_mode: false,
            language_list_state: ListState::default(),
            timezone_list_state: ListState::default(),
        }
    }
}

impl UsersViewState {
    /// Get currently selected user
    pub fn selected_user(&self) -> Option<&UserResponse> {
        self.table_state.selected().and_then(|i| self.users.get(i))
    }

    /// Move selection up
    pub fn select_previous(&mut self) {
        if self.users.is_empty() {
            return;
        }

        let selected = self.table_state.selected().unwrap_or(0);
        if selected > 0 {
            self.table_state.select(Some(selected - 1));
        }
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        if self.users.is_empty() {
            return;
        }

        let selected = self.table_state.selected().unwrap_or(0);
        if selected < self.users.len() - 1 {
            self.table_state.select(Some(selected + 1));
        }
    }

    /// Update users list
    pub fn set_users(&mut self, users: Vec<UserResponse>) {
        self.users = users;
        // Reset selection to first item if we had no selection or it's out of bounds
        if self.table_state.selected().is_none()
            || self.table_state.selected().unwrap() >= self.users.len()
        {
            self.table_state
                .select(if self.users.is_empty() { None } else { Some(0) });
        }
    }

    /// Open add user modal
    pub fn open_add_modal(&mut self) {
        self.modal = UserModal::Add;
        self.clear_inputs();
        self.focused_field = 0;
        self.scroll_offset = 0;
        self.error = None;
    }

    /// Open edit user modal
    pub fn open_edit_modal(&mut self) {
        // Clone user data to avoid borrow checker issues
        if let Some(user) = self.selected_user().cloned() {
            self.modal = UserModal::Edit;
            self.input_username = Input::new(user.username);
            self.input_display_name = Input::new(user.display_name.unwrap_or_default());
            self.input_password = Input::default(); // Don't show existing password
            self.input_enabled = user.enabled;

            // Enterprise fields
            self.input_email = Input::new(user.email.unwrap_or_default());
            self.input_phone_number = Input::new(user.phone_number.unwrap_or_default());
            self.input_mobile_number = Input::new(user.mobile_number.unwrap_or_default());
            self.input_pin = Input::default(); // Don't show existing PIN
            self.input_extension = Input::new(user.extension.unwrap_or_default());
            self.input_department = Input::new(user.department.unwrap_or_default());
            self.input_job_title = Input::new(user.job_title.unwrap_or_default());
            self.input_location = Input::new(user.location.unwrap_or_default());
            self.input_language = Input::new(user.language);
            self.input_timezone = Input::new(user.timezone);
            self.input_account_status = Input::new(user.account_status);
            self.input_emergency_contact_name =
                Input::new(user.emergency_contact_name.unwrap_or_default());
            self.input_emergency_contact_phone =
                Input::new(user.emergency_contact_phone.unwrap_or_default());

            self.focused_field = 1; // Focus on display name for edit
            self.scroll_offset = 0;
            self.in_selection_mode = false;
            self.error = None;
        }
    }

    /// Open delete user modal
    pub fn open_delete_modal(&mut self) {
        if self.selected_user().is_some() {
            self.modal = UserModal::Delete;
        }
    }

    /// Open set digest password modal
    pub fn open_set_digest_password_modal(&mut self) {
        if self.selected_user().is_some() {
            self.modal = UserModal::SetDigestPassword;
            self.input_digest_password = Input::default();
            self.error = None;
        }
    }

    /// Open remove digest password confirmation modal
    pub fn open_remove_digest_password_modal(&mut self) {
        if self.selected_user().is_some() {
            self.modal = UserModal::RemoveDigestPassword;
            self.error = None;
        }
    }

    /// Close modal
    pub fn close_modal(&mut self) {
        self.modal = UserModal::None;
        self.clear_inputs();
        self.error = None;
    }

    /// Clear input fields
    fn clear_inputs(&mut self) {
        self.input_username = Input::default();
        self.input_display_name = Input::default();
        self.input_password = Input::default();
        self.input_enabled = true;
        // Enterprise fields
        self.input_email = Input::default();
        self.input_phone_number = Input::default();
        self.input_mobile_number = Input::default();
        self.input_pin = Input::default();
        self.input_extension = Input::default();
        self.input_department = Input::default();
        self.input_job_title = Input::default();
        self.input_location = Input::default();
        self.input_language = Input::new("en-US".to_string());
        self.input_timezone = Input::new("UTC".to_string());
        self.input_account_status = Input::new("active".to_string());
        self.input_emergency_contact_name = Input::default();
        self.input_emergency_contact_phone = Input::default();
        self.scroll_offset = 0;
    }

    /// Move focus to next field
    pub fn next_field(&mut self) {
        self.focused_field = (self.focused_field + 1) % self.total_fields;
    }

    /// Move focus to previous field
    pub fn prev_field(&mut self) {
        self.focused_field = if self.focused_field == 0 {
            self.total_fields - 1
        } else {
            self.focused_field - 1
        };
    }

    /// Ensure the focused field is visible by adjusting scroll offset
    /// visible_fields is the number of fields that can fit in the visible area
    pub fn ensure_focused_visible(&mut self, visible_fields: usize) {
        // If focused field is above the visible area, scroll up
        if self.focused_field < self.scroll_offset {
            self.scroll_offset = self.focused_field;
        }
        // If focused field is below the visible area, scroll down
        else if self.focused_field >= self.scroll_offset + visible_fields {
            self.scroll_offset = self.focused_field.saturating_sub(visible_fields - 1);
        }
    }

    /// Get currently focused input
    pub fn focused_input(&mut self) -> &mut Input {
        match self.focused_field {
            0 => &mut self.input_username,
            1 => &mut self.input_display_name,
            2 => &mut self.input_password,
            3 => &mut self.input_email,
            4 => &mut self.input_phone_number,
            5 => &mut self.input_mobile_number,
            6 => &mut self.input_pin,
            7 => &mut self.input_extension,
            8 => &mut self.input_department,
            9 => &mut self.input_job_title,
            10 => &mut self.input_location,
            11 => &mut self.input_language,
            12 => &mut self.input_timezone,
            13 => &mut self.input_account_status,
            14 => &mut self.input_emergency_contact_name,
            15 => &mut self.input_emergency_contact_phone,
            _ => &mut self.input_username,
        }
    }

    /// Check if the currently focused field is a selection field (language or timezone)
    pub fn is_selection_field(&self) -> bool {
        self.focused_field == 11 || self.focused_field == 12
    }

    /// Open selection mode for the currently focused field
    pub fn open_selection_mode(&mut self) {
        if !self.is_selection_field() {
            return;
        }

        self.in_selection_mode = true;

        // Initialize selection state based on current value
        if self.focused_field == 11 {
            // Language field
            let current_lang = self.input_language.value();
            let index = LANGUAGES
                .iter()
                .position(|(code, _)| *code == current_lang)
                .unwrap_or(0);
            self.language_list_state.select(Some(index));
        } else if self.focused_field == 12 {
            // Timezone field
            let current_tz = self.input_timezone.value();
            let index = TIMEZONES
                .iter()
                .position(|tz| *tz == current_tz)
                .unwrap_or(0);
            self.timezone_list_state.select(Some(index));
        }
    }

    /// Close selection mode without changing the value
    pub fn close_selection_mode(&mut self) {
        self.in_selection_mode = false;
    }

    /// Confirm selection and update the input value
    pub fn confirm_selection(&mut self) {
        if !self.in_selection_mode {
            return;
        }

        if self.focused_field == 11 {
            // Language field
            if let Some(selected) = self.language_list_state.selected() {
                if let Some((code, _)) = LANGUAGES.get(selected) {
                    self.input_language = Input::new(code.to_string());
                }
            }
        } else if self.focused_field == 12 {
            // Timezone field
            if let Some(selected) = self.timezone_list_state.selected() {
                if let Some(tz) = TIMEZONES.get(selected) {
                    self.input_timezone = Input::new(tz.to_string());
                }
            }
        }

        self.in_selection_mode = false;
    }

    /// Navigate selection list up
    pub fn selection_previous(&mut self) {
        if !self.in_selection_mode {
            return;
        }

        if self.focused_field == 11 {
            // Language field
            let selected = self.language_list_state.selected().unwrap_or(0);
            if selected > 0 {
                self.language_list_state.select(Some(selected - 1));
            }
        } else if self.focused_field == 12 {
            // Timezone field
            let selected = self.timezone_list_state.selected().unwrap_or(0);
            if selected > 0 {
                self.timezone_list_state.select(Some(selected - 1));
            }
        }
    }

    /// Navigate selection list down
    pub fn selection_next(&mut self) {
        if !self.in_selection_mode {
            return;
        }

        if self.focused_field == 11 {
            // Language field
            let selected = self.language_list_state.selected().unwrap_or(0);
            if selected < LANGUAGES.len() - 1 {
                self.language_list_state.select(Some(selected + 1));
            }
        } else if self.focused_field == 12 {
            // Timezone field
            let selected = self.timezone_list_state.selected().unwrap_or(0);
            if selected < TIMEZONES.len() - 1 {
                self.timezone_list_state.select(Some(selected + 1));
            }
        }
    }
}

/// Render the users view
pub fn render(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &mut UsersViewState,
    area: Rect,
) {
    // Split into table and details panel
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(60), // Users table
            Constraint::Percentage(40), // Details panel
        ])
        .split(area);

    render_users_table(f, theme, state, chunks[0]);
    render_details_panel(f, theme, state, chunks[1]);

    // Render modal on top if active
    match state.modal {
        UserModal::Add => render_add_modal(f, theme, state, area),
        UserModal::Edit => render_edit_modal(f, theme, state, area),
        UserModal::Delete => render_delete_modal(f, theme, state, area),
        UserModal::SetDigestPassword => render_set_digest_password_modal(f, theme, state, area),
        UserModal::RemoveDigestPassword => {
            render_remove_digest_password_modal(f, theme, state, area)
        }
        UserModal::None => {}
    }

    // Render selection modal on top if in selection mode
    if state.in_selection_mode {
        render_selection_modal(f, theme, state, area);
    }
}

/// Render the users table
fn render_users_table(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &mut UsersViewState,
    area: Rect,
) {
    let header = Row::new(vec!["Username", "Display Name", "Tenant", "Enabled"])
        .style(theme.title())
        .bottom_margin(1);

    let rows: Vec<Row> = state
        .users
        .iter()
        .map(|user| {
            Row::new(vec![
                Cell::from(user.username.clone()),
                Cell::from(user.display_name.clone().unwrap_or_else(|| "-".to_string())),
                Cell::from(user.tenant_id.clone()),
                Cell::from(if user.enabled { "✓" } else { "✗" }),
            ])
        })
        .collect();

    let widths = [
        Constraint::Percentage(30),
        Constraint::Percentage(30),
        Constraint::Percentage(30),
        Constraint::Percentage(10),
    ];

    let title = if state.loading {
        " Users (Loading...) "
    } else {
        format!(" Users ({}) ", state.users.len()).leak() as &str
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

        let error_para = Paragraph::new(error_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme.error()),
            )
            .wrap(ratatui::widgets::Wrap { trim: true });

        // Show error in a larger area at the bottom with word wrapping
        let error_height = 6u16; // Allow more space for wrapped error messages
        let error_area = Rect {
            x: area.x,
            y: area.y + area.height.saturating_sub(error_height + 1),
            width: area.width,
            height: error_height,
        };

        f.render_widget(error_para, error_area);
    }
}

/// Render the details panel for selected user
fn render_details_panel(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &UsersViewState,
    area: Rect,
) {
    let details_text = if let Some(user) = state.selected_user() {
        let mut lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("Username: ", theme.muted()),
                Span::styled(&user.username, theme.normal()),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("User ID: ", theme.muted()),
                Span::styled(&user.id, theme.info()),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Display Name: ", theme.muted()),
                Span::styled(user.display_name.as_deref().unwrap_or("-"), theme.normal()),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Enabled: ", theme.muted()),
                Span::styled(
                    if user.enabled { "Yes" } else { "No" },
                    if user.enabled {
                        theme.success()
                    } else {
                        theme.error()
                    },
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Status: ", theme.muted()),
                Span::styled(&user.account_status, theme.info()),
            ]),
        ];

        // Enterprise fields section
        if user.email.is_some() || user.extension.is_some() || user.department.is_some() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                "── Contact Info ──",
                theme.title(),
            )]));

            if let Some(email) = &user.email {
                lines.push(Line::from(vec![
                    Span::styled("Email: ", theme.muted()),
                    Span::styled(email, theme.normal()),
                ]));
            }
            if let Some(phone) = &user.phone_number {
                lines.push(Line::from(vec![
                    Span::styled("Phone: ", theme.muted()),
                    Span::styled(phone, theme.normal()),
                ]));
            }
            if let Some(mobile) = &user.mobile_number {
                lines.push(Line::from(vec![
                    Span::styled("Mobile: ", theme.muted()),
                    Span::styled(mobile, theme.normal()),
                ]));
            }
            if let Some(ext) = &user.extension {
                lines.push(Line::from(vec![
                    Span::styled("Extension: ", theme.muted()),
                    Span::styled(ext, theme.normal()),
                ]));
            }
        }

        // Job info section
        if user.department.is_some() || user.job_title.is_some() || user.location.is_some() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                "── Job Info ──",
                theme.title(),
            )]));

            if let Some(dept) = &user.department {
                lines.push(Line::from(vec![
                    Span::styled("Department: ", theme.muted()),
                    Span::styled(dept, theme.normal()),
                ]));
            }
            if let Some(title) = &user.job_title {
                lines.push(Line::from(vec![
                    Span::styled("Title: ", theme.muted()),
                    Span::styled(title, theme.normal()),
                ]));
            }
            if let Some(loc) = &user.location {
                lines.push(Line::from(vec![
                    Span::styled("Location: ", theme.muted()),
                    Span::styled(loc, theme.normal()),
                ]));
            }
        }

        // Preferences section
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "── Preferences ──",
            theme.title(),
        )]));
        lines.push(Line::from(vec![
            Span::styled("Language: ", theme.muted()),
            Span::styled(&user.language, theme.normal()),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Timezone: ", theme.muted()),
            Span::styled(&user.timezone, theme.normal()),
        ]));

        // Emergency contact section
        if user.emergency_contact_name.is_some() || user.emergency_contact_phone.is_some() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                "── Emergency Contact ──",
                theme.title(),
            )]));

            if let Some(name) = &user.emergency_contact_name {
                lines.push(Line::from(vec![
                    Span::styled("Name: ", theme.muted()),
                    Span::styled(name, theme.normal()),
                ]));
            }
            if let Some(phone) = &user.emergency_contact_phone {
                lines.push(Line::from(vec![
                    Span::styled("Phone: ", theme.muted()),
                    Span::styled(phone, theme.normal()),
                ]));
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled("Actions:", theme.title())]));
        lines.push(Line::from(vec![
            Span::styled("  ", theme.muted()),
            Span::styled("a", theme.primary()),
            Span::styled(" - Add  ", theme.normal()),
            Span::styled("e", theme.primary()),
            Span::styled(" - Edit  ", theme.normal()),
            Span::styled("d", theme.primary()),
            Span::styled(" - Delete", theme.normal()),
        ]));

        lines
    } else if state.users.is_empty() {
        vec![
            Line::from(""),
            Line::from(vec![Span::styled("  No users to display", theme.muted())]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "  Press 'a' to add a user",
                theme.info(),
            )]),
            Line::from(vec![Span::styled("  Press 'r' to refresh", theme.info())]),
        ]
    } else {
        vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "  Select a user to view details",
                theme.muted(),
            )]),
            Line::from(""),
            Line::from(""),
            Line::from(vec![Span::styled("  Actions:", theme.title())]),
            Line::from(vec![
                Span::styled("    ", theme.muted()),
                Span::styled("a", theme.primary()),
                Span::styled(" - Add new user", theme.normal()),
            ]),
            Line::from(vec![
                Span::styled("    ", theme.muted()),
                Span::styled("e", theme.primary()),
                Span::styled(" - Edit selected user", theme.normal()),
            ]),
            Line::from(vec![
                Span::styled("    ", theme.muted()),
                Span::styled("d", theme.primary()),
                Span::styled(" - Delete selected user", theme.normal()),
            ]),
        ]
    };

    let details = Paragraph::new(details_text).block(
        Block::default()
            .title(" Details ")
            .borders(Borders::ALL)
            .border_style(theme.border())
            .title_style(theme.title()),
    );

    f.render_widget(details, area);
}

/// Handle navigation events for users view
pub fn handle_navigation(state: &mut UsersViewState, event: &crate::tui::event::Event) {
    if event.is_up() {
        state.select_previous();
    } else if event.is_down() {
        state.select_next();
    }
}

/// Fetch users data
pub async fn fetch_users(api_client: &crate::api::ApiClient, state: &mut UsersViewState) {
    state.loading = true;
    state.error = None;

    match api_client.list_users().await {
        Ok(users) => {
            state.set_users(users);
        }
        Err(e) => {
            state.error = Some(format!("Failed to fetch users: {}", e));
        }
    }

    state.loading = false;
}

/// Render add user modal
fn render_add_modal(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &UsersViewState,
    area: Rect,
) {
    let popup_area = centered_rect(70, 80, area);

    let title_text = " Add New User (Tab/Shift+Tab: navigate, Enter: save, Esc: cancel) ";
    let password_masked = "*".repeat(state.input_password.value().len());
    let pin_masked = "*".repeat(state.input_pin.value().len());

    let fields = vec![
        // Core fields
        (
            "Username:",
            state.input_username.value(),
            state.focused_field == 0,
        ),
        (
            "Display Name:",
            state.input_display_name.value(),
            state.focused_field == 1,
        ),
        (
            "Password:",
            password_masked.as_str(),
            state.focused_field == 2,
        ),
        // Enterprise fields
        (
            "Email:",
            state.input_email.value(),
            state.focused_field == 3,
        ),
        (
            "Phone Number:",
            state.input_phone_number.value(),
            state.focused_field == 4,
        ),
        (
            "Mobile Number:",
            state.input_mobile_number.value(),
            state.focused_field == 5,
        ),
        (
            "PIN (Voicemail):",
            pin_masked.as_str(),
            state.focused_field == 6,
        ),
        (
            "Extension:",
            state.input_extension.value(),
            state.focused_field == 7,
        ),
        (
            "Department:",
            state.input_department.value(),
            state.focused_field == 8,
        ),
        (
            "Job Title:",
            state.input_job_title.value(),
            state.focused_field == 9,
        ),
        (
            "Location:",
            state.input_location.value(),
            state.focused_field == 10,
        ),
        (
            "Language (Space to select):",
            state.input_language.value(),
            state.focused_field == 11,
        ),
        (
            "Timezone (Space to select):",
            state.input_timezone.value(),
            state.focused_field == 12,
        ),
        (
            "Account Status:",
            state.input_account_status.value(),
            state.focused_field == 13,
        ),
        (
            "Emergency Name:",
            state.input_emergency_contact_name.value(),
            state.focused_field == 14,
        ),
        (
            "Emergency Phone:",
            state.input_emergency_contact_phone.value(),
            state.focused_field == 15,
        ),
    ];

    render_form_modal(
        f,
        theme,
        popup_area,
        title_text,
        &fields,
        state.scroll_offset,
        state.error.as_deref(),
    );
}

/// Render edit user modal
fn render_edit_modal(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &UsersViewState,
    area: Rect,
) {
    let popup_area = centered_rect(70, 80, area);

    let title_text = " Edit User (Tab/Shift+Tab: navigate, Enter: save, Esc: cancel) ";
    let password_masked = "*".repeat(state.input_password.value().len());
    let pin_masked = "*".repeat(state.input_pin.value().len());

    let fields = vec![
        // Core fields
        ("Username:", state.input_username.value(), false), // Username not editable
        (
            "Display Name:",
            state.input_display_name.value(),
            state.focused_field == 1,
        ),
        (
            "New Password:",
            password_masked.as_str(),
            state.focused_field == 2,
        ),
        // Enterprise fields
        (
            "Email:",
            state.input_email.value(),
            state.focused_field == 3,
        ),
        (
            "Phone Number:",
            state.input_phone_number.value(),
            state.focused_field == 4,
        ),
        (
            "Mobile Number:",
            state.input_mobile_number.value(),
            state.focused_field == 5,
        ),
        ("New PIN:", pin_masked.as_str(), state.focused_field == 6),
        (
            "Extension:",
            state.input_extension.value(),
            state.focused_field == 7,
        ),
        (
            "Department:",
            state.input_department.value(),
            state.focused_field == 8,
        ),
        (
            "Job Title:",
            state.input_job_title.value(),
            state.focused_field == 9,
        ),
        (
            "Location:",
            state.input_location.value(),
            state.focused_field == 10,
        ),
        (
            "Language (Space to select):",
            state.input_language.value(),
            state.focused_field == 11,
        ),
        (
            "Timezone (Space to select):",
            state.input_timezone.value(),
            state.focused_field == 12,
        ),
        (
            "Account Status:",
            state.input_account_status.value(),
            state.focused_field == 13,
        ),
        (
            "Emergency Name:",
            state.input_emergency_contact_name.value(),
            state.focused_field == 14,
        ),
        (
            "Emergency Phone:",
            state.input_emergency_contact_phone.value(),
            state.focused_field == 15,
        ),
    ];

    render_form_modal(
        f,
        theme,
        popup_area,
        title_text,
        &fields,
        state.scroll_offset,
        state.error.as_deref(),
    );
}

/// Render delete confirmation modal
fn render_delete_modal(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &UsersViewState,
    area: Rect,
) {
    let popup_area = centered_rect(50, 20, area);

    let username = state
        .selected_user()
        .map(|u| u.username.as_str())
        .unwrap_or("unknown");

    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("Are you sure you want to delete user ", theme.normal()),
            Span::styled(username, theme.error()),
            Span::styled("?", theme.normal()),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Press ", theme.normal()),
            Span::styled("Enter", theme.error()),
            Span::styled(" to confirm, ", theme.normal()),
            Span::styled("Esc", theme.primary()),
            Span::styled(" to cancel", theme.normal()),
        ]),
    ];

    f.render_widget(Clear, popup_area);
    let block = Block::default()
        .title(" Confirm Delete ")
        .borders(Borders::ALL)
        .border_style(theme.error());
    let paragraph = Paragraph::new(text).block(block);
    f.render_widget(paragraph, popup_area);
}

/// Render set digest password modal
fn render_set_digest_password_modal(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &UsersViewState,
    area: Rect,
) {
    let popup_area = centered_rect(60, 30, area);

    let username = state
        .selected_user()
        .map(|u| u.username.as_str())
        .unwrap_or("unknown");

    let mut text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("Set Separate SIP Password for ", theme.normal()),
            Span::styled(username, theme.primary()),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Cisco-style security: Different passwords for API and SIP",
            theme.normal(),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Digest Password: ", theme.normal()),
            Span::styled(
                if state.input_digest_password.value().is_empty() {
                    "(enter password)"
                } else {
                    "••••••••"
                },
                theme.primary(),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Press ", theme.normal()),
            Span::styled("Enter", theme.primary()),
            Span::styled(" to save, ", theme.normal()),
            Span::styled("Esc", theme.normal()),
            Span::styled(" to cancel", theme.normal()),
        ]),
    ];

    if let Some(error) = &state.error {
        text.push(Line::from(""));
        text.push(Line::from(Span::styled(
            format!("Error: {}", error),
            theme.error(),
        )));
    }

    f.render_widget(Clear, popup_area);
    let block = Block::default()
        .title(" Set Digest Password ")
        .borders(Borders::ALL)
        .border_style(theme.primary());
    let paragraph = Paragraph::new(text).block(block);
    f.render_widget(paragraph, popup_area);
}

/// Render remove digest password confirmation modal
fn render_remove_digest_password_modal(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &UsersViewState,
    area: Rect,
) {
    let popup_area = centered_rect(60, 25, area);

    let username = state
        .selected_user()
        .map(|u| u.username.as_str())
        .unwrap_or("unknown");

    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("Remove digest password for ", theme.normal()),
            Span::styled(username, theme.primary()),
            Span::styled("?", theme.normal()),
        ]),
        Line::from(""),
        Line::from(Span::styled("This will:", theme.normal())),
        Line::from(Span::styled(
            "  • Remove separate SIP password",
            theme.normal(),
        )),
        Line::from(Span::styled("  • Clear HA1 hash", theme.normal())),
        Line::from(Span::styled(
            "  • Revert to using main password for SIP",
            theme.normal(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "⚠ Phone must re-register with main password",
            theme.warning(),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Press ", theme.normal()),
            Span::styled("Enter", theme.warning()),
            Span::styled(" to confirm, ", theme.normal()),
            Span::styled("Esc", theme.primary()),
            Span::styled(" to cancel", theme.normal()),
        ]),
    ];

    f.render_widget(Clear, popup_area);
    let block = Block::default()
        .title(" Remove Digest Password ")
        .borders(Borders::ALL)
        .border_style(theme.warning());
    let paragraph = Paragraph::new(text).block(block);
    f.render_widget(paragraph, popup_area);
}

/// Helper to render a form modal with scrolling support
fn render_form_modal(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    area: Rect,
    title: &str,
    fields: &[(&str, &str, bool)],
    scroll_offset: usize,
    error: Option<&str>,
) {
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(theme.border());

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Reserve space for error message at bottom if there is one
    let error_lines = if error.is_some() { 4 } else { 0 };

    // Calculate how many fields can fit in the visible area
    // Each field takes 3 lines: label (1) + value (1) + spacing (1)
    let lines_per_field = 3;
    let available_height = inner.height.saturating_sub(2 + error_lines) as usize; // Leave room for scroll indicators and error
    let visible_fields = (available_height / lines_per_field).max(1);

    // Calculate which fields to render
    let start_field = scroll_offset;
    let end_field = (scroll_offset + visible_fields).min(fields.len());

    // Show scroll indicator at top if we're scrolled down
    let mut y = inner.y;
    if scroll_offset > 0 {
        let scroll_indicator = Line::from(vec![Span::styled(
            format!("  ▲ {} more above  ", scroll_offset),
            theme.muted(),
        )]);
        f.render_widget(
            Paragraph::new(scroll_indicator).alignment(ratatui::layout::Alignment::Center),
            Rect::new(inner.x, y, inner.width, 1),
        );
        y += 1;
    } else {
        y += 1;
    }

    // Render visible form fields
    for i in start_field..end_field {
        if let Some((label, value, focused)) = fields.get(i) {
            // Render label
            let label_line = Line::from(vec![Span::styled(*label, theme.muted())]);
            f.render_widget(
                Paragraph::new(label_line),
                Rect::new(inner.x + 2, y, inner.width - 4, 1),
            );
            y += 1;

            // Render value
            let value_style = if *focused {
                theme.selected()
            } else {
                theme.normal()
            };
            let value_line = Line::from(vec![Span::styled(format!(" {} ", value), value_style)]);
            f.render_widget(
                Paragraph::new(value_line),
                Rect::new(inner.x + 2, y, inner.width - 4, 1),
            );
            y += 1;

            // Add spacing
            y += 1;
        }
    }

    // Show error message at bottom if present
    if let Some(error_msg) = error {
        let error_y = inner.y + inner.height.saturating_sub(error_lines as u16);
        let error_text = vec![Line::from(vec![
            Span::styled("Error: ", theme.error()),
            Span::raw(error_msg),
        ])];

        let error_para = Paragraph::new(error_text)
            .wrap(ratatui::widgets::Wrap { trim: true })
            .style(theme.normal());

        f.render_widget(
            error_para,
            Rect::new(inner.x + 2, error_y, inner.width - 4, error_lines as u16),
        );
    }

    // Show scroll indicator at bottom if there are more fields below
    let remaining_below = fields.len().saturating_sub(end_field);
    if remaining_below > 0 && error.is_none() {
        // Only show scroll indicator if no error (error takes priority for bottom space)
        let scroll_indicator = Line::from(vec![Span::styled(
            format!("  ▼ {} more below  ", remaining_below),
            theme.muted(),
        )]);
        f.render_widget(
            Paragraph::new(scroll_indicator).alignment(ratatui::layout::Alignment::Center),
            Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1),
        );
    }
}

/// Render selection modal for language or timezone
fn render_selection_modal(
    f: &mut Frame,
    theme: &crate::tui::theme::Theme,
    state: &mut UsersViewState,
    area: Rect,
) {
    let popup_area = centered_rect(50, 60, area);

    f.render_widget(Clear, popup_area);

    let title = if state.focused_field == 11 {
        " Select Language (↑↓: navigate, Enter: select, Esc: cancel) "
    } else {
        " Select Timezone (↑↓: navigate, Enter: select, Esc: cancel) "
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(theme.primary());

    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    // Render the list
    if state.focused_field == 11 {
        // Language selection
        let items: Vec<ListItem> = LANGUAGES
            .iter()
            .map(|(code, name)| ListItem::new(format!("{} - {}", code, name)))
            .collect();

        let list = List::new(items)
            .highlight_style(theme.selected())
            .highlight_symbol(">> ");

        f.render_stateful_widget(list, inner, &mut state.language_list_state);
    } else if state.focused_field == 12 {
        // Timezone selection
        let items: Vec<ListItem> = TIMEZONES.iter().map(|tz| ListItem::new(*tz)).collect();

        let list = List::new(items)
            .highlight_style(theme.selected())
            .highlight_symbol(">> ");

        f.render_stateful_widget(list, inner, &mut state.timezone_list_state);
    }
}

/// Create a centered rectangle
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

/// Handle keyboard events for user management
pub async fn handle_user_action(
    state: &mut UsersViewState,
    api_client: &crate::api::ApiClient,
    event: &crate::tui::event::Event,
) -> bool {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    // Check if we're in selection mode first (highest priority)
    if state.in_selection_mode {
        match event {
            crate::tui::event::Event::Key(KeyEvent {
                code, modifiers, ..
            }) => {
                match code {
                    KeyCode::Up if *modifiers == KeyModifiers::NONE => {
                        state.selection_previous();
                        return true;
                    }
                    KeyCode::Down if *modifiers == KeyModifiers::NONE => {
                        state.selection_next();
                        return true;
                    }
                    KeyCode::Enter if *modifiers == KeyModifiers::NONE => {
                        state.confirm_selection();
                        return true;
                    }
                    KeyCode::Esc if *modifiers == KeyModifiers::NONE => {
                        state.close_selection_mode();
                        return true;
                    }
                    _ => return true, // Consume all other events in selection mode
                }
            }
            _ => return true,
        }
    }

    // Check if we're in a modal
    if state.modal != UserModal::None {
        match event {
            crate::tui::event::Event::Key(KeyEvent {
                code, modifiers, ..
            }) => {
                match code {
                    KeyCode::Esc if *modifiers == KeyModifiers::NONE => {
                        state.close_modal();
                        return true;
                    }
                    KeyCode::Tab if *modifiers == KeyModifiers::NONE => {
                        if state.modal != UserModal::Delete {
                            state.next_field();
                            // Ensure focused field is visible (assume 5 visible fields as conservative estimate)
                            state.ensure_focused_visible(5);
                        }
                        return true;
                    }
                    KeyCode::BackTab => {
                        if state.modal != UserModal::Delete {
                            state.prev_field();
                            // Ensure focused field is visible (assume 5 visible fields as conservative estimate)
                            state.ensure_focused_visible(5);
                        }
                        return true;
                    }
                    KeyCode::Enter if *modifiers == KeyModifiers::NONE => {
                        match state.modal {
                            UserModal::Add => {
                                create_user(api_client, state).await;
                            }
                            UserModal::Edit => {
                                update_user(api_client, state).await;
                            }
                            UserModal::Delete => {
                                delete_user(api_client, state).await;
                            }
                            UserModal::SetDigestPassword => {
                                set_digest_password(api_client, state).await;
                            }
                            UserModal::RemoveDigestPassword => {
                                remove_digest_password(api_client, state).await;
                            }
                            UserModal::None => {}
                        }
                        return true;
                    }
                    KeyCode::Char(' ')
                        if *modifiers == KeyModifiers::NONE && state.is_selection_field() =>
                    {
                        // Open selection mode for language/timezone fields when Space is pressed
                        state.open_selection_mode();
                        return true;
                    }
                    KeyCode::Char(_) | KeyCode::Backspace => {
                        match state.modal {
                            UserModal::Delete | UserModal::RemoveDigestPassword => {
                                // No input for confirmation modals
                            }
                            UserModal::SetDigestPassword => {
                                // Handle digest password input
                                let key_event = crossterm::event::KeyEvent::new(*code, *modifiers);
                                if let Some(req) = tui_input::backend::crossterm::to_input_request(
                                    &crossterm::event::Event::Key(key_event),
                                ) {
                                    state.input_digest_password.handle(req);
                                }
                            }
                            _ => {
                                // For selection fields, open selection mode on any character input
                                if state.is_selection_field() {
                                    state.open_selection_mode();
                                } else {
                                    // Handle regular form input for non-selection fields
                                    let key_event =
                                        crossterm::event::KeyEvent::new(*code, *modifiers);
                                    if let Some(req) =
                                        tui_input::backend::crossterm::to_input_request(
                                            &crossterm::event::Event::Key(key_event),
                                        )
                                    {
                                        state.focused_input().handle(req);
                                    }
                                }
                            }
                        }
                        return true;
                    }
                    _ => return true,
                }
            }
            _ => return true,
        }
    }

    false
}

/// Create a new user
async fn create_user(api_client: &crate::api::ApiClient, state: &mut UsersViewState) {
    let username = state.input_username.value().to_string();
    let password = state.input_password.value().to_string();
    let display_name = state.input_display_name.value().to_string();

    if username.is_empty() || password.is_empty() {
        state.error = Some("Username and password are required".to_string());
        return;
    }

    // Helper function to convert empty strings to None
    let optional = |s: &str| {
        if s.is_empty() {
            None
        } else {
            Some(s.to_string())
        }
    };

    let request = CreateUserRequest {
        username,
        password,
        display_name: optional(&display_name),
        tenant_id: None,
        enabled: Some(state.input_enabled),
        role: None,          // TODO: Add role input field to TUI
        department_id: None, // TODO: Add department_id input field to TUI
        // Enterprise fields
        email: optional(state.input_email.value()),
        phone_number: optional(state.input_phone_number.value()),
        mobile_number: optional(state.input_mobile_number.value()),
        pin: optional(state.input_pin.value()),
        extension: optional(state.input_extension.value()),
        department: optional(state.input_department.value()),
        job_title: optional(state.input_job_title.value()),
        location: optional(state.input_location.value()),
        language: optional(state.input_language.value()),
        timezone: optional(state.input_timezone.value()),
        account_status: optional(state.input_account_status.value()),
        emergency_contact_name: optional(state.input_emergency_contact_name.value()),
        emergency_contact_phone: optional(state.input_emergency_contact_phone.value()),
    };

    match api_client.create_user(&request).await {
        Ok(_) => {
            state.close_modal();
            // Refresh user list
            fetch_users(api_client, state).await;
        }
        Err(e) => {
            state.error = Some(format!("Failed to create user: {}", e));
        }
    }
}

/// Update an existing user
async fn update_user(api_client: &crate::api::ApiClient, state: &mut UsersViewState) {
    let username = state.input_username.value().to_string();
    let display_name = state.input_display_name.value().to_string();
    let password = state.input_password.value().to_string();
    let pin = state.input_pin.value().to_string();

    // Helper function to convert empty strings to None
    let optional = |s: &str| {
        if s.is_empty() {
            None
        } else {
            Some(s.to_string())
        }
    };

    let request = UpdateUserRequest {
        display_name: optional(&display_name),
        enabled: Some(state.input_enabled),
        // Enterprise fields
        email: optional(state.input_email.value()),
        phone_number: optional(state.input_phone_number.value()),
        mobile_number: optional(state.input_mobile_number.value()),
        pin: optional(&pin),
        extension: optional(state.input_extension.value()),
        department: optional(state.input_department.value()),
        job_title: optional(state.input_job_title.value()),
        location: optional(state.input_location.value()),
        language: optional(state.input_language.value()),
        timezone: optional(state.input_timezone.value()),
        account_status: optional(state.input_account_status.value()),
        emergency_contact_name: optional(state.input_emergency_contact_name.value()),
        emergency_contact_phone: optional(state.input_emergency_contact_phone.value()),
    };

    match api_client.update_user(&username, &request).await {
        Ok(_) => {
            // Update password if provided
            if !password.is_empty() {
                let _ = api_client
                    .update_password(&username, &crate::api::UpdatePasswordRequest { password })
                    .await;
            }

            state.close_modal();
            // Refresh user list
            fetch_users(api_client, state).await;
        }
        Err(e) => {
            state.error = Some(format!("Failed to update user: {}", e));
        }
    }
}

/// Delete a user
async fn delete_user(api_client: &crate::api::ApiClient, state: &mut UsersViewState) {
    if let Some(user) = state.selected_user() {
        let username = user.username.clone();

        match api_client.delete_user(&username).await {
            Ok(_) => {
                state.close_modal();
                // Refresh user list
                fetch_users(api_client, state).await;
            }
            Err(e) => {
                state.error = Some(format!("Failed to delete user: {}", e));
            }
        }
    }
}

/// Set digest password for user (Cisco-style separate SIP credentials)
async fn set_digest_password(api_client: &crate::api::ApiClient, state: &mut UsersViewState) {
    if let Some(user) = state.selected_user() {
        let username = user.username.clone();
        let digest_password = state.input_digest_password.value().to_string();

        if digest_password.is_empty() {
            state.error = Some("Digest password is required".to_string());
            return;
        }

        match api_client
            .set_digest_password(&username, &digest_password)
            .await
        {
            Ok(_) => {
                state.close_modal();
                // Refresh user list
                fetch_users(api_client, state).await;
            }
            Err(e) => {
                state.error = Some(format!("Failed to set digest password: {}", e));
            }
        }
    }
}

/// Remove digest password for user (revert to using main password for SIP)
async fn remove_digest_password(api_client: &crate::api::ApiClient, state: &mut UsersViewState) {
    if let Some(user) = state.selected_user() {
        let username = user.username.clone();

        match api_client.remove_digest_password(&username).await {
            Ok(_) => {
                state.close_modal();
                // Refresh user list
                fetch_users(api_client, state).await;
            }
            Err(e) => {
                state.error = Some(format!("Failed to remove digest password: {}", e));
            }
        }
    }
}
