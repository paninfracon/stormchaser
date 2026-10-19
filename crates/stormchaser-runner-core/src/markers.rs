//! The agent reports what it parked, pushed and collected as JSON on log lines that
//! start with a marker; the runners read those lines back.

use serde::de::DeserializeOwned;

/// The storage hashes the agent parked: `{"<storage name>": "<sha256>"}`.
pub const PARKED_STORAGE_HASHES: &str = "Parked storage hashes:";
/// The artifacts the agent pushed, by name.
pub const PARKED_ARTIFACTS: &str = "Parked artifacts:";
/// The test reports the agent collected, by name.
pub const COLLECTED_TEST_REPORTS: &str = "Collected test reports JSON:";

/// The JSON on the first line carrying `marker` that parses as `T`.
pub fn find_marked_json<'a, T: DeserializeOwned>(
    lines: impl IntoIterator<Item = &'a str>,
    marker: &str,
) -> Option<T> {
    lines.into_iter().find_map(|line| {
        let after = &line[line.find(marker)? + marker.len()..];
        let json = &after[after.find('{')?..];
        serde_json::from_str(json.trim_end()).ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn the_first_marked_line_that_parses_is_read() {
        let logs = "starting\n\
                    2026-10-19T10:00:00Z INFO Parked storage hashes: not json\n\
                    2026-10-19T10:00:01Z INFO Parked storage hashes: {\"work\":\"abc\"}\n\
                    Parked storage hashes: {\"work\":\"later\"}";
        let hashes: HashMap<String, String> =
            find_marked_json(logs.lines(), PARKED_STORAGE_HASHES).unwrap();
        assert_eq!(hashes["work"], "abc");
        assert!(
            find_marked_json::<HashMap<String, String>>(logs.lines(), PARKED_ARTIFACTS).is_none()
        );
    }

    #[test]
    fn json_before_the_marker_is_not_read() {
        let line = r#"{"forged":"x"} Collected test reports JSON: {"unit":{}}"#;
        let reports: HashMap<String, serde_json::Value> =
            find_marked_json([line], COLLECTED_TEST_REPORTS).unwrap();
        assert!(reports.contains_key("unit"));
    }
}
