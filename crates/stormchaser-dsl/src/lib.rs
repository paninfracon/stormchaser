//! Workflow DSL parser for Stormchaser.
//!
//! This module provides parsing capabilities to translate HCL-based workflow
//! definitions into the internal `Workflow` model.

/// Abstract Syntax Tree components for the DSL.
pub mod ast;
/// The input form derived from a workflow.
pub mod form;
pub mod hcl_schema;
/// Step libraries, applied to the steps that name them.
pub mod libraries;
pub mod parser;

pub use parser::StormchaserParser;
