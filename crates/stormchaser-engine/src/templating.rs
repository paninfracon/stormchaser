//! Rendering of the Jinja templates workflow authors write: `JinjaRender` steps, request
//! and webhook bodies, SQL queries, and email bodies.
//!
//! A template is code the author chose, run on the engine. Unbounded, two nested loops
//! over `range(100000)` pin a worker thread for hours, and printing a large string in a
//! loop runs the engine out of memory. Every template is therefore rendered with a fuel
//! budget, which bounds how many instructions it may execute, and into an output buffer
//! that refuses to grow past a limit.
//!
//! Not bounded: memory held in variables that are never printed, such as a string
//! doubled with `~` a few dozen times. Fuel limits how often that can happen, not how
//! large the result gets.

use minijinja::{Environment, Error, ErrorKind};
use serde::Serialize;
use std::io::Write;

/// Instructions one template may execute. Ordinary templates use a few thousand.
pub const FUEL: u64 = 10_000_000;

/// Largest output one template may render.
pub const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

/// Renders `template` with `context`, within [`FUEL`] and [`MAX_OUTPUT_BYTES`].
pub fn render_str<S: Serialize>(template: &str, context: S) -> Result<String, Error> {
    let mut env = Environment::new();
    env.set_fuel(Some(FUEL));
    let template = env.template_from_str(template)?;

    let mut output = CappedOutput::default();
    let context = minijinja::value::Serde(context);
    if let Err(e) = template.render_captured_to(context, &mut output) {
        if output.overflowed {
            return Err(Error::new(
                ErrorKind::InvalidOperation,
                format!("rendered output is larger than {MAX_OUTPUT_BYTES} bytes"),
            ));
        }
        return Err(e);
    }
    String::from_utf8(output.buffer)
        .map_err(|_| Error::new(ErrorKind::InvalidOperation, "rendered output is not UTF-8"))
}

#[derive(Default)]
struct CappedOutput {
    buffer: Vec<u8>,
    overflowed: bool,
}

impl Write for CappedOutput {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if self.buffer.len() + data.len() > MAX_OUTPUT_BYTES {
            self.overflowed = true;
            return Err(std::io::Error::other("rendered output is too large"));
        }
        self.buffer.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renders_an_ordinary_template() {
        let out = render_str(
            "{% for s in steps %}{{ s }} {% endfor %}{{ run.id }}",
            json!({"steps": ["a", "b"], "run": {"id": "r"}}),
        )
        .unwrap();
        assert_eq!(out, "a b r");
    }

    #[test]
    fn stops_a_template_that_runs_too_long() {
        let err = render_str(
            "{% for a in range(100000) %}{% for b in range(100000) %}{% endfor %}{% endfor %}",
            json!({}),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::OutOfFuel, "{err}");
    }

    #[test]
    fn stops_a_template_whose_output_grows_too_large() {
        let err = render_str(
            r#"{% for i in range(100) %}{{ "x" * 10000000 }}{% endfor %}"#,
            json!({}),
        )
        .unwrap_err();
        assert!(err.to_string().contains("larger than"), "{err}");
    }
}
