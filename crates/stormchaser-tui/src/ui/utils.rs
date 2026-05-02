use ratatui::layout::{Constraint, Direction, Layout, Rect};

pub(crate) fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
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

pub(crate) fn format_status(status: &str) -> String {
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

pub(crate) fn format_time_str(ts: &str) -> String {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
        #[cfg(test)]
        let dt = dt.with_timezone(&chrono::Utc);
        #[cfg(not(test))]
        let dt = dt.with_timezone(&chrono::Local);

        dt.format("%Y-%m-%d %H:%M:%S").to_string()
    } else {
        ts.to_string()
    }
}
