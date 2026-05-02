use crate::app::App;
use crate::ui::utils::*;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Clear, HighlightSpacing, List, ListItem, Paragraph},
    Frame,
};

pub(crate) fn render_cron_workflows_tab(f: &mut Frame, area: Rect, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)].as_ref())
        .split(area);

    let list_border_color = if app.active_pane == crate::app::Pane::CronWorkflowsList {
        Color::Yellow
    } else {
        Color::White
    };

    let detail_border_color = if app.active_pane == crate::app::Pane::CronWorkflowDetail {
        Color::Yellow
    } else {
        Color::White
    };

    let crons: Vec<ListItem> = app
        .cron_workflows
        .iter()
        .map(|c| {
            let active_marker = if c.is_active {
                " (Active)"
            } else {
                " (Inactive)"
            };
            ListItem::new(format!(
                "{:<20} | {:<10}{}",
                c.name, c.cronspec, active_marker
            ))
        })
        .collect();

    let crons_list = List::new(crons)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Cron Workflows ")
                .border_style(Style::default().fg(list_border_color)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_spacing(HighlightSpacing::Always);

    f.render_stateful_widget(crons_list, chunks[0], &mut app.cron_workflows_state);

    let detail_block = Block::default()
        .borders(Borders::ALL)
        .title(" Cron Workflow Detail ")
        .border_style(Style::default().fg(detail_border_color));

    if app.cron_workflows.is_empty() {
        let empty_paragraph = Paragraph::new("\n\n   No cron workflows to display.")
            .style(Style::default().fg(Color::DarkGray))
            .block(detail_block);
        f.render_widget(empty_paragraph, chunks[1]);
        return;
    }

    if let Some(cron) = &app.selected_cron_workflow {
        let detail_text = format!(
            "ID: {}\nName: {}\nDescription: {}\nCron Spec: {}\nWorkflow: {}\nRepo: {}\nPath: {}\nGit Ref: {}\nInputs:\n{}\nSecret Token: {}\nActive: {}\nExternal Job ID: {}\nCreated At: {}\nUpdated At: {}",
            cron.id,
            cron.name,
            cron.description.as_deref().unwrap_or("-"),
            cron.cronspec,
            cron.workflow_name,
            cron.repo_url,
            cron.workflow_path,
            cron.git_ref,
            serde_json::to_string_pretty(&cron.inputs).unwrap_or_default(),
            cron.secret_token,
            cron.is_active,
            cron.external_job_id.as_deref().unwrap_or("-"),
            format_time_str(&cron.created_at.to_rfc3339()),
            format_time_str(&cron.updated_at.to_rfc3339()),
        );
        let detail_paragraph = Paragraph::new(detail_text).block(detail_block);
        f.render_widget(detail_paragraph, chunks[1]);
    } else {
        f.render_widget(detail_block, chunks[1]);
    }
}

pub(crate) fn render_cron_dialog(f: &mut Frame, app: &mut App) {
    let area = centered_rect(80, 80, f.area());
    f.render_widget(Clear, area);

    let title = if app.cron_edit_id.is_some() {
        " Edit Cron Workflow "
    } else {
        " Create Cron Workflow "
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
                Constraint::Length(3), // Cron Spec
                Constraint::Length(3), // Workflow Name
                Constraint::Length(3), // Repo URL
                Constraint::Length(3), // Workflow Path
                Constraint::Length(3), // Git Ref
                Constraint::Length(6), // Inputs
                Constraint::Length(3), // Is Active
                Constraint::Length(2), // Help text
            ]
            .as_ref(),
        )
        .split(area);

    let labels = [
        "Name:",
        "Description:",
        "Cron Spec:",
        "Workflow Name:",
        "Repo URL:",
        "Workflow Path:",
        "Git Ref:",
        "Inputs (JSON):",
    ];

    for i in 0..8 {
        let mut text_area = app.cron_inputs[i].clone();
        if app.cron_focus == i {
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

    // Is Active
    let active_block = if app.cron_focus == 8 {
        Block::default()
            .borders(Borders::ALL)
            .title("Active (Space to toggle):")
            .border_style(Style::default().fg(Color::Yellow))
    } else {
        Block::default().borders(Borders::ALL).title("Active:")
    };
    let active_text = if app.cron_is_active {
        "[X] Yes"
    } else {
        "[ ] No"
    };
    f.render_widget(Paragraph::new(active_text).block(active_block), chunks[8]);

    f.render_widget(
        Paragraph::new("Press Ctrl+Enter to Save, Esc to Cancel")
            .style(Style::default().fg(Color::Yellow)),
        chunks[9],
    );
}
