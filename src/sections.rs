//! Logical reading units overlay stable physical blocks.
use crate::ir::{BookIr, Chapter, TocEntry};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SectionRange {
    pub spine_index: u32,
    pub block_ids: Vec<String>,
}
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct LogicalSection {
    pub chapter_id: String,
    pub title: String,
    pub ranges: Vec<SectionRange>,
}

pub(crate) fn build(book: &BookIr, anchors: &BTreeMap<String, String>) -> Vec<LogicalSection> {
    fn leaves<'a>(toc: &'a [TocEntry], items: &mut Vec<&'a TocEntry>) {
        for entry in toc {
            if entry.children.is_empty() {
                items.push(entry)
            } else {
                leaves(&entry.children, items)
            }
        }
    }
    let mut entries = Vec::new();
    leaves(&book.toc, &mut entries);
    let mut starts = Vec::new();
    for entry in entries {
        let (path, fragment) = entry
            .href
            .split_once('#')
            .map_or((entry.href.as_ref(), None), |(p, f)| (p, Some(f)));
        let matches = book
            .chapters
            .iter()
            .filter(|c| c.href.as_ref() == path || c.href.ends_with(&format!("/{path}")))
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Vec::new();
        }
        let chapter = matches[0];
        if chapter.is_noise {
            continue;
        }
        let block = if let Some(fragment) = fragment {
            anchors.get(&format!("{}#{fragment}", chapter.href))
        } else {
            chapter.first_block_id.as_ref()
        };
        let Some(position) =
            block.and_then(|id| book.blocks.iter().position(|b| &b.block_id == id))
        else {
            return Vec::new();
        };
        if starts.last().is_some_and(|(prior, _)| *prior >= position) {
            return Vec::new();
        }
        starts.push((position, entry.label.to_string()));
    }
    if starts.is_empty() {
        return Vec::new();
    }
    if starts[0].0 > 0 {
        starts.insert(0, (0, "Front matter".into()));
    }
    let physical_starts = book
        .chapters
        .iter()
        .filter_map(|c| c.first_block_id.as_ref())
        .collect::<Vec<_>>();
    if starts.len() == physical_starts.len()
        && starts
            .iter()
            .zip(physical_starts)
            .all(|((p, _), id)| &book.blocks[*p].block_id == id)
    {
        return Vec::new();
    }
    starts
        .iter()
        .enumerate()
        .map(|(index, (start, title))| {
            let end = starts.get(index + 1).map_or(book.blocks.len(), |(p, _)| *p);
            let mut ranges: Vec<SectionRange> = Vec::new();
            for block in &book.blocks[*start..end] {
                if ranges
                    .last()
                    .is_none_or(|r| r.spine_index != block.chapter_index)
                {
                    ranges.push(SectionRange {
                        spine_index: block.chapter_index,
                        block_ids: Vec::new(),
                    });
                }
                ranges
                    .last_mut()
                    .unwrap()
                    .block_ids
                    .push(block.block_id.clone());
            }
            LogicalSection {
                chapter_id: format!("chapter_{:03}", index + 1),
                title: title.clone(),
                ranges,
            }
        })
        .collect()
}

pub fn block_sections(
    sections: &[LogicalSection],
    blocks: &[(String, u32)],
) -> Result<BTreeMap<String, u32>, String> {
    let physical = blocks.iter().cloned().collect::<BTreeMap<_, _>>();
    let mut result = BTreeMap::new();
    let mut ordered = Vec::new();
    for (index, section) in sections.iter().enumerate() {
        if section.chapter_id != format!("chapter_{:03}", index + 1)
            || section.title.trim().is_empty()
            || section.ranges.is_empty()
        {
            return Err("invalid logical section metadata".into());
        }
        for range in &section.ranges {
            if range.block_ids.is_empty() {
                return Err("empty logical section range".into());
            }
            for id in &range.block_ids {
                if physical.get(id) != Some(&range.spine_index)
                    || result.insert(id.clone(), index as u32).is_some()
                {
                    return Err("invalid or overlapping logical section block".into());
                }
                ordered.push(id);
            }
        }
    }
    if !sections.is_empty()
        && (ordered.len() != blocks.len()
            || ordered
                .iter()
                .zip(blocks)
                .any(|(id, (expected, _))| *id != expected))
    {
        return Err("logical sections must partition physical blocks in reading order".into());
    }
    Ok(result)
}

pub fn analysis_view(book: &BookIr) -> BookIr {
    if book.logical_sections.is_empty() {
        return book.clone();
    }
    let mut view = book.clone();
    view.chapters.clear();
    view.blocks.clear();
    let blocks = book
        .blocks
        .iter()
        .map(|b| (&b.block_id, b))
        .collect::<BTreeMap<_, _>>();
    for (index, section) in book.logical_sections.iter().enumerate() {
        let selected = section
            .ranges
            .iter()
            .flat_map(|r| &r.block_ids)
            .map(|id| blocks[id])
            .collect::<Vec<_>>();
        let text = selected
            .iter()
            .map(|b| b.text.as_ref())
            .collect::<Vec<_>>()
            .join("\n\n");
        view.chapters.push(Chapter {
            spine_index: index as u32,
            href: selected[0].source_ref.chapter_href.clone(),
            title: Arc::from(section.title.as_str()),
            block_count: selected.len() as u32,
            visible_text: Arc::from(text.as_str()),
            content_hash: format!(
                "{:?}",
                pagelet::core::ContentHash::from_bytes(text.as_bytes())
            ),
            first_block_id: selected.first().map(|b| b.block_id.clone()),
            last_block_id: selected.last().map(|b| b.block_id.clone()),
            is_noise: false,
        });
        for (order, block) in selected.into_iter().enumerate() {
            let mut block = block.clone();
            block.chapter_index = index as u32;
            block.order = order as u32;
            view.blocks.push(block);
        }
    }
    view
}

pub(crate) fn json_analysis_view(ir: serde_json::Value) -> Result<serde_json::Value, String> {
    let sections: Vec<LogicalSection> = serde_json::from_value(
        ir.get("logical_sections")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
    )
    .map_err(|e| e.to_string())?;
    if sections.is_empty() {
        return Ok(ir);
    }
    let source = ir
        .get("blocks")
        .and_then(serde_json::Value::as_array)
        .ok_or("blocks missing")?;
    let physical = source
        .iter()
        .map(|b| {
            Ok((
                b["block_id"].as_str().ok_or("block id missing")?.to_owned(),
                u32::try_from(b["chapter_index"].as_u64().ok_or("chapter index missing")?)
                    .map_err(|_| "invalid chapter index")?,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mapping = block_sections(&sections, &physical)?;
    let mut result = ir.clone();
    let mut chapters = Vec::new();
    let mut blocks = Vec::new();
    for (index, section) in sections.iter().enumerate() {
        let selected = source
            .iter()
            .filter(|b| mapping.get(b["block_id"].as_str().unwrap()) == Some(&(index as u32)))
            .collect::<Vec<_>>();
        chapters.push(serde_json::json!({"spine_index":index,"title":section.title,"is_noise":false,"href":selected[0]["source_ref"]["chapter_href"],"block_count":selected.len(),"first_block_id":selected[0]["block_id"],"last_block_id":selected.last().unwrap()["block_id"]}));
        for (order, block) in selected.into_iter().enumerate() {
            let mut block = block.clone();
            block["chapter_index"] = serde_json::json!(index);
            block["order"] = serde_json::json!(order);
            blocks.push(block);
        }
    }
    result["chapters"] = serde_json::json!(chapters);
    result["blocks"] = serde_json::json!(blocks);
    Ok(result)
}
