use anyhow::{Context, Result};
use futures::{Stream, StreamExt};
use serde_json::json;
use serde_json::Value;
use stormchaser_client::{Client, Event};
use stormchaser_model::api::EnqueueResponse;
use stormchaser_model::RunId;

pub fn parse_key_val_list(list: Vec<String>) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    for i in list {
        if let Some((key, value)) = i.split_once('=') {
            // Try to parse as JSON first (for numbers, bools), fallback to string
            let val = serde_json::from_str(value).unwrap_or_else(|_| json!(value));
            map.insert(key.to_string(), val);
        }
    }
    map
}

/// Run inputs: `key=value` as [`parse_key_val_list`] reads them, or `key=@path`, whose
/// value is the text of the file at `path` without its final newline. A script can pass a
/// secret that way (`--input password=@<(printf %s "$PASSWORD")`) without it appearing
/// in the process list.
pub fn parse_inputs(list: Vec<String>) -> Result<serde_json::Map<String, Value>> {
    let mut literal = Vec::new();
    let mut from_files = serde_json::Map::new();
    for i in list {
        match i.split_once("=@") {
            Some((key, path)) if !key.contains('=') => {
                let text = std::fs::read_to_string(path)
                    .with_context(|| format!("Failed to read input '{}' from {}", key, path))?;
                let text = text.strip_suffix('\n').unwrap_or(&text);
                from_files.insert(key.to_string(), json!(text));
            }
            _ => literal.push(i),
        }
    }
    let mut map = parse_key_val_list(literal);
    map.extend(from_files);
    Ok(map)
}

/// Removes terminal control characters from text that came from the server.
///
/// Log lines, error bodies and step names are written by whoever ran the workflow. Left
/// alone, escape sequences in them can move the cursor, rewrite earlier output, set the
/// window title or, on some terminals, inject input. Newlines and tabs are kept.
pub fn strip_control(text: &str) -> std::borrow::Cow<'_, str> {
    let is_unsafe = |c: char| c.is_control() && c != '\n' && c != '\t';
    if text.contains(is_unsafe) {
        std::borrow::Cow::Owned(text.chars().filter(|c| !is_unsafe(*c)).collect())
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

/// Server text made safe for the stream it is going to: stripped when a person is
/// reading it in a terminal, untouched when it is piped or redirected.
pub(crate) fn for_stdout(text: &str) -> std::borrow::Cow<'_, str> {
    use std::io::IsTerminal;
    if std::io::stdout().is_terminal() {
        strip_control(text)
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

pub fn for_stderr(text: &str) -> std::borrow::Cow<'_, str> {
    use std::io::IsTerminal;
    if std::io::stderr().is_terminal() {
        strip_control(text)
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

/// Prints a response as pretty JSON.
pub fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// Prints a stream's events until it ends, breaks off or sends an `error` event. With
/// `named`, each line starts with the event's name.
pub async fn print_events(
    events: impl Stream<Item = stormchaser_client::Result<Event>>,
    named: bool,
) -> Result<()> {
    let mut events = std::pin::pin!(events);
    while let Some(event) = events.next().await {
        match event {
            Ok(event) => {
                if event.event == "error" {
                    eprintln!("Error from stream: {}", for_stderr(&event.data));
                    break;
                }
                if named {
                    println!("{}: {}", for_stdout(&event.event), for_stdout(&event.data));
                } else {
                    println!("{}", for_stdout(&event.data));
                }
            }
            Err(e) => {
                eprintln!("Stream error: {}", e);
                break;
            }
        }
    }
    Ok(())
}

pub async fn stream_run_logs(client: &Client, run_id: RunId) -> Result<()> {
    print_events(client.stream_run_logs(run_id).await?, false).await
}

pub async fn stream_run_status(client: &Client, run_id: RunId) -> Result<()> {
    print_events(client.stream_run_status(run_id).await?, true).await
}

pub fn require_token(token: Option<&str>) -> Result<&str> {
    token.context("Authentication token required (use --token or STORMCHASER_TOKEN env var)")
}

/// Prints a started run, then follows its logs (`tail`) or its status (`watch`).
pub async fn follow_run(
    client: &Client,
    run: stormchaser_client::Result<EnqueueResponse>,
    tail: bool,
    watch: bool,
) -> Result<()> {
    let run = run?;
    print_json(&run)?;
    if tail {
        println!("Streaming logs for run {}...", run.run_id);
        stream_run_logs(client, run.run_id).await?;
    } else if watch {
        println!("Watching status for run {}...", run.run_id);
        stream_run_status(client, run.run_id).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_key_val_list_strings() {
        let input = vec!["key1=value1".to_string(), "key2=value2".to_string()];
        let map = parse_key_val_list(input);
        assert_eq!(map.get("key1").unwrap().as_str().unwrap(), "value1");
        assert_eq!(map.get("key2").unwrap().as_str().unwrap(), "value2");
    }

    #[test]
    fn test_parse_key_val_list_json_types() {
        let input = vec!["num=42".to_string(), "bool=true".to_string()];
        let map = parse_key_val_list(input);
        assert_eq!(map.get("num").unwrap().as_i64().unwrap(), 42);
        assert!(map.get("bool").unwrap().as_bool().unwrap());
    }

    #[test]
    fn an_input_can_be_read_from_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("password");
        std::fs::write(&path, "s3cret=@x\n").unwrap();

        let map = parse_inputs(vec![
            format!("password=@{}", path.display()),
            "num=42".to_string(),
            "plain=a=@b".to_string(),
        ])
        .unwrap();
        assert_eq!(map["password"], "s3cret=@x");
        assert_eq!(map["num"], 42);
        assert_eq!(map["plain"], "a=@b");

        parse_inputs(vec!["missing=@/nonexistent/paninfracon".to_string()]).unwrap_err();
    }

    #[test]
    fn control_characters_are_removed_but_layout_is_kept() {
        // Clear screen, set the title, then overwrite the line with a carriage return
        let hostile = "ok\n\x1b[2J\x1b]0;owned\x07done\rFAKE\tend\u{9b}31m\x7f";
        assert_eq!(strip_control(hostile), "ok\n[2J]0;owneddoneFAKE\tend31m");
        // Ordinary text, including non-ASCII, is passed through without copying
        let plain = "naïve ✓ line\n\tindented";
        assert!(matches!(
            strip_control(plain),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    #[test]
    fn test_require_token_missing() {
        let err = require_token(None).unwrap_err();
        assert!(err.to_string().contains("Authentication token required"));
    }

    #[test]
    fn test_require_token_present() {
        let token = require_token(Some("my-token")).unwrap();
        assert_eq!(token, "my-token");
    }
}
