use crate::app::App;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
    Frame,
};

pub fn render_pending_approvals_tab(f: &mut Frame, area: Rect, app: &mut App) {
    let chunks = Layout::default()
        .direction(ratatui::layout::Direction::Horizontal)
        .constraints([Constraint::Percentage(100)].as_ref())
        .split(area);

    let block = Block::default()
        .title(" Pending Approvals ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(
            if app.active_pane == crate::app::Pane::PendingApprovalsList {
                Color::Yellow
            } else {
                Color::White
            },
        ));

    let items: Vec<ListItem> = app
        .pending_approvals
        .iter()
        .map(|run| {
            let id = run.id;
            let name = &run.workflow_name;
            let status = &run.status;
            let time = run.created_at.format("%Y-%m-%d %H:%M:%S").to_string();
            let content = Line::from(vec![
                Span::styled(
                    format!("{:<8} ", id.to_string().chars().take(8).collect::<String>()),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(format!("{:<20} ", name), Style::default().fg(Color::White)),
                Span::styled(
                    format!("{:<15} ", format!("{:?}", status)),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(time, Style::default().fg(Color::DarkGray)),
            ]);
            ListItem::new(content)
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");

    f.render_stateful_widget(list, chunks[0], &mut app.pending_approvals_state);
}
