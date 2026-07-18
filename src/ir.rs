//! Book IR types — the normalised, analyser-friendly representation of a book.
//!
//! These types represent the output of the Book IR Normalizer.
//! They are designed to be:
//! - Stable across repeated parses of the same EPUB
//! - Rich enough for LLM analysis (chapters, blocks, locations)
//! - Source-trackable (every piece of content links back to EPUB positions)

use std::sync::Arc;

/// The top-level Book Intermediate Representation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BookIr {
    pub metadata: Metadata,
    pub toc: Vec<TocEntry>,
    pub spine: Vec<SpineEntry>,
    pub chapters: Vec<Chapter>,
    pub blocks: Vec<Block>,
}

/// Book-level metadata.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Metadata {
    pub title: Option<Arc<str>>,
    pub identifier: Option<Arc<str>>,
    pub language: Option<Arc<str>>,
    pub package_version: Arc<str>,
}

/// One entry in the table of contents.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct TocEntry {
    pub label: Arc<str>,
    pub href: Arc<str>,
    pub children: Vec<TocEntry>,
}

/// One item in the EPUB reading order.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SpineEntry {
    pub spine_index: u32,
    pub idref: Arc<str>,
    pub href: Option<Arc<str>>,
    pub linear: bool,
}

/// One chapter in the book.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Chapter {
    pub spine_index: u32,
    pub href: Arc<str>,
    pub title: Arc<str>,
    pub block_count: u32,
    pub visible_text: Arc<str>,
    pub content_hash: String,
    pub is_noise: bool,
}

/// One text-bearing content block.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Block {
    pub block_id: String,
    pub chapter_index: u32,
    pub order: u32,
    pub kind: String,
    pub text: Arc<str>,
    pub text_fingerprint: String,
    pub source_ref: SourceRef,
}

/// A stable reference back to the EPUB source.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SourceRef {
    pub chapter_href: Arc<str>,
    pub spine_index: u32,
    pub node_id: u32,
    pub cfi: Option<String>,
}

/// Processing profile for the compiler.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Profile {
    Basic,
    Standard,
    Deep,
}

/// The complete compiled output ready for serialisation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CompiledOutput {
    pub book_ir: BookIr,
    pub structure: Structure,
    pub manifest: Manifest,
}

/// Structure JSON — table of contents and chapter ordering.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Structure {
    pub title: Option<Arc<str>>,
    pub spine_count: u32,
    pub chapter_count: u32,
    pub toc: Vec<TocEntry>,
    pub chapters: Vec<ChapterSummary>,
}

/// Summary of one chapter in the structure.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ChapterSummary {
    pub spine_index: u32,
    pub title: Arc<str>,
    pub block_count: u32,
    pub is_noise: bool,
}

/// Manifest file describing the compiled package.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Manifest {
    pub format_version: Arc<str>,
    pub profile: Arc<str>,
    pub source_hash: String,
    pub created_at: String,
    pub block_count: u32,
    pub chapter_count: u32,
}

/// Serialize a BookIr to deterministic JSON.
#[must_use]
pub fn book_ir_to_json(ir: &BookIr) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    push_json_str_opt(&mut out, 1, "title", ir.metadata.title.as_deref(), true);
    push_json_str_opt(
        &mut out,
        1,
        "identifier",
        ir.metadata.identifier.as_deref(),
        true,
    );
    push_json_str_opt(
        &mut out,
        1,
        "language",
        ir.metadata.language.as_deref(),
        true,
    );
    push_json_str(
        &mut out,
        1,
        "package_version",
        &ir.metadata.package_version,
        true,
    );

    indent(&mut out, 1);
    out.push_str("\"toc\": ");
    out.push_str(&serialize_toc(&ir.toc));
    out.push_str(",\n");

    indent(&mut out, 1);
    out.push_str("\"spine\": [\n");
    for (index, entry) in ir.spine.iter().enumerate() {
        indent(&mut out, 2);
        out.push('{');
        push_inline_json_u32(&mut out, "spine_index", entry.spine_index, true);
        push_inline_json_str(&mut out, "idref", &entry.idref, true);
        push_inline_json_str_opt(&mut out, "href", entry.href.as_deref(), true);
        push_inline_json_bool(&mut out, "linear", entry.linear, false);
        out.push('}');
        if index + 1 < ir.spine.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(&mut out, 1);
    out.push_str("],\n");

    indent(&mut out, 1);
    out.push_str("\"chapters\": [\n");
    for (index, chapter) in ir.chapters.iter().enumerate() {
        indent(&mut out, 2);
        out.push('{');
        push_inline_json_u32(&mut out, "spine_index", chapter.spine_index, true);
        push_inline_json_str(&mut out, "href", &chapter.href, true);
        push_inline_json_str(&mut out, "title", &chapter.title, true);
        push_inline_json_u32(&mut out, "block_count", chapter.block_count, true);
        push_inline_json_str(&mut out, "visible_text", &chapter.visible_text, true);
        push_inline_json_str(&mut out, "content_hash", &chapter.content_hash, true);
        push_inline_json_bool(&mut out, "is_noise", chapter.is_noise, false);
        out.push('}');
        if index + 1 < ir.chapters.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(&mut out, 1);
    out.push_str("],\n");

    indent(&mut out, 1);
    out.push_str("\"blocks\": [\n");
    for (index, block) in ir.blocks.iter().enumerate() {
        indent(&mut out, 2);
        out.push('{');
        push_inline_json_str(&mut out, "block_id", &block.block_id, true);
        push_inline_json_u32(&mut out, "chapter_index", block.chapter_index, true);
        push_inline_json_u32(&mut out, "order", block.order, true);
        push_inline_json_str(&mut out, "kind", &block.kind, true);
        push_inline_json_str(&mut out, "text", &block.text, true);
        push_inline_json_str(&mut out, "text_fingerprint", &block.text_fingerprint, true);
        indent(&mut out, 2);
        out.push_str("\"source_ref\": {");
        push_inline_json_str(
            &mut out,
            "chapter_href",
            &block.source_ref.chapter_href,
            true,
        );
        push_inline_json_u32(&mut out, "spine_index", block.source_ref.spine_index, true);
        push_inline_json_u32(&mut out, "node_id", block.source_ref.node_id, true);
        push_inline_json_str_opt(&mut out, "cfi", block.source_ref.cfi.as_deref(), false);
        out.push('}');
        out.push('}');
        if index + 1 < ir.blocks.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(&mut out, 1);
    out.push_str("]\n");
    out.push_str("}\n");
    out
}

fn serialize_toc(toc: &[TocEntry]) -> String {
    let mut out = String::from("[");
    for (index, entry) in toc.iter().enumerate() {
        out.push('{');
        out.push_str(&format!(
            "\"label\": \"{}\", ",
            escape_json_str(&entry.label)
        ));
        out.push_str(&format!("\"href\": \"{}\"", escape_json_str(&entry.href)));
        if !entry.children.is_empty() {
            out.push_str(&format!(
                ", \"children\": {}",
                serialize_toc(&entry.children)
            ));
        }
        out.push('}');
        if index + 1 < toc.len() {
            out.push_str(", ");
        }
    }
    out.push(']');
    out
}

fn push_json_str(out: &mut String, level: usize, name: &str, value: &str, trailing: bool) {
    indent(out, level);
    out.push('"');
    out.push_str(name);
    out.push_str("\": \"");
    out.push_str(&escape_json_str(value));
    out.push('"');
    if trailing {
        out.push(',');
    }
    out.push('\n');
}

fn push_json_str_opt(
    out: &mut String,
    level: usize,
    name: &str,
    value: Option<&str>,
    trailing: bool,
) {
    indent(out, level);
    out.push('"');
    out.push_str(name);
    out.push_str("\": ");
    if let Some(value) = value {
        out.push('"');
        out.push_str(&escape_json_str(value));
        out.push('"');
    } else {
        out.push_str("null");
    }
    if trailing {
        out.push(',');
    }
    out.push('\n');
}

fn push_inline_json_str(out: &mut String, name: &str, value: &str, trailing: bool) {
    out.push('"');
    out.push_str(name);
    out.push_str("\": \"");
    out.push_str(&escape_json_str(value));
    out.push('"');
    if trailing {
        out.push_str(", ");
    }
}

fn push_inline_json_u32(out: &mut String, name: &str, value: u32, trailing: bool) {
    out.push('"');
    out.push_str(name);
    out.push_str("\": ");
    out.push_str(&value.to_string());
    if trailing {
        out.push_str(", ");
    }
}

fn push_inline_json_bool(out: &mut String, name: &str, value: bool, trailing: bool) {
    out.push('"');
    out.push_str(name);
    out.push_str("\": ");
    out.push_str(if value { "true" } else { "false" });
    if trailing {
        out.push_str(", ");
    }
}

fn push_inline_json_str_opt(out: &mut String, name: &str, value: Option<&str>, trailing: bool) {
    out.push('"');
    out.push_str(name);
    out.push_str("\": ");
    if let Some(value) = value {
        out.push('"');
        out.push_str(&escape_json_str(value));
        out.push('"');
    } else {
        out.push_str("null");
    }
    if trailing {
        out.push_str(", ");
    }
}

fn indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str("  ");
    }
}

fn escape_json_str(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_book_ir_serializes() {
        let ir = BookIr {
            metadata: Metadata {
                title: Some(Arc::from("Test")),
                identifier: None,
                language: Some(Arc::from("en")),
                package_version: Arc::from("3.0"),
            },
            toc: vec![TocEntry {
                label: Arc::from("Chapter 1"),
                href: Arc::from("ch1.xhtml"),
                children: vec![],
            }],
            spine: vec![SpineEntry {
                spine_index: 0,
                idref: Arc::from("chapter-1"),
                href: Some(Arc::from("chapter-1.xhtml")),
                linear: true,
            }],
            chapters: vec![],
            blocks: vec![],
        };
        let json = book_ir_to_json(&ir);
        assert!(json.contains(r#""title": "Test""#));
        assert!(json.contains(r#""language": "en""#));
        assert!(json.contains(r#""label": "Chapter 1""#));
        assert!(json.contains(r#""idref": "chapter-1""#));
    }
}
