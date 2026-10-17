//! Keeps workflow-supplied HCL from overflowing the stack.
//!
//! The HCL parser and evaluator recurse once for every level of nesting, and for every
//! link in a chain of operators, in the text they are given. They have no limit of their
//! own, and a stack overflow cannot be caught: it ends the process. A few hundred
//! nested brackets, or `!` signs in a row, in a workflow, a step's condition or a rule's
//! expression would therefore take down whichever service read it.
//!
//! The guard does not try to understand the text. It counts the characters that can
//! make the parser go one level deeper (see [`complexity`]). The parser cannot recurse
//! further than that count, whatever the text means, so:
//!
//! * text whose count is over a limit is refused before it is parsed, and
//! * text within the limit is parsed on a stack sized for its count.
//!
//! Counting without understanding is what makes this safe: there is no lexer whose
//! idea of where a string or a comment ends could be made to differ from the parser's.
//! Its cost is that brackets and operators inside literal text (a shell script in a
//! template, say) are counted too, so the limits are generous.

use anyhow::{bail, Result};
use serde_json::Value;

/// Stack the parser and evaluator may use per counted character. Measured at up to 12 KiB
/// in release builds and 23 KiB in debug builds; this leaves room for both.
const STACK_PER_UNIT: usize = 32 * 1024;

/// Text counting no more than this is handled on the caller's stack, without the cost of
/// setting up another.
const INLINE_LIMIT: usize = 8;

/// Limit for one expression or template string. Override with
/// `STORMCHASER_EXPR_MAX_COMPLEXITY`.
pub const DEFAULT_EXPR_LIMIT: usize = 8192;

/// Limit for a whole workflow file. Override with `STORMCHASER_DSL_MAX_COMPLEXITY`.
pub const DEFAULT_FILE_LIMIT: usize = 16384;

/// Deepest nesting allowed in a value that parsing or evaluating produces. Whatever
/// handles the value later (serialising, comparing, dropping it) recurses over it on an
/// ordinary stack. It is also below the limit at which JSON stops being read back.
pub const MAX_VALUE_DEPTH: usize = 100;

fn env_limit(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

/// The limit for one expression or template string.
pub fn expr_limit() -> usize {
    env_limit("STORMCHASER_EXPR_MAX_COMPLEXITY", DEFAULT_EXPR_LIMIT)
}

/// The limit for a whole workflow file.
pub fn file_limit() -> usize {
    env_limit("STORMCHASER_DSL_MAX_COMPLEXITY", DEFAULT_FILE_LIMIT)
}

/// How many characters in `text` could each take the parser one level deeper: brackets,
/// braces and parentheses that open something, and the characters operators are made of.
///
/// The parser's recursion depth is at most this number. Traversals (`a.b.c`, `a[0][1]`)
/// and long lists are handled in loops and need not be counted.
pub fn complexity(text: &str) -> usize {
    text.bytes()
        .filter(|b| {
            matches!(
                b,
                b'(' | b'['
                    | b'{'
                    | b'!'
                    | b'-'
                    | b'+'
                    | b'*'
                    | b'/'
                    | b'%'
                    | b'?'
                    | b'<'
                    | b'>'
                    | b'='
                    | b'&'
                    | b'|'
            )
        })
        .count()
}

/// Runs `work`, which parses or evaluates `text`, on a stack that `text` cannot overflow,
/// or refuses if `text` counts more than `limit`.
///
/// Anything deeply nested that `work` creates must be dropped before it returns, since
/// dropping recurses too; use [`ensure_shallow`] on what it hands back.
pub fn guarded<T>(text: &str, limit: usize, work: impl FnOnce() -> Result<T>) -> Result<T> {
    let units = complexity(text);
    if units > limit {
        bail!(
            "too complex to evaluate safely: {} brackets and operators, the limit is {}",
            units,
            limit
        );
    }
    if units <= INLINE_LIMIT {
        return work();
    }
    on_larger_stack(units, work)
}

#[cfg(not(target_arch = "wasm32"))]
fn on_larger_stack<T>(units: usize, work: impl FnOnce() -> Result<T>) -> Result<T> {
    // Reserved, not used: the memory is only touched as deep as the text really nests
    stacker::grow(units * STACK_PER_UNIT + 512 * 1024, work)
}

/// The browser gives no way to ask for more stack, so only shallow text is evaluated.
#[cfg(target_arch = "wasm32")]
fn on_larger_stack<T>(units: usize, work: impl FnOnce() -> Result<T>) -> Result<T> {
    if units > 64 {
        bail!(
            "too complex to evaluate here: {} brackets and operators",
            units
        );
    }
    work()
}

/// How deeply `value` nests, found without recursing.
pub fn value_depth(value: &Value) -> usize {
    let mut deepest = 0;
    let mut pending = vec![(value, 1usize)];
    while let Some((value, depth)) = pending.pop() {
        deepest = deepest.max(depth);
        match value {
            Value::Array(items) => pending.extend(items.iter().map(|item| (item, depth + 1))),
            Value::Object(map) => pending.extend(map.values().map(|item| (item, depth + 1))),
            _ => {}
        }
    }
    deepest
}

/// Refuses a value nested more deeply than [`MAX_VALUE_DEPTH`]. Call it inside
/// [`guarded`], so that a refused value is dropped where there is stack to drop it.
pub fn ensure_shallow(value: Value) -> Result<Value> {
    let depth = value_depth(&value);
    if depth > MAX_VALUE_DEPTH {
        bail!(
            "nested too deeply: {} levels, the limit is {}",
            depth,
            MAX_VALUE_DEPTH
        );
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_count_covers_everything_that_can_nest() {
        assert_eq!(complexity("inputs.env"), 0);
        assert_eq!(complexity("a.b[0].c"), 1);
        assert_eq!(complexity("!(a == b) && c"), 6);
        assert_eq!(complexity("${upper(x)}"), 2);
        // Literal text counts too: the guard does not know it is literal
        assert_eq!(complexity("curl -s https://x/y"), 4);
    }

    #[test]
    fn text_over_the_limit_is_refused_without_being_touched() {
        let err = guarded(&"!".repeat(50), 10, || -> Result<()> {
            panic!("must not run");
        })
        .unwrap_err();
        assert!(err.to_string().contains("too complex"), "{err}");
        assert_eq!(guarded("a + b", 10, || Ok(3)).unwrap(), 3);
    }

    #[test]
    fn depth_is_measured_without_recursing() {
        assert_eq!(value_depth(&json!(1)), 1);
        assert_eq!(value_depth(&json!({"a": [1, {"b": []}]})), 4);
        // As deep as text of 5,000 brackets could make a value. It is built, measured
        // and dropped inside the guard, which is the only place with room to drop it.
        guarded(&"[".repeat(5000), usize::MAX, || {
            let mut deep = json!(0);
            for _ in 0..5000 {
                deep = json!([deep]);
            }
            assert_eq!(value_depth(&deep), 5001);
            assert!(ensure_shallow(deep).is_err());
            Ok(())
        })
        .unwrap();
        assert!(ensure_shallow(json!([[1]])).is_ok());
    }
}
