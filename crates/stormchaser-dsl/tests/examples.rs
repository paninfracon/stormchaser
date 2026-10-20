//! Every workflow in the repository, and every HCL example in the documentation, parses.
//!
//! The parser refuses blocks, attributes and values it does not understand, so an
//! example that documents something the parser does not read fails here instead of
//! being silently ignored when someone copies it.

#![expect(
    clippy::disallowed_methods,
    reason = "classifies documentation examples, which are small and trusted"
)]

use std::path::{Path, PathBuf};
use stormchaser_dsl::StormchaserParser;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf()
}

fn files_with_extension(dir: &Path, extension: &str, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy();
        if path.is_dir() {
            if name != "target" && name != "node_modules" && !name.starts_with('.') {
                files_with_extension(&path, extension, found);
            }
        } else if path.extension().is_some_and(|e| e == extension) {
            found.push(path);
        }
    }
}

/// The ```hcl blocks of a Markdown file, with the line each starts on.
fn hcl_blocks(markdown: &str) -> Vec<(usize, String)> {
    let mut blocks = Vec::new();
    let mut current: Option<(usize, String)> = None;
    for (number, line) in markdown.lines().enumerate() {
        match current.as_mut() {
            None if line.trim_start() == "```hcl" => current = Some((number + 2, String::new())),
            None => {}
            Some(_) if line.trim_start().starts_with("```") => blocks.push(current.take().unwrap()),
            Some((_, text)) => {
                text.push_str(line);
                text.push('\n');
            }
        }
    }
    blocks
}

/// Splits HCL text into its top-level attributes and blocks, each with the comments
/// before it. Brackets inside strings are skipped; heredocs are not handled.
fn top_level_items(text: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    for line in text.lines() {
        current.push_str(line);
        current.push('\n');
        let mut in_string = false;
        let mut escaped = false;
        for c in line.chars() {
            match c {
                _ if escaped => escaped = false,
                '\\' if in_string => escaped = true,
                '"' => in_string = !in_string,
                '{' | '[' | '(' if !in_string => depth += 1,
                '}' | ']' | ')' if !in_string => depth -= 1,
                _ => {}
            }
        }
        let trimmed = line.trim();
        let comment_or_blank =
            trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//");
        if depth == 0 && !comment_or_blank {
            items.push(std::mem::take(&mut current));
        }
    }
    items
}

/// The identifier or attribute name an item starts with, past its comments.
fn first_word(item: &str) -> &str {
    item.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("//"))
        .and_then(|l| {
            l.split(|c: char| c.is_whitespace() || c == '=' || c == '{')
                .next()
        })
        .unwrap_or("")
}

/// Blocks that only a step holds: an example made of these is the inside of a step.
const STEP_BLOCKS: &[&str] = &["spec", "params", "retry", "outputs", "reports"];

/// Makes a documentation example a whole workflow. An example of a step's settings is
/// put in a step, `step` blocks go in the workflow's `steps`, and everything else in
/// the workflow itself. An example that is not valid HCL is returned as it is, for the
/// parser to report.
fn as_workflow(example: &str) -> String {
    let Ok(body) = hcl::from_str::<hcl::Body>(example) else {
        return example.to_string();
    };
    if body
        .blocks()
        .any(|b| matches!(b.identifier(), "workflow" | "workflow_template"))
    {
        return example.to_string();
    }
    if body.blocks().next().is_some()
        && body.blocks().all(|b| STEP_BLOCKS.contains(&b.identifier()))
    {
        return format!(
            "workflow \"example\" {{\n  steps {{\n    step \"example\" \"RunContainer\" {{\n{example}\n    }}\n  }}\n}}\n"
        );
    }

    let (mut file, mut workflow, mut steps) = (String::new(), String::new(), String::new());
    for item in top_level_items(example) {
        match first_word(&item) {
            "stormchaser_dsl_version" => file.push_str(&item),
            "step" => steps.push_str(&item),
            _ => workflow.push_str(&item),
        }
    }
    format!("{file}workflow \"example\" {{\n{workflow}\n  steps {{\n{steps}\n  }}\n}}\n")
}

#[test]
fn every_workflow_in_the_repository_parses() {
    let root = repo_root();
    let mut files = Vec::new();
    files_with_extension(&root, "storm", &mut files);
    files.sort();
    assert!(files.len() > 20, "found only {} workflows", files.len());

    let parser = StormchaserParser::new();
    let failures: Vec<String> = files
        .iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(path).unwrap();
            parser
                .parse(&text)
                .err()
                .map(|e| format!("{}: {:#}", path.strip_prefix(&root).unwrap().display(), e))
        })
        .collect();
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn every_hcl_example_in_the_documentation_parses() {
    let root = repo_root();
    let mut files = vec![root.join("README.md")];
    files_with_extension(&root.join("docs"), "md", &mut files);
    files.sort();

    let parser = StormchaserParser::new();
    let mut examples = 0;
    let mut failures = Vec::new();
    for path in &files {
        let markdown = std::fs::read_to_string(path).unwrap();
        for (line, example) in hcl_blocks(&markdown) {
            examples += 1;
            if let Err(e) = parser.parse(&as_workflow(&example)) {
                failures.push(format!(
                    "{}:{}: {:#}",
                    path.strip_prefix(&root).unwrap().display(),
                    line,
                    e
                ));
            }
        }
    }
    assert!(examples > 40, "found only {} examples", examples);
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
