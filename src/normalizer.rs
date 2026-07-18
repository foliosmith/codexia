//! Book IR Normalizer — transforms pagelet ChapterIR into codexia BookIR
//!
//! Pipeline:
//!   EPUB bytes → pagelet open_book + spine chapters
//!              → normalise_chapters → BookIR
//!              → serialise → JSON / JSONL / structure.json

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use pagelet::{
    core::NodeId,
    document::{ChapterIr as PageletChapterIr, DocumentNode},
    epub::is_likely_noise_chapter,
};

use crate::ir::{
    Block, BookIr, Chapter, ChapterSummary, CompiledOutput, Manifest, Metadata, Profile, SourceRef,
    SpineEntry, Structure, TocEntry,
};

/// Normaliser configuration.
#[derive(Debug, Clone)]
pub struct NormaliserOptions {
    pub profile: Profile,
    pub filter_noise: bool,
}

impl Default for NormaliserOptions {
    fn default() -> Self {
        Self {
            profile: Profile::Standard,
            filter_noise: true,
        }
    }
}

/// Build a BookIR from pagelet's ChapterIR outputs.
pub fn normalise(
    chapters: &[PageletChapterIr],
    toc: &[TocEntry],
    spine: &[SpineEntry],
    metadata: &Metadata,
    options: &NormaliserOptions,
) -> BookIr {
    let noise_indices = detect_noise_chapters(chapters, options.filter_noise);
    let repeated_boundaries = if options.filter_noise {
        repeated_boundary_fingerprints(chapters, &noise_indices)
    } else {
        BTreeSet::new()
    };

    let mut ir_chapters = Vec::new();
    let mut ir_blocks = Vec::new();

    for (spine_index, chapter) in chapters.iter().enumerate() {
        let spine_index = spine_index as u32;
        let is_noise = noise_indices.contains_key(&(spine_index as usize));

        let blocks = chapter.blocks();
        let boundary_fingerprints = boundary_fingerprints(&blocks);
        let retained_blocks = if is_noise {
            Vec::new()
        } else {
            blocks
                .into_iter()
                .filter(|block| {
                    let fingerprint = block_fingerprint(block);
                    !repeated_boundaries.contains(&fingerprint)
                        || !boundary_fingerprints.contains(&fingerprint)
                })
                .collect::<Vec<_>>()
        };
        let block_count = retained_blocks.len() as u32;
        let visible_text = retained_blocks
            .iter()
            .map(|block| block.text.trim())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        let semantic_kinds = semantic_block_kinds(chapter);

        ir_chapters.push(Chapter {
            spine_index,
            href: chapter.href.clone(),
            title: chapter.title.clone(),
            block_count,
            visible_text: Arc::from(visible_text),
            content_hash: hex_encode(chapter.content_hash.as_bytes()),
            is_noise,
        });

        for block in retained_blocks {
            let text_fingerprint = block_fingerprint(&block);
            let cfi = chapter
                .node_cfi(block.node_id, spine_index, None)
                .map(|cfi| cfi.to_cfi_string());
            let kind = semantic_kinds
                .get(&block.node_id.get())
                .copied()
                .unwrap_or(block.kind.as_str())
                .to_owned();
            ir_blocks.push(Block {
                block_id: block.block_id.clone(),
                chapter_index: spine_index,
                order: block.order,
                kind,
                text: Arc::from(block.text),
                text_fingerprint,
                source_ref: SourceRef {
                    chapter_href: chapter.href.clone(),
                    spine_index,
                    node_id: block.node_id.get(),
                    cfi,
                },
            });
        }
    }

    BookIr {
        metadata: metadata.clone(),
        toc: toc.to_vec(),
        spine: spine.to_vec(),
        chapters: ir_chapters,
        blocks: ir_blocks,
    }
}

/// Build a compiled package from a BookIR.
pub fn compile(book_ir: &BookIr, profile: Profile, source_hash: &str) -> CompiledOutput {
    let chapter_summaries: Vec<ChapterSummary> = book_ir
        .chapters
        .iter()
        .map(|ch| ChapterSummary {
            spine_index: ch.spine_index,
            title: ch.title.clone(),
            block_count: ch.block_count,
            is_noise: ch.is_noise,
        })
        .collect();

    let structure = Structure {
        title: book_ir.metadata.title.clone(),
        spine_count: book_ir.chapters.len() as u32,
        chapter_count: chapter_summaries.len() as u32,
        toc: book_ir.toc.clone(),
        chapters: chapter_summaries,
    };

    let manifest = Manifest {
        format_version: Arc::from("0.1"),
        profile: Arc::from(profile_name(profile)),
        source_hash: source_hash.to_owned(),
        created_at: chrono_now(),
        block_count: book_ir.blocks.len() as u32,
        chapter_count: book_ir.chapters.len() as u32,
    };

    CompiledOutput {
        book_ir: book_ir.clone(),
        structure,
        manifest,
    }
}

fn detect_noise_chapters(chapters: &[PageletChapterIr], enabled: bool) -> BTreeMap<usize, bool> {
    if !enabled {
        return BTreeMap::new();
    }

    let spine_len = chapters.len();
    chapters
        .iter()
        .enumerate()
        .filter_map(|(index, chapter)| {
            let text = chapter.visible_text();
            is_codexia_noise_chapter(&chapter.title, &text, index, spine_len)
                .then_some((index, true))
        })
        .collect()
}

fn is_codexia_noise_chapter(
    title: &str,
    visible_text: &str,
    spine_index: usize,
    spine_len: usize,
) -> bool {
    if is_likely_noise_chapter(title, visible_text, spine_index, spine_len) {
        return true;
    }

    let title = title.trim().to_lowercase();
    let explicit_noise_titles = [
        "contents",
        "table of contents",
        "目录",
        "advertisement",
        "advertisements",
        "sponsored message",
        "about the publisher",
    ];

    explicit_noise_titles
        .iter()
        .any(|candidate| title == *candidate || title.starts_with(&format!("{candidate}:")))
}

fn repeated_boundary_fingerprints(
    chapters: &[PageletChapterIr],
    noise_indices: &BTreeMap<usize, bool>,
) -> BTreeSet<String> {
    let content_chapter_count = chapters.len().saturating_sub(noise_indices.len());
    if content_chapter_count < 2 {
        return BTreeSet::new();
    }

    let mut counts = BTreeMap::<String, usize>::new();
    for (index, chapter) in chapters.iter().enumerate() {
        if noise_indices.contains_key(&index) {
            continue;
        }
        for fingerprint in boundary_fingerprints(&chapter.blocks()) {
            *counts.entry(fingerprint).or_default() += 1;
        }
    }

    let threshold = 2.max((content_chapter_count * 3).div_ceil(5));
    counts
        .into_iter()
        .filter_map(|(fingerprint, count)| (count >= threshold).then_some(fingerprint))
        .collect()
}

fn boundary_fingerprints(blocks: &[pagelet::document::ChapterBlock]) -> BTreeSet<String> {
    let candidates = blocks
        .iter()
        .filter(|block| {
            let length = block.text.trim().chars().count();
            length > 0 && length <= 160
        })
        .collect::<Vec<_>>();
    let mut fingerprints = BTreeSet::new();
    if let Some(first) = candidates.first() {
        fingerprints.insert(block_fingerprint(first));
    }
    if let Some(last) = candidates.last() {
        fingerprints.insert(block_fingerprint(last));
    }
    fingerprints
}

fn block_fingerprint(block: &pagelet::document::ChapterBlock) -> String {
    hex_encode(block.fingerprint.hash().as_bytes())
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum SemanticContext {
    Plain,
    List,
    ListItem,
    BlockQuote,
}

fn semantic_block_kinds(chapter: &PageletChapterIr) -> BTreeMap<u32, &'static str> {
    let mut kinds = BTreeMap::new();
    collect_semantic_block_kinds(chapter, chapter.root, SemanticContext::Plain, &mut kinds);
    kinds
}

fn collect_semantic_block_kinds(
    chapter: &PageletChapterIr,
    node_id: NodeId,
    context: SemanticContext,
    kinds: &mut BTreeMap<u32, &'static str>,
) {
    let Some(node) = chapter.nodes.get(node_id) else {
        return;
    };

    match node {
        DocumentNode::Paragraph(_) => {
            let kind = match context {
                SemanticContext::Plain => "paragraph",
                SemanticContext::List => "list",
                SemanticContext::ListItem => "list-item",
                SemanticContext::BlockQuote => "blockquote",
            };
            kinds.insert(node_id.get(), kind);
        }
        DocumentNode::Heading(_) => {}
        DocumentNode::List(_) => {
            visit_semantic_children(chapter, node, SemanticContext::List, kinds);
        }
        DocumentNode::ListItem(_) => {
            let next_context = if context == SemanticContext::BlockQuote {
                context
            } else {
                SemanticContext::ListItem
            };
            visit_semantic_children(chapter, node, next_context, kinds);
        }
        DocumentNode::BlockQuote(_) => {
            visit_semantic_children(chapter, node, SemanticContext::BlockQuote, kinds);
        }
        DocumentNode::Footnote(_) => {}
        _ => visit_semantic_children(chapter, node, context, kinds),
    }
}

fn visit_semantic_children(
    chapter: &PageletChapterIr,
    node: &DocumentNode,
    context: SemanticContext,
    kinds: &mut BTreeMap<u32, &'static str>,
) {
    for child in node.children() {
        collect_semantic_block_kinds(chapter, *child, context, kinds);
    }
}

/// Serialize blocks as JSONL (one JSON object per line).
#[must_use]
pub fn blocks_jsonl(blocks: &[Block]) -> String {
    let mut out = String::new();
    for block in blocks {
        out.push('{');
        out.push_str(&format!(
            "\"block_id\": \"{}\", ",
            escape_json_str(&block.block_id)
        ));
        out.push_str(&format!("\"chapter_index\": {}, ", block.chapter_index));
        out.push_str(&format!("\"order\": {}, ", block.order));
        out.push_str(&format!("\"kind\": \"{}\", ", escape_json_str(&block.kind)));
        out.push_str(&format!("\"text\": \"{}\", ", escape_json_str(&block.text)));
        out.push_str(&format!(
            "\"source_ref\": {{\"chapter_href\": \"{}\", \"spine_index\": {}, \"node_id\": {}",
            escape_json_str(&block.source_ref.chapter_href),
            block.source_ref.spine_index,
            block.source_ref.node_id,
        ));
        if let Some(cfi) = &block.source_ref.cfi {
            out.push_str(&format!(", \"cfi\": \"{}\"", escape_json_str(cfi)));
        }
        out.push_str("}}");
        out.push_str("}\n");
    }
    out
}

/// Serialize structure.json.
#[must_use]
pub fn structure_json(structure: &Structure) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    push_field_opt(&mut out, 1, "title", structure.title.as_deref(), true);
    push_field_u32(&mut out, 1, "spine_count", structure.spine_count, true);
    push_field_u32(&mut out, 1, "chapter_count", structure.chapter_count, true);
    indent(&mut out, 1);
    out.push_str("\"chapters\": [\n");
    for (index, ch) in structure.chapters.iter().enumerate() {
        indent(&mut out, 2);
        out.push('{');
        out.push_str(&format!("\"spine_index\": {}, ", ch.spine_index));
        out.push_str(&format!("\"title\": \"{}\", ", escape_json_str(&ch.title)));
        out.push_str(&format!("\"block_count\": {}, ", ch.block_count));
        out.push_str(&format!("\"is_noise\": {}", ch.is_noise));
        out.push('}');
        if index + 1 < structure.chapters.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(&mut out, 1);
    out.push_str("]\n");
    out.push_str("}\n");
    out
}

/// Serialize manifest.json.
#[must_use]
pub fn manifest_json(manifest: &Manifest) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    push_field_str(
        &mut out,
        1,
        "format_version",
        &manifest.format_version,
        true,
    );
    push_field_str(&mut out, 1, "profile", &manifest.profile, true);
    push_field_str(&mut out, 1, "source_hash", &manifest.source_hash, true);
    push_field_str(&mut out, 1, "created_at", &manifest.created_at, true);
    push_field_u32(&mut out, 1, "block_count", manifest.block_count, true);
    push_field_u32(&mut out, 1, "chapter_count", manifest.chapter_count, false);
    out.push_str("}\n");
    out
}

// -- helpers --

fn profile_name(profile: Profile) -> &'static str {
    match profile {
        Profile::Basic => "basic",
        Profile::Standard => "standard",
        Profile::Deep => "deep",
    }
}

fn chrono_now() -> String {
    use std::time::SystemTime;
    match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => format!("{}", d.as_secs()),
        Err(_) => "unknown".to_owned(),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            use std::fmt::Write;
            let _ = write!(s, "{b:02x}");
            s
        })
}

fn push_field_str(out: &mut String, level: usize, name: &str, value: &str, trailing: bool) {
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

fn push_field_opt(out: &mut String, level: usize, name: &str, value: Option<&str>, trailing: bool) {
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

fn push_field_u32(out: &mut String, level: usize, name: &str, value: u32, trailing: bool) {
    indent(out, level);
    out.push('"');
    out.push_str(name);
    out.push_str("\": ");
    out.push_str(&value.to_string());
    if trailing {
        out.push(',');
    }
    out.push('\n');
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
    use crate::ir::Metadata;

    #[test]
    fn normalise_empty_chapters_produces_empty_ir() {
        let metadata = Metadata {
            title: Some(Arc::from("Test")),
            identifier: None,
            language: Some(Arc::from("en")),
            package_version: Arc::from("3.0"),
        };
        let ir = normalise(&[], &[], &[], &metadata, &NormaliserOptions::default());
        assert_eq!(ir.chapters.len(), 0);
        assert_eq!(ir.blocks.len(), 0);
    }
}
