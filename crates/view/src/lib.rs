//! HTML rendering for noal.
//!
//! Every function here maps owned data to markup and does nothing else. No
//! database handle, no request, no clock. That keeps rendering testable by
//! comparing strings, and keeps the shell free to fetch data however it likes.
//!
//! noal serves HTML to htmx, so there are two kinds of response. A *page* is a
//! full document with the chrome. A *fragment* is the bare element that htmx
//! swaps into an existing page. Handlers pick one; the templates never guess.
#![deny(clippy::unwrap_used, clippy::expect_used)]
#![deny(missing_docs)]

pub mod ask;
pub mod layout;
pub mod pages;
pub mod render;
pub mod windows;
