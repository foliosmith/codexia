//! Book IR Normalizer — transforms pagelet ChapterIR into codexia BookIR
//!
//! Pipeline:
//!   EPUB bytes → pagelet open_book + spine chapters
//!              → normalise_chapters → BookIR
//!              → serialise → JSON / JSONL / structure.json

use std::{collections::BTreeMap, sync::Arc};

use pagelet::{
    document::ChapterIr as PageletChapterIr,
    epub::filter_noise_chapters,
};

use crate::ir::{Block, BookIr, Chapter, ChapterSummary, CompiledOutput, Manifest, Metadata, Profile, SourceRef, Structure, TocEntry};

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
    metadata: &Metadata,
    options: &NormaliserOptions,
) -> BookIr {
    let spine_len = chapters.len();
    let mut noise_indices = BTreeMap::new();

    if options.filter_noise {
        let chapter_infos: Vec<(usize, String, &str)> = chapters
            .iter()
            .enumerate()
            .map(|(i, ch)| (i, ch.visible_text(), ch.title.as_ref()))
            .collect();
        let refs: Vec<(usize, &str, &str)> = chapter_infos
            .iter()
            .map(|(i, text, title)| (*i, &**title, text.as_str()))
            .collect();
        let filtered = filter_noise_chapters(&refs, spine_len);
        for (i, _ch) in chapters.iter().enumerate() {
            if !filtered.contains(&i) {
                noise_indices.insert(i, true);
            }
        }
    }

    let mut ir_chapters = Vec::new();
    let mut ir_blocks = Vec::new();

    for (spine_index, chapter) in chapters.iter().enumerate() {
        let spine_index = spine_index as u32;
        let is_noise = noise_indices.contains_key(&(spine_index as usize));

        let visible_text = chapter.visible_text();
        let blocks = chapter.blocks();
        let block_count = blocks.len() as u32;

        ir_chapters.push(Chapter {
            spine_index,
            href: chapter.href.clone(),
            title: chapter.title.clone(),
            block_count,
            visible_text: Arc::from(visible_text),
            content_hash: hex_encode(chapter.content_hash.as_bytes()),
        });

        if !is_noise {
            for block in blocks {
                let cfi = chapter
                    .node_cfi(block.node_id, spine_index, None)
                    .map(|cfi| cfi.to_cfi_string());
                ir_blocks.push(Block {
                    block_id: block.block_id.clone(),
                    chapter_index: spine_index,
                    order: block.order,
                    kind: block.kind.clone(),
                    text: Arc::from(block.text),
                    source_ref: SourceRef {
                        chapter_href: chapter.href.clone(),
                        spine_index,
                        node_id: block.node_id.get(),
                        cfi,
                    },
                });
            }
        }
    }

    BookIr {
        metadata: metadata.clone(),
        toc: toc.to_vec(),
        chapters: ir_chapters,
        blocks: ir_blocks,
    }
}

/// Build a compiled package from a BookIR.
pub fn compile(
    book_ir: &BookIr,
    profile: Profile,
    source_hash: &str,
) -> CompiledOutput {
    let chapter_summaries: Vec<ChapterSummary> = book_ir
        .chapters
        .iter()
        .map(|ch| ChapterSummary {
            spine_index: ch.spine_index,
            title: ch.title.clone(),
            block_count: ch.block_count,
            is_noise: false,
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

/// Serialize blocks as JSONL (one JSON object per line).
#[must_use]
pub fn blocks_jsonl(blocks: &[Block]) -> String {
    let mut out = String::new();
    for block in blocks {
        out.push('{');
        out.push_str(&format!("\"block_id\": \"{}\", ", escape_json_str(&block.block_id)));
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
    push_field_str(&mut out, 1, "format_version", &manifest.format_version, true);
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
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
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
        let ir = normalise(&[], &[], &metadata, &NormaliserOptions::default());
        assert_eq!(ir.chapters.len(), 0);
        assert_eq!(ir.blocks.len(), 0);
    }
}
