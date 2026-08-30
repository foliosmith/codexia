#![forbid(unsafe_code)]
//! codexia — compile EPUB into structured Book Intelligence Packages.
//!
//! This crate provides the compiler pipeline that transforms raw EPUB data
//! (parsed by pagelet) into a normalised Book IR suitable for LLM analysis,
//! grounding validation, and reader sessions.

pub mod book_analysis;
pub mod chapter_analysis;
pub mod epub_parser;
pub mod ir;
pub mod normalizer;
pub mod pipeline;
pub mod reader_cards;
pub mod runtime_api;
pub mod studio;
pub mod web_runtime;

/// Static build metadata for the codexia crate.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct BuildInfo {
    pub crate_name: &'static str,
    pub version: &'static str,
}

#[must_use]
pub const fn build_info() -> BuildInfo {
    BuildInfo {
        crate_name: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
    }
}
