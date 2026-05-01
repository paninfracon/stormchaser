use crate::app::{App, AppState};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{
        Block, Borders, Clear, HighlightSpacing, List, ListItem, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState,
    },
    Frame,
};
use std::time::Duration;
use stormchaser_model::workflow::RunStatus;

use stormchaser_model::test_report::TestCaseStatus;

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
    } else {
        render_runs_tab(f, chunks[0], app);
    }

    // Status bar
    let status_text = if let Some(err) = &app.error {
        format!("Error: {}", err)
    } else {
        "Tabs: 1-Runs 2-Backends | Panes: Tab/h/l | Nav: j/k | Scroll: [/]/PgUp/PgDn | Actions: c(reate)/e(dit)/d(elete) | Filter: f | Quit: q"
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
    } else if let Some(form) = &mut app.direct_submit_form {
        let area = centered_rect(60, 60, f.area());
        f.render_widget(Clear, area);
        form.render(area, f.buffer_mut());
    } else if app.file_browser_active {
        render_file_browser(f, app);
    }
}

fn render_login_screen(f: &mut Frame, app: &mut App) {
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

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Percentage((100 - percent_y) / 2),
                Constraint::Percentage(percent_y),
                Constraint::Percentage((100 - percent_y) / 2),
            ]
            .as_ref(),
        )
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints(
            [
                Constraint::Percentage((100 - percent_x) / 2),
                Constraint::Percentage(percent_x),
                Constraint::Percentage((100 - percent_x) / 2),
            ]
            .as_ref(),
        )
        .split(popup_layout[1])[1]
}

fn format_status(status: &str) -> String {
    match status.to_lowercase().as_str() {
        "queued" => "Queued".to_string(),
        "resolving" => "Resolving".to_string(),
        "start_pending" => "Start Pending".to_string(),
        "running" => "Running".to_string(),
        "succeeded" => "Succeeded".to_string(),
        "failed" => "Failed".to_string(),
        "aborted" => "Aborted".to_string(),
        "unpacking_sfs" => "Unpacking SFS".to_string(),
        "packing_sfs" => "Packing SFS".to_string(),
        "waiting_for_event" => "Waiting...".to_string(),
        "failed_ignored" => "Failed (Ignored)".to_string(),
        _ => {
            let mut chars = status.chars();
            match chars.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
            }
        }
    }
}

fn format_time_str(ts: &str) -> String {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
        dt.with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string()
    } else {
        ts.to_string()
    }
}

fn render_runs_tab(f: &mut Frame, area: Rect, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)].as_ref())
        .split(area);

    let list_border_color = if app.active_pane == crate::app::Pane::RunsList {
        Color::Yellow
    } else {
        Color::White
    };

    let detail_border_color = if app.active_pane == crate::app::Pane::RunDetail
        || app.active_pane == crate::app::Pane::TestResults
    {
        Color::Yellow
    } else {
        Color::White
    };

    let runs: Vec<ListItem> = app
        .runs
        .iter()
        .map(|r| {
            let status_color = match r.status {
                RunStatus::Succeeded => Color::Green,
                RunStatus::Failed => Color::Red,
                RunStatus::Running => Color::Yellow,
                RunStatus::Resolving => Color::Cyan,
                RunStatus::Queued => Color::Gray,
                _ => Color::White,
            };
            let status_text: String = r.status.clone().into();
            ListItem::new(format!(
                "{:<15} | {:<15} | {}",
                r.workflow_name,
                format_status(&status_text),
                format_time_str(&r.created_at.to_rfc3339())
            ))
            .style(Style::default().fg(status_color))
        })
        .collect();

    let runs_list = List::new(runs)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Workflow Runs ")
                .border_style(Style::default().fg(list_border_color)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_spacing(HighlightSpacing::Always);

    f.render_stateful_widget(runs_list, chunks[0], &mut app.runs_state);

    let detail_block = Block::default()
        .borders(Borders::ALL)
        .title(if app.active_pane == crate::app::Pane::TestResults {
            " Test Results (T to switch back) "
        } else {
            " Run Detail (T for Tests) "
        })
        .border_style(Style::default().fg(detail_border_color));

    if app.runs.is_empty() {
        f.render_widget(detail_block, chunks[1]);
        return;
    }

    if let Some(run) = app.selected_run.clone() {
        if app.active_pane == crate::app::Pane::TestResults {
            render_test_results(f, chunks[1], &run, detail_block);
        } else {
            render_run_detail(f, chunks[1], app, &run, detail_block);
        }
    } else {
        if app.runs_state.selected().is_some() {
            let loading_paragraph = Paragraph::new("\n\n   Loading run details...")
                .style(Style::default().fg(Color::Yellow))
                .block(detail_block);
            f.render_widget(loading_paragraph, chunks[1]);
        } else {
            f.render_widget(detail_block, chunks[1]);
        }
    }
}

fn render_run_detail(
    f: &mut Frame,
    area: Rect,
    app: &mut App,
    run: &crate::app::WorkflowRunFullDetail,
    block: Block,
) {
    f.render_widget(&block, area);
    let inner_area = area.inner(ratatui::layout::Margin {
        vertical: 1,
        horizontal: 1,
    });

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
        .split(inner_area);

    let status_text: String = run.detail.status.clone().into();
    let mut detail_text = format!(
        "Workflow: {}\nRun ID: {}\nStatus: {}\nCreated: {}\nUser: {}\n",
        run.detail.workflow_name,
        run.detail.id,
        format_status(&status_text.to_lowercase()),
        run.detail.created_at.format("%Y-%m-%d %H:%M:%S"),
        run.detail.initiating_user
    );

    if let Some(finished) = run.detail.finished_at {
        detail_text.push_str(&format!(
            "Finished: {}\nDuration: {}\n",
            finished.format("%Y-%m-%d %H:%M:%S"),
            humantime::format_duration(
                (finished - run.detail.created_at)
                    .to_std()
                    .unwrap_or(Duration::from_secs(0))
            )
        ));
    }

    detail_text.push_str("\nSteps:\n");
    detail_text.push_str(&format!(
        "{:<20} | {:<35} | {}\n",
        "Name", "Status", "Started At"
    ));
    detail_text.push_str(&format!("{:-<20}-|-{:-<35}-|-{:-<20}\n", "", "", ""));

    for (i, step_detail) in run.steps.iter().enumerate() {
        let instance = &step_detail.instance;
        let name = instance
            .get("step_name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let status = instance
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let status_fmt = format_status(status);
        let started_raw = instance
            .get("started_at")
            .and_then(|v| v.as_str())
            .or_else(|| instance.get("created_at").and_then(|v| v.as_str()));
        let started = started_raw.map(format_time_str).unwrap_or("-".to_string());
        let finished_raw = instance.get("finished_at").and_then(|v| v.as_str());

        let mut status_line = status_fmt.clone();
        if let (Some(s_str), Some(f_str)) = (started_raw, finished_raw) {
            if let (Ok(s), Ok(f)) = (
                chrono::DateTime::parse_from_rfc3339(s_str),
                chrono::DateTime::parse_from_rfc3339(f_str),
            ) {
                let std_dur = (f - s).to_std().unwrap_or(Duration::from_secs(0));
                status_line = format!(
                    "{:<15} ({})",
                    status_fmt,
                    humantime::format_duration(std_dur)
                );
            }
        }

        let prefix =
            if i == app.selected_step_index && app.active_pane == crate::app::Pane::RunDetail {
                ">> "
            } else {
                "   "
            };

        detail_text.push_str(&format!(
            "{}{:<17} | {:<35} | {}\n",
            prefix, name, status_line, started
        ));

        if status == "succeeded" {
            if let (Some(s), Some(f)) = (started_raw, finished_raw) {
                detail_text.push_str(&format!(
                    "       └─ Detailed Timing: Started at {}, Finished at {}\n",
                    s, f
                ));
            }
        }

        if i == app.selected_step_index && app.active_pane == crate::app::Pane::RunDetail {
            for entry in &step_detail.history {
                let h_status = entry
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let h_time = entry
                    .get("created_at")
                    .and_then(|v| v.as_str())
                    .map(format_time_str)
                    .unwrap_or("-".to_string());
                detail_text.push_str(&format!(
                    "       └─ {:<15} at {}\n",
                    format_status(h_status),
                    h_time
                ));
            }
        }
    }

    if !run.artifacts.is_empty() {
        detail_text.push_str("\nPublished Artifacts:\n");
        detail_text.push_str(&format!("{:<40} | {}\n", "Name", "Step Name"));
        detail_text.push_str(&format!("{:-<40}-|-{:-<36}\n", "", ""));
        for artifact in &run.artifacts {
            let step_instance_str = artifact.step_instance_id.to_string();
            let step_name = run
                .steps
                .iter()
                .find(|s| s.instance.get("id").and_then(|v| v.as_str()) == Some(&step_instance_str))
                .and_then(|s| s.instance.get("step_name"))
                .and_then(|v| v.as_str())
                .unwrap_or(&step_instance_str);

            detail_text.push_str(&format!("{:<40} | {}\n", artifact.artifact_name, step_name));
        }
    }

    let detail_paragraph =
        Paragraph::new(detail_text.clone()).scroll((app.overview_scroll as u16, 0));
    f.render_widget(detail_paragraph, right_chunks[0]);

    // Add scrollbar to overview if content exceeds height
    let overview_content_lines = detail_text.lines().count();
    let overview_height = right_chunks[0].height as usize;
    let max_overview_scroll = overview_content_lines.saturating_sub(overview_height);

    if max_overview_scroll > 0 {
        if app.overview_scroll > max_overview_scroll {
            app.overview_scroll = max_overview_scroll;
        }

        let scrollbar = Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"));
        let mut scrollbar_state = ScrollbarState::default()
            .content_length(max_overview_scroll)
            .position(app.overview_scroll);
        f.render_stateful_widget(scrollbar, right_chunks[0], &mut scrollbar_state);
    }

    // Logs in lower half
    let log_block = Block::default()
        .borders(Borders::TOP)
        .title(" Logs ")
        .border_style(Style::default().fg(Color::White));

    let log_inner = log_block.inner(right_chunks[1]);
    f.render_widget(log_block, right_chunks[1]);

    let log_lines = if app.selected_step_index < run.steps.len() {
        &run.steps[app.selected_step_index].logs
    } else {
        &app.run_logs
    };

    let log_content: Vec<ratatui::text::Line> = log_lines
        .iter()
        .map(|l| ratatui::text::Line::from(l.as_str()))
        .collect();

    let log_height = log_inner.height as usize;
    if app.log_auto_scroll && log_lines.len() > log_height {
        app.log_scroll = log_lines.len() - log_height;
    }

    let log_paragraph = Paragraph::new(log_content).scroll((app.log_scroll as u16, 0));
    f.render_widget(log_paragraph, log_inner);

    if log_lines.len() > log_height {
        let scrollbar = Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"));
        let mut scrollbar_state = ScrollbarState::default()
            .content_length(log_lines.len() - log_height)
            .position(app.log_scroll);
        f.render_stateful_widget(scrollbar, log_inner, &mut scrollbar_state);
    }
}

fn render_test_results(
    f: &mut Frame,
    area: Rect,
    run: &crate::app::WorkflowRunFullDetail,
    block: Block,
) {
    f.render_widget(&block, area);
    let inner_area = area.inner(ratatui::layout::Margin {
        vertical: 1,
        horizontal: 1,
    });

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(0)].as_ref())
        .split(inner_area);

    // 1. Summary
    let mut total = 0;
    let mut passed = 0;
    let mut failed = 0;
    let mut skipped = 0;
    let mut errors = 0;
    let mut duration_ms = 0;

    for s in &run.test_summaries {
        total += s.total_tests;
        passed += s.passed;
        failed += s.failed;
        skipped += s.skipped;
        errors += s.errors;
        duration_ms += s.duration_ms;
    }

    let summary_text = format!(
        "Aggregate Test Results:\n\
         Total Tests:   {}\n\
         Passed:        {}\n\
         Failed:        {}\n\
         Errors:        {}\n\
         Skipped:       {}\n\
         Duration:      {}ms\n",
        total, passed, failed, errors, skipped, duration_ms
    );

    let summary_paragraph =
        Paragraph::new(summary_text).style(Style::default().fg(if failed + errors > 0 {
            Color::Red
        } else {
            Color::Green
        }));
    f.render_widget(summary_paragraph, chunks[0]);

    // 2. Grid View
    let mut grid_text = ratatui::text::Text::from("\nTest Grid:\n");
    let mut current_line = ratatui::text::Line::default();
    let width = chunks[1].width as usize - 4;
    let mut count = 0;

    for tc in &run.test_cases {
        let symbol = match tc.status {
            TestCaseStatus::Passed => "✔",
            TestCaseStatus::Failed => "✘",
            TestCaseStatus::Error => "!",
            TestCaseStatus::Skipped => "○",
        };
        let color = match tc.status {
            TestCaseStatus::Passed => Color::Green,
            TestCaseStatus::Failed => Color::Red,
            TestCaseStatus::Error => Color::LightRed,
            TestCaseStatus::Skipped => Color::Yellow,
        };

        current_line.spans.push(ratatui::text::Span::styled(
            symbol,
            Style::default().fg(color),
        ));
        current_line.spans.push(ratatui::text::Span::raw(" "));
        count += 2;

        if count >= width {
            grid_text.lines.push(current_line);
            current_line = ratatui::text::Line::default();
            count = 0;
        }
    }
    if !current_line.spans.is_empty() {
        grid_text.lines.push(current_line);
    }

    f.render_widget(Paragraph::new(grid_text), chunks[1]);
}

#[allow(deprecated)]
fn render_filter_dialog(f: &mut Frame, app: &mut App) {
    let area = centered_rect(60, 40, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Filter Runs ")
        .border_style(Style::default().fg(Color::Cyan));
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints(
            [
                Constraint::Length(3), // Workflow Name
                Constraint::Length(3), // Initiating User
                Constraint::Length(3), // Repo URL
                Constraint::Length(3), // Workflow Path
                Constraint::Length(3), // Created After
                Constraint::Length(3), // Created Before
                Constraint::Length(3), // Status
                Constraint::Min(0),
            ]
            .as_ref(),
        )
        .split(area);

    let labels = [
        "Workflow Name:",
        "Initiating User:",
        "Repo URL:",
        "Workflow Path:",
        "Created After (YYYY-MM-DD):",
        "Created Before (YYYY-MM-DD):",
    ];

    for i in 0..6 {
        let mut text_area = app.filter_inputs[i].clone();
        if app.filter_focus == i {
            text_area.set_block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(labels[i])
                    .border_style(Style::default().fg(Color::Yellow)),
            );
        } else {
            text_area.set_block(Block::default().borders(Borders::ALL).title(labels[i]));
        }
        f.render_widget(&text_area, chunks[i]);
    }

    // Status filter
    let status_block = if app.filter_focus == 6 {
        Block::default()
            .borders(Borders::ALL)
            .title("Status (Arrows to change):")
            .border_style(Style::default().fg(Color::Yellow))
    } else {
        Block::default().borders(Borders::ALL).title("Status:")
    };
    let status_text = crate::app::FILTER_STATUS_OPTIONS[app.filter_status_index];
    f.render_widget(Paragraph::new(status_text).block(status_block), chunks[6]);

    f.render_widget(
        Paragraph::new("Press Enter to Apply, Esc to Cancel")
            .style(Style::default().fg(Color::Yellow)),
        chunks[7],
    );
}

fn render_file_browser(f: &mut Frame, app: &mut App) {
    let area = centered_rect(80, 80, f.area());
    f.render_widget(Clear, area);

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
        .split(area);

    // Explorer Pane
    tui_file_explorer::render(&mut app.file_explorer, f, chunks[0]);

    // Preview Pane
    let preview_block = Block::default()
        .borders(Borders::ALL)
        .title(" Preview ")
        .border_style(Style::default().fg(Color::Yellow));

    let current = app.file_explorer.current_entry();
    let preview_content = if let Some(c) = current {
        if c.is_dir {
            "Directory selected".to_string()
        } else {
            std::fs::read_to_string(&c.path)
                .unwrap_or_else(|e| format!("Could not read file: {}", e))
        }
    } else {
        "Nothing selected".to_string()
    };

    let preview = Paragraph::new(preview_content).block(preview_block);
    f.render_widget(preview, chunks[1]);
}

#[allow(deprecated)]
fn render_schedule_git_dialog(f: &mut Frame, app: &mut App) {
    let area = centered_rect(60, 30, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Schedule Workflow from Git ")
        .border_style(Style::default().fg(Color::Cyan));
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints(
            [
                Constraint::Length(3), // Repo URL
                Constraint::Length(3), // Workflow Path
                Constraint::Length(3), // Git Ref
                Constraint::Min(0),    // Help text
            ]
            .as_ref(),
        )
        .split(area);

    let labels = ["Repo URL:", "Workflow Path:", "Git Ref (branch/tag/sha):"];

    for i in 0..3 {
        let mut text_area = app.schedule_git_inputs[i].clone();
        if app.schedule_git_focus == i {
            text_area.set_block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(labels[i])
                    .border_style(Style::default().fg(Color::Yellow)),
            );
        } else {
            text_area.set_block(Block::default().borders(Borders::ALL).title(labels[i]));
        }
        f.render_widget(&text_area, chunks[i]);
    }

    f.render_widget(
        Paragraph::new("Press Enter to Schedule, Esc to Cancel")
            .style(Style::default().fg(Color::Yellow)),
        chunks[3],
    );
}

fn render_storage_backends_tab(f: &mut Frame, area: Rect, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)].as_ref())
        .split(area);

    let list_border_color = if app.active_pane == crate::app::Pane::StorageBackendsList {
        Color::Yellow
    } else {
        Color::White
    };

    let detail_border_color = if app.active_pane == crate::app::Pane::StorageBackendDetail {
        Color::Yellow
    } else {
        Color::White
    };

    let backends: Vec<ListItem> = app
        .storage_backends
        .iter()
        .map(|b| {
            let sfs_marker = if b.is_default_sfs {
                " (Default SFS)"
            } else {
                ""
            };
            ListItem::new(format!(
                "{:<20} | {:<10}{}",
                b.name,
                format!("{:?}", b.backend_type),
                sfs_marker
            ))
        })
        .collect();

    let backends_list = List::new(backends)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Storage Backends ")
                .border_style(Style::default().fg(list_border_color)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_spacing(HighlightSpacing::Always);

    f.render_stateful_widget(backends_list, chunks[0], &mut app.storage_backends_state);

    let detail_block = Block::default()
        .borders(Borders::ALL)
        .title(" Backend Detail ")
        .border_style(Style::default().fg(detail_border_color));

    if app.storage_backends.is_empty() {
        f.render_widget(detail_block, chunks[1]);
        return;
    }

    if let Some(backend) = &app.selected_storage_backend {
        let detail_text = format!(
            "ID: {}\nName: {}\nDescription: {}\nType: {:?}\nDefault SFS: {}\nCreated At: {}\n\nConfig:\n{}",
            backend.id,
            backend.name,
            backend.description.as_deref().unwrap_or("-"),
            backend.backend_type,
            backend.is_default_sfs,
            format_time_str(&backend.created_at.to_rfc3339()),
            serde_json::to_string_pretty(&backend.config).unwrap_or_default()
        );
        let detail_paragraph = Paragraph::new(detail_text).block(detail_block);
        f.render_widget(detail_paragraph, chunks[1]);
    } else {
        f.render_widget(detail_block, chunks[1]);
    }
}

fn render_storage_backend_dialog(f: &mut Frame, app: &mut App) {
    let area = centered_rect(60, 60, f.area());
    f.render_widget(Clear, area);

    let title = if app.storage_backend_edit_id.is_some() {
        " Edit Storage Backend "
    } else {
        " Create Storage Backend "
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(Color::Cyan));
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints(
            [
                Constraint::Length(3), // Name
                Constraint::Length(3), // Description
                Constraint::Min(5),    // Config (JSON)
                Constraint::Length(3), // Assume Role ARN
                Constraint::Length(3), // Type
                Constraint::Length(3), // Is Default SFS
                Constraint::Length(2), // Help text
            ]
            .as_ref(),
        )
        .split(area);

    let labels = [
        "Name:",
        "Description:",
        "Config (JSON):",
        "Assume Role ARN:",
    ];

    for i in 0..4 {
        let mut text_area = app.storage_backend_inputs[i].clone();
        if app.storage_backend_focus == i {
            text_area.set_block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(labels[i])
                    .border_style(Style::default().fg(Color::Yellow)),
            );
        } else {
            text_area.set_block(Block::default().borders(Borders::ALL).title(labels[i]));
        }
        f.render_widget(&text_area, chunks[i]);
    }

    // Type filter
    let type_block = if app.storage_backend_focus == 4 {
        Block::default()
            .borders(Borders::ALL)
            .title("Type (Arrows to change):")
            .border_style(Style::default().fg(Color::Yellow))
    } else {
        Block::default().borders(Borders::ALL).title("Type:")
    };
    let type_text = crate::app::BACKEND_TYPE_OPTIONS[app.storage_backend_type_index];
    f.render_widget(Paragraph::new(type_text).block(type_block), chunks[4]);

    // Is Default
    let default_block = if app.storage_backend_focus == 5 {
        Block::default()
            .borders(Borders::ALL)
            .title("Is Default SFS (Space to toggle):")
            .border_style(Style::default().fg(Color::Yellow))
    } else {
        Block::default()
            .borders(Borders::ALL)
            .title("Is Default SFS:")
    };
    let default_text = if app.storage_backend_is_default {
        "Yes"
    } else {
        "No"
    };
    f.render_widget(Paragraph::new(default_text).block(default_block), chunks[5]);

    f.render_widget(
        Paragraph::new("Ctrl+Enter to Save, Esc to Cancel")
            .style(Style::default().fg(Color::Yellow)),
        chunks[6],
    );
}
