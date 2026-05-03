use crate::app::App;
use crate::ui::utils::*;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Clear, HighlightSpacing, List, ListItem, Paragraph},
    Frame,
};

pub(crate) fn render_event_rules_tab(f: &mut Frame, area: Rect, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)].as_ref())
        .split(area);

    let list_border_color = if app.active_pane == crate::app::Pane::EventRulesList {
        Color::Yellow
    } else {
        Color::White
    };

    let detail_border_color = if app.active_pane == crate::app::Pane::EventRuleDetail {
        Color::Yellow
    } else {
        Color::White
    };

    let rules: Vec<ListItem> = app
        .event_rules
        .iter()
        .map(|r| {
            let active_marker = if r.is_active {
                " (Active)"
            } else {
                " (Inactive)"
            };
            ListItem::new(format!(
                "{:<20} | {:<10}{}",
                r.name, r.event_type_pattern, active_marker
            ))
        })
        .collect();

    let rules_list = List::new(rules)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Event Rules ")
                .border_style(Style::default().fg(list_border_color)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_spacing(HighlightSpacing::Always);

    f.render_stateful_widget(rules_list, chunks[0], &mut app.event_rules_state);

    let detail_block = Block::default()
        .borders(Borders::ALL)
        .title(" Event Rule Detail ")
        .border_style(Style::default().fg(detail_border_color));

    if app.event_rules.is_empty() {
        let empty_paragraph = Paragraph::new("\n\n   No event rules to display.")
            .style(Style::default().fg(Color::DarkGray))
            .block(detail_block);
        f.render_widget(empty_paragraph, chunks[1]);
        return;
    }

    if let Some(rule) = &app.selected_event_rule {
        let detail_text = format!(
            "ID: {}\nName: {}\nDescription: {}\nWebhook ID: {}\nEvent Type Pattern: {}\nCondition Expr: {}\nWorkflow: {}\nRepo: {}\nPath: {}\nGit Ref: {}\nInput Mappings:\n{}\nActive: {}\nCreated At: {}\nUpdated At: {}",
            rule.id,
            rule.name,
            rule.description.as_deref().unwrap_or("-"),
            rule.webhook_id.map(|id| id.to_string()).unwrap_or_else(|| "-".to_string()),
            rule.event_type_pattern,
            rule.condition_expr.as_deref().unwrap_or("-"),
            rule.workflow_name,
            rule.repo_url,
            rule.workflow_path,
            rule.git_ref,
            serde_json::to_string_pretty(&rule.input_mappings).unwrap_or_default(),
            rule.is_active,
            format_time_str(&rule.created_at.to_rfc3339()),
            format_time_str(&rule.updated_at.to_rfc3339()),
        );
        let detail_paragraph = Paragraph::new(detail_text).block(detail_block);
        f.render_widget(detail_paragraph, chunks[1]);
    } else {
        f.render_widget(detail_block, chunks[1]);
    }
}

pub(crate) fn render_event_rule_dialog(f: &mut Frame, app: &mut App) {
    let area = centered_rect(80, 80, f.area());
    f.render_widget(Clear, area);

    let title = if app.event_rule_edit_id.is_some() {
        " Edit Event Rule "
    } else {
        " Create Event Rule "
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
                Constraint::Length(3), // Webhook ID
                Constraint::Length(3), // Event Type Pattern
                Constraint::Length(3), // Condition Expr
                Constraint::Length(3), // Workflow Name
                Constraint::Length(3), // Repo URL
                Constraint::Length(3), // Workflow Path
                Constraint::Length(3), // Git Ref
                Constraint::Length(6), // Input Mappings
                Constraint::Length(3), // Is Active
                Constraint::Length(2), // Help text
            ]
            .as_ref(),
        )
        .split(area);

    let labels = [
        "Name:",
        "Description:",
        "Webhook ID (Optional):",
        "Event Type Pattern:",
        "Condition Expr (Optional):",
        "Workflow Name:",
        "Repo URL:",
        "Workflow Path:",
        "Git Ref:",
        "Input Mappings (JSON):",
    ];

    for i in 0..10 {
        let mut text_area = app.event_rule_inputs[i].clone();
        if app.event_rule_focus == i {
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
    let active_block = if app.event_rule_focus == 10 {
        Block::default()
            .borders(Borders::ALL)
            .title("Active (Space to toggle):")
            .border_style(Style::default().fg(Color::Yellow))
    } else {
        Block::default().borders(Borders::ALL).title("Active:")
    };
    let active_text = if app.event_rule_is_active {
        "[X] Yes"
    } else {
        "[ ] No"
    };
    f.render_widget(Paragraph::new(active_text).block(active_block), chunks[10]);

    f.render_widget(
        Paragraph::new("Press Ctrl+Enter to Save, Esc to Cancel")
            .style(Style::default().fg(Color::Yellow)),
        chunks[11],
    );
}
