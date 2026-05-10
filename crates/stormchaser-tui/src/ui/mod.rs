use crate::app::{App, AppState};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub mod cron;
pub mod dialogs;
pub mod event_rules;
pub mod runs;
pub mod storage;
pub mod utils;
pub mod webhooks;

use cron::*;
use dialogs::*;
use event_rules::*;
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
    } else if app.active_pane == crate::app::Pane::EventRulesList
        || app.active_pane == crate::app::Pane::EventRuleDetail
    {
        render_event_rules_tab(f, chunks[0], app);
    } else if app.active_pane == crate::app::Pane::CronWorkflowsList
        || app.active_pane == crate::app::Pane::CronWorkflowDetail
    {
        render_cron_workflows_tab(f, chunks[0], app);
    } else {
        render_runs_tab(f, chunks[0], app);
    }

    // Status bar
    let status_text = if let Some(err) = &app.error {
        format!("Error: {}", err)
    } else {
        "Tabs: 1-Runs 2-Backends 3-Webhooks 4-Rules 5-Cron | Panes: Tab/h/l | Nav: j/k | Scroll: [/]/PgUp/PgDn | Actions: c(reate)/e(dit)/d(elete)/r(un local) | Filter: f | Quit: q"
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
    } else if app.event_rule_dialog_active {
        render_event_rule_dialog(f, app);
    } else if app.cron_dialog_active {
        render_cron_dialog(f, app);
    } else if app.delete_run_dialog_active {
        render_delete_run_dialog(f, app);
    } else if app.approval_dialog_active {
        render_approval_dialog(f, app);
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
                Constraint::Length(2),
                Constraint::Min(3),
                Constraint::Length(3),
            ]
            .as_ref(),
        )
        .split(area);

    f.render_widget(block, area);

    let msg = if app.state == AppState::LoggingIn {
        "Authenticating... Please wait or 'q' to Quit"
    } else if app.auto_login_credentials.is_empty() {
        "Press Enter to login via Web Browser, 'q' to Quit"
    } else {
        "Select account (↑/↓) and press Enter to Auto-Login\nOr press 'b' for Web Browser login, 'q' to Quit"
    };

    f.render_widget(
        Paragraph::new(msg).style(Style::default().fg(Color::Yellow)),
        chunks[0],
    );

    if !app.auto_login_credentials.is_empty() && app.state == AppState::LoggedOut {
        let items: Vec<ratatui::widgets::ListItem> = app
            .auto_login_credentials
            .iter()
            .enumerate()
            .map(|(i, (email, _))| {
                let style = if i == app.auto_login_index {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else {
                    Style::default().fg(Color::White)
                };
                ratatui::widgets::ListItem::new(ratatui::text::Line::from(format!("> {}", email)))
                    .style(style)
            })
            .collect();
        let list =
            ratatui::widgets::List::new(items).block(Block::default().borders(Borders::NONE));
        f.render_widget(list, chunks[1]);
    }

    if let Some(err) = &app.error {
        f.render_widget(
            Paragraph::new(format!("Error: {}", err)).style(Style::default().fg(Color::Red)),
            chunks[2],
        );
    }
}
