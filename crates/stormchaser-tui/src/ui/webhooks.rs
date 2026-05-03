use crate::app::App;
use crate::ui::utils::*;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Clear, HighlightSpacing, List, ListItem, Paragraph},
    Frame,
};

pub(crate) fn render_webhooks_tab(f: &mut Frame, area: Rect, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)].as_ref())
        .split(area);

    let list_border_color = if app.active_pane == crate::app::Pane::WebhooksList {
        Color::Yellow
    } else {
        Color::White
    };

    let detail_border_color = if app.active_pane == crate::app::Pane::WebhookDetail {
        Color::Yellow
    } else {
        Color::White
    };

    let hooks: Vec<ListItem> = app
        .webhooks
        .iter()
        .map(|w| {
            let active_marker = if w.is_active {
                " (Active)"
            } else {
                " (Inactive)"
            };
            ListItem::new(format!(
                "{:<20} | {:<10}{}",
                w.name, w.source_type, active_marker
            ))
        })
        .collect();

    let hooks_list = List::new(hooks)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Webhooks ")
                .border_style(Style::default().fg(list_border_color)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_spacing(HighlightSpacing::Always);

    f.render_stateful_widget(hooks_list, chunks[0], &mut app.webhooks_state);

    let detail_block = Block::default()
        .borders(Borders::ALL)
        .title(" Webhook Detail ")
        .border_style(Style::default().fg(detail_border_color));

    if app.webhooks.is_empty() {
        let empty_paragraph = Paragraph::new("\n\n   No webhooks to display.")
            .style(Style::default().fg(Color::DarkGray))
            .block(detail_block);
        f.render_widget(empty_paragraph, chunks[1]);
        return;
    }

    if let Some(webhook) = &app.selected_webhook {
        let detail_text = format!(
            "ID: {}\nName: {}\nDescription: {}\nSource Type: {}\nActive: {}\nSecret Token Configured: {}\nCreated At: {}\nUpdated At: {}",
            webhook.id,
            webhook.name,
            webhook.description.as_deref().unwrap_or("-"),
            webhook.source_type,
            webhook.is_active,
            webhook.secret_token.is_some(),
            format_time_str(&webhook.created_at.to_rfc3339()),
            format_time_str(&webhook.updated_at.to_rfc3339()),
        );
        let detail_paragraph = Paragraph::new(detail_text).block(detail_block);
        f.render_widget(detail_paragraph, chunks[1]);
    } else {
        f.render_widget(detail_block, chunks[1]);
    }
}

pub(crate) fn render_webhook_dialog(f: &mut Frame, app: &mut App) {
    let area = centered_rect(60, 60, f.area());
    f.render_widget(Clear, area);

    let title = if app.webhook_edit_id.is_some() {
        " Edit Webhook "
    } else {
        " Create Webhook "
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
                Constraint::Length(3), // Secret Token
                Constraint::Length(3), // Source Type
                Constraint::Length(3), // Is Active
                Constraint::Length(2), // Help text
            ]
            .as_ref(),
        )
        .split(area);

    let labels = ["Name:", "Description:", "Secret Token:"];

    for i in 0..3 {
        let mut text_area = app.webhook_inputs[i].clone();
        if app.webhook_focus == i {
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

    // Source Type filter
    let type_block = if app.webhook_focus == 3 {
        Block::default()
            .borders(Borders::ALL)
            .title("Source Type (Arrows to change):")
            .border_style(Style::default().fg(Color::Yellow))
    } else {
        Block::default().borders(Borders::ALL).title("Source Type:")
    };
    let type_text = crate::app::WEBHOOK_SOURCE_TYPE_OPTIONS[app.webhook_source_type_index];
    f.render_widget(Paragraph::new(type_text).block(type_block), chunks[3]);

    // Is Active
    let active_block = if app.webhook_focus == 4 {
        Block::default()
            .borders(Borders::ALL)
            .title("Is Active (Space to toggle):")
            .border_style(Style::default().fg(Color::Yellow))
    } else {
        Block::default().borders(Borders::ALL).title("Is Active:")
    };
    let active_text = if app.webhook_is_active { "Yes" } else { "No" };
    f.render_widget(Paragraph::new(active_text).block(active_block), chunks[4]);

    f.render_widget(
        Paragraph::new("Ctrl+Enter to Save, Esc to Cancel")
            .style(Style::default().fg(Color::Yellow)),
        chunks[5],
    );
}
