use crate::app::{App, AppState};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub mod dialogs;
pub mod runs;
pub mod storage;
pub mod utils;
pub mod webhooks;

use dialogs::*;
use runs::*;
use storage::*;
use utils::*;
use webhooks::*;

/// Renders the main user interface based on the current application state.
pub fn ui(f: &mut Frame, app: &mut App) {
    if app.state == AppState::LoggedOut || app.state == AppState::LoggingIn {
        render_login_screen(f, app);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints([Constraint::Min(0), Constraint::Length(3)].as_ref())
        .split(f.area());

    if app.active_pane == crate::app::Pane::StorageBackendsList
        || app.active_pane == crate::app::Pane::StorageBackendDetail
    {
        render_storage_backends_tab(f, chunks[0], app);
    } else if app.active_pane == crate::app::Pane::WebhooksList
        || app.active_pane == crate::app::Pane::WebhookDetail
    {
        render_webhooks_tab(f, chunks[0], app);
    } else {
        render_runs_tab(f, chunks[0], app);
    }

    // Status bar
    let status_text = if let Some(err) = &app.error {
        format!("Error: {}", err)
    } else {
        "Tabs: 1-Runs 2-Backends 3-Webhooks | Panes: Tab/h/l | Nav: j/k | Scroll: [/]/PgUp/PgDn | Actions: c(reate)/e(dit)/d(elete) | Filter: f | Quit: q"
            .to_string()
    };
    let status_bar = Paragraph::new(status_text)
        .block(Block::default().borders(Borders::ALL))
        .style(Style::default().fg(if app.error.is_some() {
            Color::Red
        } else {
            Color::White
        }));
    f.render_widget(status_bar, chunks[1]);

    if app.filter_dialog_active {
        render_filter_dialog(f, app);
    } else if app.schedule_git_dialog_active {
        render_schedule_git_dialog(f, app);
    } else if app.storage_backend_dialog_active {
        render_storage_backend_dialog(f, app);
    } else if app.webhook_dialog_active {
        render_webhook_dialog(f, app);
    } else if app.approval_dialog_active {
        render_approval_dialog(f, app);
    } else if let Some(form) = &mut app.direct_submit_form {
        let area = centered_rect(60, 60, f.area());
        f.render_widget(Clear, area);
        form.render(area, f.buffer_mut());
    } else if app.file_browser_active {
        render_file_browser(f, app);
    }
}

pub(crate) fn render_login_screen(f: &mut Frame, app: &mut App) {
    let area = centered_rect(60, 20, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Login to Stormchaser ")
        .border_style(Style::default().fg(Color::Cyan));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints(
            [
                Constraint::Length(1),
                Constraint::Length(3),
                Constraint::Min(0),
            ]
            .as_ref(),
        )
        .split(area);

    f.render_widget(block, area);

    let msg = if app.state == AppState::LoggingIn {
        "Opening Web Browser for authentication... Waiting for login to complete. 'q' to Quit"
    } else {
        "Press Enter to login via Web Browser, 'q' to Quit"
    };

    f.render_widget(
        Paragraph::new(msg).style(Style::default().fg(Color::Yellow)),
        chunks[0],
    );

    if let Some(err) = &app.error {
        f.render_widget(
            Paragraph::new(format!("Error: {}", err)).style(Style::default().fg(Color::Red)),
            chunks[1],
        );
    }
}
