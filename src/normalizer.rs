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
    core::{make_stable_block_id, BlockFingerprint, NodeId},
    document::{ChapterIr as PageletChapterIr, DocumentNode, ImageNode, LinkKind},
    epub::is_likely_noise_chapter,
};

use crate::ir::{
    Block, BookIr, Chapter, ChapterSummary, CompiledOutput, ImageReference, Manifest, Metadata,
    Profile, SourceRef, SpineEntry, Structure, TocEntry,
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
    let heading_levels = normalised_heading_levels(chapters, &noise_indices);
    let repeated_boundaries = if options.filter_noise {
        repeated_boundary_fingerprints(chapters, &noise_indices)
    } else {
        BTreeSet::new()
    };

    let mut anchor_blocks = BTreeMap::new();
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
                    if block.text.trim().is_empty() {
                        return false;
                    }
                    let fingerprint = block_fingerprint(block);
                    !repeated_boundaries.contains(&fingerprint)
                        || !boundary_fingerprints.contains(&fingerprint)
                })
                .collect::<Vec<_>>()
        };

        let node_metadata = semantic_block_metadata(chapter, &heading_levels);
        let mut normalised_blocks = retained_blocks
            .into_iter()
            .map(|block| {
                let text = normalise_inline_whitespace(&block.text);
                let metadata = node_metadata.get(&block.node_id.get());
                let original_block_id = block.block_id.clone();
                let fingerprint = BlockFingerprint::from_text(&text);
                let cfi = chapter
                    .node_cfi(block.node_id, spine_index, None)
                    .map(|cfi| cfi.to_cfi_string());
                DraftBlock {
                    block: Block {
                        block_id: make_stable_block_id(&chapter.href, block.order, fingerprint),
                        chapter_index: spine_index,
                        order: block.order,
                        kind: metadata
                            .map(|value| value.kind.clone())
                            .unwrap_or_else(|| block.kind.clone()),
                        text_fingerprint: hex_encode(fingerprint.hash().as_bytes()),
                        text: Arc::from(text),
                        heading_level: metadata.and_then(|value| value.heading_level),
                        merged_from: Vec::new(),
                        footnote_id: metadata.and_then(|value| value.footnote_id.clone()),
                        footnote_refs: Vec::new(),
                        referenced_by: Vec::new(),
                        image: metadata.and_then(|value| value.image.clone()),
                        starts_chapter: false,
                        ends_chapter: false,
                        source_ref: SourceRef {
                            chapter_href: chapter.href.clone(),
                            spine_index,
                            node_id: block.node_id.get(),
                            cfi,
                        },
                    },
                    node_ids: vec![block.node_id],
                    source_block_ids: vec![original_block_id],
                }
            })
            .collect::<Vec<_>>();

        normalised_blocks = merge_paragraph_boundaries(normalised_blocks);
        associate_footnotes(chapter, &mut normalised_blocks);
        if let Some(first) = normalised_blocks.first_mut() {
            first.block.starts_chapter = true;
        }
        if let Some(last) = normalised_blocks.last_mut() {
            last.block.ends_chapter = true;
        }

        let first_block_id = normalised_blocks
            .first()
            .map(|draft| draft.block.block_id.clone());
        let last_block_id = normalised_blocks
            .last()
            .map(|draft| draft.block.block_id.clone());
        let visible_text = normalised_blocks
            .iter()
            .map(|draft| draft.block.text.as_ref())
            .collect::<Vec<_>>()
            .join("\n\n");

        ir_chapters.push(Chapter {
            spine_index,
            href: chapter.href.clone(),
            title: Arc::from(normalise_inline_whitespace(&chapter.title)),
            block_count: u32::try_from(normalised_blocks.len()).unwrap_or(u32::MAX),
            visible_text: Arc::from(visible_text),
            content_hash: hex_encode(chapter.content_hash.as_bytes()),
            first_block_id,
            last_block_id,
            is_noise,
        });

        fn contains(chapter: &PageletChapterIr, root: NodeId, target: NodeId) -> bool {
            root == target
                || chapter.nodes.get(root).is_some_and(|node| {
                    node.children()
                        .iter()
                        .any(|child| contains(chapter, *child, target))
                })
        }
        for anchor in chapter.anchors.anchors.values() {
            if anchor.utf8_byte_offset != 0 {
                continue;
            }
            if let Some(draft) = normalised_blocks.iter().find(|draft| {
                draft
                    .node_ids
                    .first()
                    .is_some_and(|id| contains(chapter, anchor.node_id, *id))
            }) {
                anchor_blocks.insert(anchor.key.to_string(), draft.block.block_id.clone());
            }
        }
        for draft in normalised_blocks {
            ir_blocks.push(draft.block);
        }
    }

    let mut book = BookIr {
        logical_sections: Vec::new(),
        metadata: metadata.clone(),
        toc: normalise_toc(toc),
        spine: spine.to_vec(),
        chapters: ir_chapters,
        blocks: ir_blocks,
    };
    book.logical_sections = crate::sections::build(&book, &anchor_blocks);
    book
}

#[derive(Debug, Clone)]
struct DraftBlock {
    block: Block,
    node_ids: Vec<NodeId>,
    source_block_ids: Vec<String>,
}

fn merge_paragraph_boundaries(blocks: Vec<DraftBlock>) -> Vec<DraftBlock> {
    let mut merged = Vec::<DraftBlock>::new();
    for block in blocks {
        if let Some(previous) = merged.last_mut() {
            if paragraphs_form_one_block(&previous.block, &block.block) {
                merge_paragraph_block(previous, block);
                continue;
            }
        }
        merged.push(block);
    }
    merged
}

fn paragraphs_form_one_block(previous: &Block, next: &Block) -> bool {
    if previous.kind != "paragraph"
        || next.kind != "paragraph"
        || previous.text.chars().count() > 1_200
    {
        return false;
    }

    let previous = previous.text.trim();
    let next = next.text.trim();
    if previous.is_empty() || next.is_empty() {
        return false;
    }

    let terminal = previous
        .chars()
        .next_back()
        .is_some_and(is_hard_paragraph_terminal);
    let next_starts_lowercase = next
        .chars()
        .find(|character| !character.is_whitespace())
        .is_some_and(char::is_lowercase);
    let soft_cjk_boundary = previous
        .chars()
        .next_back()
        .is_some_and(|character| matches!(character, '，' | '、'));

    !terminal && (next_starts_lowercase || soft_cjk_boundary)
}

fn is_hard_paragraph_terminal(character: char) -> bool {
    matches!(
        character,
        '.' | '!' | '?' | ':' | ';' | '。' | '！' | '？' | '：' | '；'
    )
}

fn merge_paragraph_block(previous: &mut DraftBlock, next: DraftBlock) {
    let text = format!("{} {}", previous.block.text.trim(), next.block.text.trim());
    let fingerprint = BlockFingerprint::from_text(&text);
    previous.block.block_id = make_stable_block_id(
        &previous.block.source_ref.chapter_href,
        previous.block.order,
        fingerprint,
    );
    previous.block.text = Arc::from(text);
    previous.block.text_fingerprint = hex_encode(fingerprint.hash().as_bytes());
    previous.node_ids.extend(next.node_ids);
    previous.source_block_ids.extend(next.source_block_ids);
    previous.block.merged_from = previous.source_block_ids.clone();
}

fn associate_footnotes(chapter: &PageletChapterIr, blocks: &mut [DraftBlock]) {
    let mut node_to_block = BTreeMap::new();
    let mut note_to_block = BTreeMap::<Arc<str>, usize>::new();

    for (index, draft) in blocks.iter().enumerate() {
        for node_id in &draft.node_ids {
            node_to_block.insert(node_id.get(), index);
        }
        if let Some(note_id) = draft.block.footnote_id.clone() {
            note_to_block.insert(note_id, index);
        }
    }

    let associations = chapter
        .links
        .iter()
        .filter(|link| link.kind == LinkKind::Footnote)
        .filter_map(|link| {
            let note_id = link.fragment.as_ref()?;
            let source_index = *node_to_block.get(&link.source_node.get())?;
            let note_index = *note_to_block.get(note_id)?;
            Some((source_index, note_index))
        })
        .collect::<Vec<_>>();

    for (source_index, note_index) in associations {
        if source_index == note_index {
            continue;
        }
        let source_id = blocks[source_index].block.block_id.clone();
        let note_id = blocks[note_index].block.block_id.clone();
        if !blocks[source_index].block.footnote_refs.contains(&note_id) {
            blocks[source_index].block.footnote_refs.push(note_id);
        }
        if !blocks[note_index].block.referenced_by.contains(&source_id) {
            blocks[note_index].block.referenced_by.push(source_id);
        }
    }
}

fn normalise_toc(toc: &[TocEntry]) -> Vec<TocEntry> {
    let mut result = Vec::<TocEntry>::new();
    for entry in toc {
        let label = normalise_inline_whitespace(&entry.label);
        let href = entry.href.trim();
        if label.is_empty() || href.is_empty() {
            continue;
        }
        let normalised = TocEntry {
            label: Arc::from(label),
            href: Arc::from(href),
            children: normalise_toc(&entry.children),
        };
        if let Some(existing) = result.iter_mut().find(|candidate| {
            candidate.label.eq_ignore_ascii_case(&normalised.label)
                && candidate.href == normalised.href
        }) {
            let mut children = existing.children.clone();
            children.extend(normalised.children);
            existing.children = normalise_toc(&children);
        } else {
            result.push(normalised);
        }
    }
    result
}

fn normalised_heading_levels(
    chapters: &[PageletChapterIr],
    noise_indices: &BTreeMap<usize, bool>,
) -> BTreeMap<u8, u8> {
    let mut authored = BTreeSet::new();
    for (index, chapter) in chapters.iter().enumerate() {
        if noise_indices.contains_key(&index) {
            continue;
        }
        for (_, node) in chapter.nodes.iter_with_ids() {
            if let DocumentNode::Heading(heading) = node {
                authored.insert(heading.level.clamp(1, 6));
            }
        }
    }
    authored
        .into_iter()
        .enumerate()
        .map(|(index, authored_level)| {
            (authored_level, u8::try_from(index + 1).unwrap_or(6).min(6))
        })
        .collect()
}

fn normalise_inline_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Build a compiled package from a BookIR.
pub fn compile(book_ir: &BookIr, profile: Profile, source_hash: &str) -> CompiledOutput {
    let chapter_summaries: Vec<ChapterSummary> = book_ir
        .chapters
        .iter()
        .map(|ch| ChapterSummary {
            spine_index: ch.spine_index,
            href: ch.href.clone(),
            title: ch.title.clone(),
            block_count: ch.block_count,
            first_block_id: ch.first_block_id.clone(),
            last_block_id: ch.last_block_id.clone(),
            is_noise: ch.is_noise,
        })
        .collect();

    let structure = Structure {
        title: book_ir.metadata.title.clone(),
        spine_count: u32::try_from(book_ir.spine.len()).unwrap_or(u32::MAX),
        chapter_count: chapter_summaries.len() as u32,
        toc: book_ir.toc.clone(),
        spine: book_ir.spine.clone(),
        chapters: chapter_summaries,
    };

    let manifest = Manifest {
        format_version: Arc::from(if book_ir.logical_sections.is_empty() {
            "0.1"
        } else {
            "0.2"
        }),
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

#[derive(Debug, Clone, Eq, PartialEq)]
struct NodeMetadata {
    kind: String,
    heading_level: Option<u8>,
    footnote_id: Option<Arc<str>>,
    image: Option<ImageReference>,
}

fn semantic_block_metadata(
    chapter: &PageletChapterIr,
    heading_levels: &BTreeMap<u8, u8>,
) -> BTreeMap<u32, NodeMetadata> {
    let mut metadata = BTreeMap::new();
    collect_semantic_block_metadata(
        chapter,
        chapter.root,
        SemanticContext::Plain,
        None,
        heading_levels,
        &mut metadata,
    );
    metadata
}

fn collect_semantic_block_metadata(
    chapter: &PageletChapterIr,
    node_id: NodeId,
    context: SemanticContext,
    figure_image: Option<&ImageReference>,
    heading_levels: &BTreeMap<u8, u8>,
    metadata: &mut BTreeMap<u32, NodeMetadata>,
) {
    let Some(node) = chapter.nodes.get(node_id) else {
        return;
    };

    match node {
        DocumentNode::Paragraph(_) => {
            let kind = if figure_image.is_some() {
                "image-caption"
            } else {
                match context {
                    SemanticContext::Plain => "paragraph",
                    SemanticContext::List => "list",
                    SemanticContext::ListItem => "list-item",
                    SemanticContext::BlockQuote => "blockquote",
                }
            };
            metadata.insert(
                node_id.get(),
                NodeMetadata {
                    kind: kind.to_owned(),
                    heading_level: None,
                    footnote_id: None,
                    image: figure_image.cloned(),
                },
            );
        }
        DocumentNode::Heading(heading) => {
            let authored = heading.level.clamp(1, 6);
            let level = heading_levels.get(&authored).copied().unwrap_or(1);
            metadata.insert(
                node_id.get(),
                NodeMetadata {
                    kind: format!("heading-{level}"),
                    heading_level: Some(level),
                    footnote_id: None,
                    image: None,
                },
            );
        }
        DocumentNode::List(_) => {
            visit_semantic_children(
                chapter,
                node,
                SemanticContext::List,
                figure_image,
                heading_levels,
                metadata,
            );
        }
        DocumentNode::ListItem(_) => {
            let next_context = if context == SemanticContext::BlockQuote {
                context
            } else {
                SemanticContext::ListItem
            };
            visit_semantic_children(
                chapter,
                node,
                next_context,
                figure_image,
                heading_levels,
                metadata,
            );
        }
        DocumentNode::BlockQuote(_) => {
            visit_semantic_children(
                chapter,
                node,
                SemanticContext::BlockQuote,
                figure_image,
                heading_levels,
                metadata,
            );
        }
        DocumentNode::Figure(_) => {
            let image = first_descendant_image(chapter, node_id).map(image_reference);
            visit_semantic_children(
                chapter,
                node,
                context,
                image.as_ref(),
                heading_levels,
                metadata,
            );
        }
        DocumentNode::Footnote(note) => {
            metadata.insert(
                node_id.get(),
                NodeMetadata {
                    kind: "footnote".to_owned(),
                    heading_level: None,
                    footnote_id: note.note_id.clone(),
                    image: None,
                },
            );
        }
        _ => visit_semantic_children(
            chapter,
            node,
            context,
            figure_image,
            heading_levels,
            metadata,
        ),
    }
}

fn visit_semantic_children(
    chapter: &PageletChapterIr,
    node: &DocumentNode,
    context: SemanticContext,
    figure_image: Option<&ImageReference>,
    heading_levels: &BTreeMap<u8, u8>,
    metadata: &mut BTreeMap<u32, NodeMetadata>,
) {
    for child in node.children() {
        collect_semantic_block_metadata(
            chapter,
            *child,
            context,
            figure_image,
            heading_levels,
            metadata,
        );
    }
}

fn first_descendant_image(chapter: &PageletChapterIr, node_id: NodeId) -> Option<&ImageNode> {
    let node = chapter.nodes.get(node_id)?;
    if let DocumentNode::Image(image) = node {
        return Some(image);
    }
    node.children()
        .iter()
        .find_map(|child| first_descendant_image(chapter, *child))
}

fn image_reference(image: &ImageNode) -> ImageReference {
    ImageReference {
        src: image.src.clone(),
        resolved_path: image.resolved_path.clone(),
        alt: image.alt.clone(),
        title: image.title.clone(),
    }
}

/// Serialize blocks as JSONL (one JSON object per line).
#[must_use]
pub fn blocks_jsonl(blocks: &[Block]) -> String {
    let mut out = String::new();
    for block in blocks {
        out.push('{');
        push_inline_str(&mut out, "block_id", &block.block_id, true);
        push_inline_u32(&mut out, "chapter_index", block.chapter_index, true);
        push_inline_u32(&mut out, "order", block.order, true);
        push_inline_str(&mut out, "kind", &block.kind, true);
        push_inline_str(&mut out, "text", &block.text, true);
        push_inline_str(&mut out, "text_fingerprint", &block.text_fingerprint, true);
        push_inline_u8_opt(&mut out, "heading_level", block.heading_level, true);
        push_inline_string_array(&mut out, "merged_from", &block.merged_from, true);
        push_inline_str_opt(&mut out, "footnote_id", block.footnote_id.as_deref(), true);
        push_inline_string_array(&mut out, "footnote_refs", &block.footnote_refs, true);
        push_inline_string_array(&mut out, "referenced_by", &block.referenced_by, true);
        out.push_str("\"image\": ");
        push_image(&mut out, block.image.as_ref());
        out.push_str(", ");
        push_inline_bool(&mut out, "starts_chapter", block.starts_chapter, true);
        push_inline_bool(&mut out, "ends_chapter", block.ends_chapter, true);
        out.push_str("\"source_ref\": {");
        push_inline_str(
            &mut out,
            "chapter_href",
            &block.source_ref.chapter_href,
            true,
        );
        push_inline_u32(&mut out, "spine_index", block.source_ref.spine_index, true);
        push_inline_u32(&mut out, "node_id", block.source_ref.node_id, true);
        push_inline_str_opt(&mut out, "cfi", block.source_ref.cfi.as_deref(), false);
        out.push('}');
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
    out.push_str("\"toc\": ");
    push_toc(&mut out, &structure.toc);
    out.push_str(",\n");
    indent(&mut out, 1);
    out.push_str("\"spine\": [\n");
    for (index, entry) in structure.spine.iter().enumerate() {
        indent(&mut out, 2);
        out.push('{');
        push_inline_u32(&mut out, "spine_index", entry.spine_index, true);
        push_inline_str(&mut out, "idref", &entry.idref, true);
        push_inline_str_opt(&mut out, "href", entry.href.as_deref(), true);
        push_inline_bool(&mut out, "linear", entry.linear, false);
        out.push('}');
        if index + 1 < structure.spine.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(&mut out, 1);
    out.push_str("],\n");
    indent(&mut out, 1);
    out.push_str("\"chapters\": [\n");
    for (index, ch) in structure.chapters.iter().enumerate() {
        indent(&mut out, 2);
        out.push('{');
        push_inline_u32(&mut out, "spine_index", ch.spine_index, true);
        push_inline_str(&mut out, "href", &ch.href, true);
        push_inline_str(&mut out, "title", &ch.title, true);
        push_inline_u32(&mut out, "block_count", ch.block_count, true);
        push_inline_str_opt(
            &mut out,
            "first_block_id",
            ch.first_block_id.as_deref(),
            true,
        );
        push_inline_str_opt(&mut out, "last_block_id", ch.last_block_id.as_deref(), true);
        push_inline_bool(&mut out, "is_noise", ch.is_noise, false);
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

fn push_inline_str(out: &mut String, name: &str, value: &str, trailing: bool) {
    out.push('"');
    out.push_str(name);
    out.push_str("\": \"");
    out.push_str(&escape_json_str(value));
    out.push('"');
    if trailing {
        out.push_str(", ");
    }
}

fn push_inline_str_opt(out: &mut String, name: &str, value: Option<&str>, trailing: bool) {
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

fn push_inline_u32(out: &mut String, name: &str, value: u32, trailing: bool) {
    out.push('"');
    out.push_str(name);
    out.push_str("\": ");
    out.push_str(&value.to_string());
    if trailing {
        out.push_str(", ");
    }
}

fn push_inline_u8_opt(out: &mut String, name: &str, value: Option<u8>, trailing: bool) {
    out.push('"');
    out.push_str(name);
    out.push_str("\": ");
    if let Some(value) = value {
        out.push_str(&value.to_string());
    } else {
        out.push_str("null");
    }
    if trailing {
        out.push_str(", ");
    }
}

fn push_inline_bool(out: &mut String, name: &str, value: bool, trailing: bool) {
    out.push('"');
    out.push_str(name);
    out.push_str("\": ");
    out.push_str(if value { "true" } else { "false" });
    if trailing {
        out.push_str(", ");
    }
}

fn push_inline_string_array(out: &mut String, name: &str, values: &[String], trailing: bool) {
    out.push('"');
    out.push_str(name);
    out.push_str("\": [");
    for (index, value) in values.iter().enumerate() {
        out.push('"');
        out.push_str(&escape_json_str(value));
        out.push('"');
        if index + 1 < values.len() {
            out.push_str(", ");
        }
    }
    out.push(']');
    if trailing {
        out.push_str(", ");
    }
}

fn push_image(out: &mut String, image: Option<&ImageReference>) {
    let Some(image) = image else {
        out.push_str("null");
        return;
    };
    out.push('{');
    push_inline_str(out, "src", &image.src, true);
    push_inline_str_opt(out, "resolved_path", image.resolved_path.as_deref(), true);
    push_inline_str(out, "alt", &image.alt, true);
    push_inline_str_opt(out, "title", image.title.as_deref(), false);
    out.push('}');
}

fn push_toc(out: &mut String, toc: &[TocEntry]) {
    out.push('[');
    for (index, entry) in toc.iter().enumerate() {
        out.push('{');
        push_inline_str(out, "label", &entry.label, true);
        push_inline_str(out, "href", &entry.href, true);
        out.push_str("\"children\": ");
        push_toc(out, &entry.children);
        out.push('}');
        if index + 1 < toc.len() {
            out.push_str(", ");
        }
    }
    out.push(']');
}

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
