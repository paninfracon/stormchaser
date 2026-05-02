use crate::app::App;
use crate::ui::utils::*;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{
        Block, Borders, HighlightSpacing, List, ListItem, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState,
    },
    Frame,
};
use std::time::Duration;
use stormchaser_model::test_report::TestCaseStatus;
use stormchaser_model::workflow::RunStatus;

pub(crate) fn render_runs_tab(f: &mut Frame, area: Rect, app: &mut App) {
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
        let empty_paragraph = Paragraph::new("\n\n   No workflow runs to display.")
            .style(Style::default().fg(Color::DarkGray))
            .block(detail_block);
        f.render_widget(empty_paragraph, chunks[1]);
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

pub(crate) fn render_run_detail(
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

        if status == "waiting_for_event" {
            detail_text
                .push_str("       └─ Action Required: Press 'A' to Approve, 'R' to Reject\n");
        }

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

    let log_paragraph =
        Paragraph::new(log_content).scroll((app.log_scroll as u16, app.log_scroll_x));
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

pub(crate) fn render_test_results(
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
