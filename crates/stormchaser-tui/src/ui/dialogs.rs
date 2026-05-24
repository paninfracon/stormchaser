use crate::app::App;
use crate::ui::utils::*;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub(crate) fn render_filter_dialog(f: &mut Frame, app: &mut App) {
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
        Paragraph::new("Press Ctrl+Enter to Apply, Esc to Cancel")
            .style(Style::default().fg(Color::Yellow)),
        chunks[7],
    );
}

pub(crate) fn render_file_browser(f: &mut Frame, app: &mut App) {
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
pub(crate) fn render_schedule_git_dialog(f: &mut Frame, app: &mut App) {
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

    let labels = [
        "Git Connection (Name or ID):",
        "Workflow Path:",
        "Git Ref (branch/tag/sha):",
    ];

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

pub(crate) fn render_approval_dialog(f: &mut Frame, app: &mut App) {
    let area = centered_rect(60, 40, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Approve Step ")
        .border_style(Style::default().fg(Color::Green));
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints(
            [
                Constraint::Min(5),    // JSON Inputs
                Constraint::Length(3), // Help text
            ]
            .as_ref(),
        )
        .split(area);

    let mut text_area = app.approval_inputs.clone();
    text_area.set_block(
        Block::default()
            .borders(Borders::ALL)
            .title("JSON Inputs:")
            .border_style(Style::default().fg(Color::Yellow)),
    );
    f.render_widget(&text_area, chunks[0]);

    f.render_widget(
        Paragraph::new("Press Ctrl+A to Approve, Esc to Cancel")
            .style(Style::default().fg(Color::Yellow)),
        chunks[1],
    );
}

pub(crate) fn render_delete_run_dialog(f: &mut Frame, app: &App) {
    let area = centered_rect(40, 20, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Confirm Deletion ")
        .border_style(Style::default().fg(Color::Red));
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints(
            [
                Constraint::Min(3),    // Message
                Constraint::Length(3), // Help text
            ]
            .as_ref(),
        )
        .split(area);

    let run_id = app
        .runs_state
        .selected()
        .and_then(|i| app.runs.get(i))
        .map(|r| r.id.to_string())
        .unwrap_or_default();

    let msg = format!(
        "\nAre you sure you want to permanently delete run \n\n{}\n\nThis cannot be undone.",
        run_id
    );

    f.render_widget(
        Paragraph::new(msg).style(Style::default().fg(Color::White)),
        chunks[0],
    );

    f.render_widget(
        Paragraph::new("Press Enter or 'y' to Confirm, 'n' or Esc to Cancel")
            .style(Style::default().fg(Color::Red)),
        chunks[1],
    );
}
