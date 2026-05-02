use crate::app::App;
use crate::ui::utils::*;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Clear, HighlightSpacing, List, ListItem, Paragraph},
    Frame,
};

pub(crate) fn render_storage_backends_tab(f: &mut Frame, area: Rect, app: &mut App) {
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

pub(crate) fn render_storage_backend_dialog(f: &mut Frame, app: &mut App) {
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
