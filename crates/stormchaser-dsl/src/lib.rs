//! Workflow DSL parser for Stormchaser.
//!
//! This module provides parsing capabilities to translate HCL-based workflow
//! definitions into the internal `Workflow` model.

/// Abstract Syntax Tree components for the DSL.
pub mod ast;
pub mod hcl_schema;
pub mod parser;

pub use parser::StormchaserParser;
