use anyhow::Result;
use clap::Parser;
use ratatui::crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::time::Duration;
use stormchaser_tui::app::{App, AppState};
use stormchaser_tui::ui::ui;
use stormchaser_tui::AppEvent;
use tokio::sync::mpsc;

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
                app.handle_filter_dialog_key(key).await;
            } else if app.schedule_git_dialog_active {
                app.handle_schedule_git_dialog_key(key).await;
            } else if app.storage_backend_dialog_active {
                app.handle_storage_backend_dialog_key(key).await;
            } else if app.webhook_dialog_active {
                app.handle_webhook_dialog_key(key).await;
            } else if app.approval_dialog_active {
                app.handle_approval_dialog_key(key).await;
            } else if app.direct_submit_form.is_some() {
                app.handle_direct_submit_form_key(key).await;
            } else if app.file_browser_active {
                app.handle_file_browser_key(key).await;
            } else {
                return app.handle_default_key(key).await;
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
        AppEvent::StepLogsFetched(run_id, step_index, logs) => {
            app.handle_step_logs_fetched(run_id, step_index, logs);
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
            let _ = app.refresh_storage_backends().await;
            let _ = app.refresh_webhooks().await;
            let _ = app.refresh_event_rules().await;
            let _ = app.refresh_cron_workflows().await;
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
        let _ = app.refresh_webhooks().await;
        let _ = app.refresh_event_rules().await;
        let _ = app.refresh_cron_workflows().await;
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
            if let AppEvent::Terminal(Event::Key(key)) = &event {
                if key.code == KeyCode::Char('l') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    terminal.clear()?;
                }
            }
            should_quit = handle_app_event(&mut app, event).await;

            while !should_quit {
                if let Ok(next_event) = rx.try_recv() {
                    if let AppEvent::Terminal(Event::Key(key)) = &next_event {
                        if key.code == KeyCode::Char('l')
                            && key.modifiers.contains(KeyModifiers::CONTROL)
                        {
                            terminal.clear()?;
                        }
                    }
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
