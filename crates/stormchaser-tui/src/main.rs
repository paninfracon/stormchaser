use anyhow::Result;
use clap::Parser;
use ratatui::crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::time::Duration;
use stormchaser_tui::app::{App, AppState, Pane};
use stormchaser_tui::ui::ui;
use stormchaser_tui::AppEvent;
use tokio::sync::mpsc;

use stormchaser_tui::app;

#[derive(Parser)]
struct Cli {
    #[arg(
        short,
        long,
        env = "STORMCHASER_URL",
        default_value = "http://localhost:3000"
    )]
    url: String,

    #[arg(short, long, env = "STORMCHASER_TOKEN")]
    token: Option<String>,

    #[arg(long)]
    owner: Option<String>,

    #[arg(long)]
    name: Option<String>,

    #[arg(long)]
    repo_url: Option<String>,

    #[arg(long)]
    workflow_path: Option<String>,

    #[arg(long)]
    created_after: Option<String>,

    #[arg(long)]
    created_before: Option<String>,

    #[arg(long)]
    status: Option<String>,
}

async fn handle_app_event<'a>(app: &mut App<'a>, event: AppEvent) -> bool {
    match event {
        AppEvent::Tick => {}
        AppEvent::Terminal(Event::Resize(_, _)) => {}
        AppEvent::TokenExpired if app.refresh_session().await.unwrap_or(false) => {
            app.start_listening_for_workflows().await;
            if let Some(run) = &app.selected_run {
                let id = run.detail.id;
                app.start_watching(id).await;
            }
        }
        AppEvent::TokenExpired => {
            app.state = AppState::LoggedOut;
        }
        AppEvent::Terminal(Event::Key(key)) => {
            if app.state == AppState::LoggedOut || app.state == AppState::LoggingIn {
                match key.code {
                    KeyCode::Enter if app.state == AppState::LoggedOut => {
                        let _ = app.login().await;
                    }
                    KeyCode::Char('q') => return true,
                    _ => {}
                }
            } else if app.filter_dialog_active {
                let focus_count = app.filter_inputs.len() + 1;
                match key.code {
                    KeyCode::Esc => {
                        app.filter_dialog_active = false;
                    }
                    KeyCode::Enter => {
                        let _ = app.apply_filters().await;
                    }
                    KeyCode::Up | KeyCode::BackTab => {
                        app.filter_focus = (app.filter_focus + focus_count - 1) % focus_count;
                    }
                    KeyCode::Down | KeyCode::Tab => {
                        app.filter_focus = (app.filter_focus + 1) % focus_count;
                    }
                    KeyCode::Left if app.filter_focus == 6 => {
                        let opts_len = app::FILTER_STATUS_OPTIONS.len();
                        app.filter_status_index =
                            (app.filter_status_index + opts_len - 1) % opts_len;
                    }
                    KeyCode::Right if app.filter_focus == 6 => {
                        let opts_len = app::FILTER_STATUS_OPTIONS.len();
                        app.filter_status_index = (app.filter_status_index + 1) % opts_len;
                    }
                    _ => {
                        if app.filter_focus < 6 {
                            app.filter_inputs[app.filter_focus].input(key);
                        }
                    }
                }
            } else if app.schedule_git_dialog_active {
                let focus_count = app.schedule_git_inputs.len();
                match key.code {
                    KeyCode::Esc => {
                        app.schedule_git_dialog_active = false;
                    }
                    KeyCode::Enter => {
                        let _ = app.submit_schedule_git().await;
                    }
                    KeyCode::Up | KeyCode::BackTab => {
                        app.schedule_git_focus =
                            (app.schedule_git_focus + focus_count - 1) % focus_count;
                    }
                    KeyCode::Down | KeyCode::Tab => {
                        app.schedule_git_focus = (app.schedule_git_focus + 1) % focus_count;
                    }
                    _ => {
                        app.schedule_git_inputs[app.schedule_git_focus].input(key);
                    }
                }
            } else if app.storage_backend_dialog_active {
                let focus_count = app.storage_backend_inputs.len() + 2; // name, desc, config, arn, type, is_default
                match key.code {
                    KeyCode::Esc => {
                        app.storage_backend_dialog_active = false;
                    }
                    KeyCode::Enter
                        if key
                            .modifiers
                            .contains(ratatui::crossterm::event::KeyModifiers::CONTROL) =>
                    {
                        let _ = app.submit_storage_backend_form().await;
                    }
                    KeyCode::BackTab => {
                        app.storage_backend_focus =
                            (app.storage_backend_focus + focus_count - 1) % focus_count;
                    }
                    KeyCode::Tab => {
                        app.storage_backend_focus = (app.storage_backend_focus + 1) % focus_count;
                    }
                    KeyCode::Left if app.storage_backend_focus == 4 => {
                        let opts_len = stormchaser_tui::app::BACKEND_TYPE_OPTIONS.len();
                        app.storage_backend_type_index =
                            (app.storage_backend_type_index + opts_len - 1) % opts_len;
                    }
                    KeyCode::Right if app.storage_backend_focus == 4 => {
                        let opts_len = stormchaser_tui::app::BACKEND_TYPE_OPTIONS.len();
                        app.storage_backend_type_index =
                            (app.storage_backend_type_index + 1) % opts_len;
                    }
                    KeyCode::Char(' ') | KeyCode::Enter if app.storage_backend_focus == 5 => {
                        app.storage_backend_is_default = !app.storage_backend_is_default;
                    }
                    _ => {
                        if app.storage_backend_focus < 4 {
                            app.storage_backend_inputs[app.storage_backend_focus].input(key);
                        }
                    }
                }
            } else if let Some(ref mut form) = app.direct_submit_form {
                form.handle_input(key);
                match form.result() {
                    ratatui_form::FormResult::Submitted => {
                        let _ = app.submit_direct_form().await;
                    }
                    ratatui_form::FormResult::Cancelled => {
                        app.direct_submit_form = None;
                        app.direct_submit_dsl = None;
                    }
                    ratatui_form::FormResult::Active => {}
                }
            } else if app.file_browser_active {
                match key.code {
                    KeyCode::Esc => {
                        app.file_browser_active = false;
                    }
                    KeyCode::Enter => {
                        if app.file_explorer.current_entry().is_some_and(|e| e.is_dir) {
                            let _ = app.file_explorer.handle_key(key);
                        } else {
                            let _ = app.submit_file().await;
                        }
                    }
                    _ => {
                        let _ = app.file_explorer.handle_key(key);
                    }
                }
            } else {
                match key.code {
                    KeyCode::Char('f') | KeyCode::Char('/') => {
                        app.open_filter_dialog();
                    }
                    KeyCode::Char('q') => return true,
                    KeyCode::Char('j') | KeyCode::Down => match app.active_pane {
                        Pane::RunsList => app.next_run(),
                        Pane::StorageBackendsList => app.next_storage_backend(),
                        _ => app.next_step(),
                    },
                    KeyCode::Char('k') | KeyCode::Up => match app.active_pane {
                        Pane::RunsList => app.previous_run(),
                        Pane::StorageBackendsList => app.previous_storage_backend(),
                        _ => app.previous_step(),
                    },
                    KeyCode::Char('1') => {
                        app.active_pane = Pane::RunsList;
                    }
                    KeyCode::Char('2') => {
                        app.active_pane = Pane::StorageBackendsList;
                    }
                    KeyCode::Tab => {
                        app.active_pane = match app.active_pane {
                            Pane::RunsList => Pane::RunDetail,
                            Pane::RunDetail => Pane::TestResults,
                            Pane::TestResults => Pane::StorageBackendsList,
                            Pane::StorageBackendsList => Pane::StorageBackendDetail,
                            Pane::StorageBackendDetail => Pane::RunsList,
                        };
                    }
                    KeyCode::Char('t') => {
                        if app.active_pane == Pane::TestResults {
                            app.active_pane = Pane::RunDetail;
                        } else if app.active_pane == Pane::RunDetail
                            || app.active_pane == Pane::RunsList
                        {
                            app.active_pane = Pane::TestResults;
                        }
                    }
                    KeyCode::Char('h') | KeyCode::Left => {
                        app.active_pane = match app.active_pane {
                            Pane::RunDetail | Pane::TestResults => Pane::RunsList,
                            Pane::StorageBackendDetail => Pane::StorageBackendsList,
                            other => other,
                        };
                    }
                    KeyCode::Char('l') | KeyCode::Right => {
                        app.active_pane = match app.active_pane {
                            Pane::RunsList => Pane::RunDetail,
                            Pane::StorageBackendsList => Pane::StorageBackendDetail,
                            other => other,
                        };
                    }
                    KeyCode::Char('[') => {
                        app.scroll_logs_up();
                    }
                    KeyCode::Char(']') => {
                        app.scroll_logs_down();
                    }
                    KeyCode::Char('{') => {
                        app.scroll_overview_up();
                    }
                    KeyCode::Char('}') => {
                        app.scroll_overview_down();
                    }
                    KeyCode::PageUp => {
                        for _ in 0..10 {
                            app.scroll_logs_up();
                        }
                    }
                    KeyCode::PageDown => {
                        for _ in 0..10 {
                            app.scroll_logs_down();
                        }
                    }
                    KeyCode::Char('a') => {
                        app.log_auto_scroll = !app.log_auto_scroll;
                    }
                    KeyCode::Char('c')
                        if app.active_pane == Pane::StorageBackendsList
                            || app.active_pane == Pane::StorageBackendDetail =>
                    {
                        app.open_storage_backend_dialog(false);
                    }
                    KeyCode::Char('c') => {} // No-op if not in backends tab
                    KeyCode::Char('e') => {
                        if app.active_pane == Pane::StorageBackendsList
                            || app.active_pane == Pane::StorageBackendDetail
                        {
                            app.open_storage_backend_dialog(true);
                        } else {
                            app.open_file_browser();
                        }
                    }
                    KeyCode::Char('d')
                        if app.active_pane == Pane::StorageBackendsList
                            || app.active_pane == Pane::StorageBackendDetail =>
                    {
                        let _ = app.delete_selected_storage_backend().await;
                    }
                    KeyCode::Char('d') => {} // No-op if not in backends tab
                    KeyCode::Char('r') => {
                        app.open_file_browser();
                    }
                    _ => {}
                }
            }
        }
        AppEvent::StatusUpdate(run_id, status) => {
            app.handle_status_update(run_id, status);
        }
        AppEvent::StepUpdate(run_id, step_name, status) => {
            app.handle_step_update(run_id, step_name, status);
        }
        AppEvent::LogLine(run_id, line) => {
            app.handle_log_line(run_id, line);
        }
        AppEvent::WorkflowUpdate(run) => {
            app.handle_workflow_update(run);
        }
        AppEvent::FullRunUpdate(run_detail) => {
            app.handle_full_run_update(run_detail);
        }
        AppEvent::StartWatching(run_id) => {
            app.start_watching(run_id).await;
        }
        AppEvent::LoginSuccessful(token, refresh_token) => {
            app.token = Some(token);
            app.refresh_token = refresh_token;
            app.state = AppState::LoggedIn;
            app.error = None;
            app.start_listening_for_workflows().await;
            let _ = app.refresh_runs().await;
        }
        AppEvent::LoginFailed(err) => {
            app.error = Some(err);
            app.state = AppState::LoggedOut;
        }
        _ => {}
    }
    false
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Setup app state
    let (tx, mut rx) = mpsc::channel(100);
    let mut app = App::new(cli.url, cli.token, tx.clone());
    app.filter_owner = cli.owner;
    app.filter_name = cli.name;
    app.filter_repo_url = cli.repo_url;
    app.filter_workflow_path = cli.workflow_path;
    app.filter_created_after = cli.created_after;
    app.filter_created_before = cli.created_before;
    app.filter_status = cli.status;

    if app.token.is_some() {
        app.start_listening_for_workflows().await;
        let _ = app.refresh_runs().await;
        let _ = app.refresh_storage_backends().await;
    }

    // Input loop
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        loop {
            if event::poll(Duration::from_millis(100)).unwrap() {
                let e = event::read().unwrap();
                let _ = tx_clone.send(AppEvent::Terminal(e)).await;
            } else {
                let _ = tx_clone.send(AppEvent::Tick).await;
            }
        }
    });

    let mut should_quit;
    loop {
        terminal.draw(|f| ui(f, &mut app))?;

        if let Some(event) = rx.recv().await {
            should_quit = handle_app_event(&mut app, event).await;

            while !should_quit {
                if let Ok(next_event) = rx.try_recv() {
                    should_quit = handle_app_event(&mut app, next_event).await;
                } else {
                    break;
                }
            }
        } else {
            break;
        }

        if should_quit {
            break;
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parsing() {
        let args = vec![
            "stormchaser-tui",
            "--url",
            "http://test:3000",
            "--token",
            "my-token",
            "--owner",
            "alice",
            "--status",
            "Running",
        ];
        let cli = Cli::parse_from(args);
        assert_eq!(cli.url, "http://test:3000");
        assert_eq!(cli.token.unwrap(), "my-token");
        assert_eq!(cli.owner.unwrap(), "alice");
        assert_eq!(cli.status.unwrap(), "Running");
        assert!(cli.name.is_none());
    }

    #[test]
    fn test_cli_parsing_defaults() {
        let args = vec!["stormchaser-tui"];
        let cli = Cli::parse_from(args);
        assert_eq!(cli.url, "http://localhost:3000"); // default
        assert!(cli.token.is_none());
        assert!(cli.owner.is_none());
    }
}
