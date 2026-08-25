//! The ask pipeline's pure half: what the model is told, what it returns, and
//! what the page is given.
//!
//! The shell calls the model and the database. Everything that decides *what
//! to send* and *what a result means* is here, as functions over values.

pub mod outcome;
pub mod parent_url;
pub mod pipeline;
pub mod plan;
pub mod prompt;
pub mod validator;

/// The schema description every plan prompt carries.
pub const CATALOG: &str = include_str!("catalog.md");
