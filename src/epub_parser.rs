//! EPUB parser adapter — converts pagelet package and chapter IR into Codexia BookIR.

use std::sync::Arc;

use pagelet::{
    document::ChapterIr as PageletChapterIr,
    engine::{BookSession, Engine},
    epub::{NavigationItem, OpenOptions},
};

use crate::{
    ir::{BookIr, Metadata, SpineEntry, TocEntry},
    normalizer::{self, NormaliserOptions},
};

/// Parse EPUB bytes into normalized Codexia BookIR.
pub fn parse_epub(bytes: &[u8]) -> Result<BookIr, String> {
    parse_epub_with_options(bytes, OpenOptions::default(), &NormaliserOptions::default())
}

/// Parse EPUB bytes with explicit pagelet and normalizer options.
pub fn parse_epub_with_options(
    bytes: &[u8],
    open_options: OpenOptions,
    normaliser_options: &NormaliserOptions,
) -> Result<BookIr, String> {
    let session = Engine::builder()
        .compatibility(open_options.compatibility_mode)
        .limits(open_options.limits)
        .build()
        .open_bytes(bytes.to_vec())
        .map_err(|error| error.to_string())?;
    parse_book_session_with_options(&session, normaliser_options)
}

/// Compile an already-open pagelet book session into normalized Codexia BookIR.
pub fn parse_book_session(session: &BookSession) -> Result<BookIr, String> {
    parse_book_session_with_options(session, &NormaliserOptions::default())
}

/// Compile an existing pagelet session with explicit normalizer options.
pub fn parse_book_session_with_options(
    session: &BookSession,
    normaliser_options: &NormaliserOptions,
) -> Result<BookIr, String> {
    let package = session.package();

    let metadata = Metadata {
        title: package.metadata.title.as_deref().map(Arc::from),
        identifier: package.metadata.identifier.as_deref().map(Arc::from),
        language: package.metadata.language.as_deref().map(Arc::from),
        package_version: Arc::from(package.metadata.package_version.as_ref()),
    };

    let toc = epub3_nested_toc(session).unwrap_or_else(|| {
        session
            .navigation()
            .toc
            .iter()
            .map(map_toc_entry)
            .collect::<Vec<_>>()
    });

    let spine = package
        .spine
        .iter()
        .enumerate()
        .map(|(spine_index, item)| {
            let href = package
                .manifest
                .iter()
                .find(|manifest_item| manifest_item.id == item.idref)
                .map(|manifest_item| Arc::from(manifest_item.resolved_path.as_ref()));
            SpineEntry {
                spine_index: u32::try_from(spine_index).unwrap_or(u32::MAX),
                idref: Arc::from(item.idref.as_ref()),
                href,
                linear: item.linear,
            }
        })
        .collect::<Vec<_>>();

    let mut chapters = Vec::with_capacity(package.spine.len());
    for (spine_index, spine_item) in package.spine.iter().enumerate() {
        let manifest_item = package
            .manifest
            .iter()
            .find(|item| item.id == spine_item.idref);
        if let Some(manifest_item) = manifest_item {
            let is_xhtml = matches!(
                manifest_item.media_type.as_ref(),
                "application/xhtml+xml" | "text/html"
            );
            if !is_xhtml {
                chapters.push(empty_chapter(
                    spine_index,
                    &manifest_item.resolved_path,
                    &spine_item.idref,
                ));
                continue;
            }
        }

        match session.open_spine_item(spine_index) {
            Ok(chapter) => chapters.push((*chapter).clone()),
            Err(error) => return Err(format!("spine item {spine_index} failed to parse: {error}")),
        }
    }

    Ok(normalizer::normalise(
        &chapters,
        &toc,
        &spine,
        &metadata,
        normaliser_options,
    ))
}

fn map_toc_entry(item: &NavigationItem) -> TocEntry {
    TocEntry {
        label: Arc::from(item.label.as_str()),
        href: Arc::from(item.href.as_str()),
        children: item.children.iter().map(map_toc_entry).collect(),
    }
}

fn epub3_nested_toc(session: &BookSession) -> Option<Vec<TocEntry>> {
    let nav_item = session.package().manifest.iter().find(|item| {
        item.properties
            .iter()
            .any(|property| property.split_whitespace().any(|value| value == "nav"))
    })?;
    let payload = session.read_resource(nav_item.resource_id?).ok()?;
    let document = String::from_utf8(payload.bytes).ok()?;
    let toc = parse_epub3_toc(&document);
    (!toc.is_empty()).then_some(toc)
}

#[derive(Debug, Default)]
struct PendingTocEntry {
    label: String,
    href: String,
    children: Vec<TocEntry>,
}

fn parse_epub3_toc(document: &str) -> Vec<TocEntry> {
    let Some(nav) = toc_nav_section(document) else {
        return Vec::new();
    };
    let mut roots = Vec::new();
    let mut stack = Vec::<PendingTocEntry>::new();
    let mut cursor = 0;

    while let Some(tag_start_offset) = nav[cursor..].find('<') {
        let tag_start = cursor + tag_start_offset;
        let Some(tag_end_offset) = nav[tag_start..].find('>') else {
            break;
        };
        let tag_end = tag_start + tag_end_offset;
        let raw_tag = nav[tag_start + 1..tag_end].trim();
        let closing = raw_tag.starts_with('/');
        let tag_name = raw_tag
            .trim_start_matches('/')
            .split_ascii_whitespace()
            .next()
            .unwrap_or_default()
            .trim_end_matches('/');
        cursor = tag_end + 1;

        match (closing, tag_name) {
            (false, "li") => stack.push(PendingTocEntry::default()),
            (false, "a") => {
                let href = attribute_value(raw_tag, "href").unwrap_or_default();
                let Some(anchor_end_offset) = nav[cursor..].find("</a>") else {
                    continue;
                };
                let anchor_end = cursor + anchor_end_offset;
                let label = decode_xml_text(&strip_markup(&nav[cursor..anchor_end]));
                if let Some(entry) = stack.last_mut() {
                    entry.href = href;
                    entry.label = label;
                }
                cursor = anchor_end + "</a>".len();
            }
            (true, "li") => finish_toc_entry(&mut stack, &mut roots),
            _ => {}
        }
    }

    while !stack.is_empty() {
        finish_toc_entry(&mut stack, &mut roots);
    }
    roots
}

fn toc_nav_section(document: &str) -> Option<&str> {
    let mut cursor = 0;
    let mut fallback = None;
    while let Some(nav_start_offset) = document[cursor..].find("<nav") {
        let nav_start = cursor + nav_start_offset;
        let tag_end = nav_start + document[nav_start..].find('>')?;
        let section_end = tag_end + document[tag_end..].find("</nav>")?;
        let section = &document[nav_start..section_end];
        fallback.get_or_insert(section);

        let opening_tag = &document[nav_start + 1..tag_end];
        let nav_type = attribute_value(opening_tag, "epub:type")
            .or_else(|| attribute_value(opening_tag, "type"));
        if nav_type.as_deref().is_none_or(|value| value == "toc") {
            return Some(section);
        }
        cursor = section_end + "</nav>".len();
    }
    fallback
}

fn finish_toc_entry(stack: &mut Vec<PendingTocEntry>, roots: &mut Vec<TocEntry>) {
    let Some(entry) = stack.pop() else {
        return;
    };
    if entry.label.trim().is_empty() || entry.href.trim().is_empty() {
        return;
    }
    let entry = TocEntry {
        label: Arc::from(entry.label),
        href: Arc::from(entry.href),
        children: entry.children,
    };
    if let Some(parent) = stack.last_mut() {
        parent.children.push(entry);
    } else {
        roots.push(entry);
    }
}

fn attribute_value(tag: &str, name: &str) -> Option<String> {
    let marker = format!("{name}=");
    let start = tag.find(&marker)? + marker.len();
    let quote = tag.as_bytes().get(start).copied()?;
    if !matches!(quote, b'\'' | b'"') {
        return None;
    }
    let value_start = start + 1;
    let value_end = tag[value_start..].find(char::from(quote))? + value_start;
    Some(decode_xml_text(&tag[value_start..value_end]))
}

fn strip_markup(value: &str) -> String {
    let mut out = String::new();
    let mut inside_tag = false;
    for character in value.chars() {
        match character {
            '<' => inside_tag = true,
            '>' => inside_tag = false,
            _ if !inside_tag => out.push(character),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_xml_text(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

fn empty_chapter(spine_index: usize, href: &str, title: &str) -> PageletChapterIr {
    PageletChapterIr::empty(
        pagelet::core::DocumentId::new(u32::try_from(spine_index).unwrap_or(u32::MAX)),
        href,
        title,
        pagelet::core::ContentHash::from_bytes(&[]),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::ir::book_ir_to_json;

    #[test]
    fn parses_structure_semantics_fingerprints_cfi_and_noise() {
        let bytes = epub_fixture("Plain paragraph.");
        let first = parse_epub(&bytes).expect("parse fixture");
        let session = pagelet::engine::Engine::new()
            .open_bytes(bytes)
            .expect("open shared session");
        let second = parse_book_session(&session).expect("parse shared session");

        assert_eq!(first.metadata.title.as_deref(), Some("Codexia Fixture"));
        assert_eq!(first.spine.len(), 5);
        assert_eq!(first.spine[2].idref.as_ref(), "chapter-1");
        assert_eq!(first.spine[2].href.as_deref(), Some("EPUB/chapter-1.xhtml"));

        assert_eq!(first.toc.len(), 1);
        assert_eq!(first.toc[0].label.as_ref(), "Part One");
        assert_eq!(first.toc[0].children.len(), 1);
        assert_eq!(first.toc[0].children[0].label.as_ref(), "Chapter One");
        assert_eq!(first.toc[0].children[0].children.len(), 1);
        assert_eq!(
            first.toc[0].children[0].children[0].label.as_ref(),
            "Chapter Two"
        );

        let noise_indices = first
            .chapters
            .iter()
            .filter(|chapter| chapter.is_noise)
            .map(|chapter| chapter.spine_index)
            .collect::<Vec<_>>();
        assert_eq!(noise_indices, vec![0, 1, 4]);
        assert!(first
            .chapters
            .iter()
            .filter(|chapter| chapter.is_noise)
            .all(|chapter| chapter.block_count == 0 && chapter.visible_text.is_empty()));
        assert!(first
            .chapters
            .iter()
            .all(|chapter| !chapter.visible_text.contains("Running Header")));
        assert!(first
            .chapters
            .iter()
            .all(|chapter| !chapter.visible_text.contains("Running Footer")));

        let texts = first
            .blocks
            .iter()
            .map(|block| block.text.as_ref())
            .collect::<Vec<_>>();
        assert!(!texts
            .iter()
            .any(|text| text.contains("All rights reserved")));
        assert!(!texts.iter().any(|text| text.contains("Buy the sequel")));
        assert!(!texts.iter().any(|text| text.contains("Running Header")));
        assert!(!texts.iter().any(|text| text.contains("Running Footer")));

        let kinds = first
            .blocks
            .iter()
            .map(|block| block.kind.as_str())
            .collect::<BTreeSet<_>>();
        assert!(kinds.contains("heading-1"));
        assert!(kinds.contains("heading-2"));
        assert!(kinds.contains("paragraph"));
        assert!(kinds.contains("blockquote"));
        assert!(kinds.contains("list-item"));
        assert!(kinds.contains("footnote"));
        assert!(kinds.contains("image-caption"));

        let chapter_one_heading = first
            .blocks
            .iter()
            .find(|block| block.text.as_ref() == "Chapter One")
            .expect("chapter one heading");
        assert_eq!(chapter_one_heading.heading_level, Some(1));
        let deep_heading = first
            .blocks
            .iter()
            .find(|block| block.text.as_ref() == "Deep Section")
            .expect("deep heading");
        assert_eq!(deep_heading.heading_level, Some(2));

        let merged = first
            .blocks
            .iter()
            .find(|block| {
                block.text.as_ref() == "Broken paragraph boundary continues with lower-case text."
            })
            .expect("merged paragraph");
        assert_eq!(merged.merged_from.len(), 2);
        assert!(first
            .blocks
            .iter()
            .all(|block| !block.text.trim().is_empty()));

        let footnote = first
            .blocks
            .iter()
            .find(|block| block.footnote_id.as_deref() == Some("note-1"))
            .expect("footnote block");
        assert_eq!(footnote.referenced_by.len(), 1);
        let footnote_source = first
            .blocks
            .iter()
            .find(|block| block.footnote_refs.contains(&footnote.block_id))
            .expect("footnote source block");
        assert_eq!(
            footnote.referenced_by,
            vec![footnote_source.block_id.clone()]
        );

        let caption = first
            .blocks
            .iter()
            .find(|block| block.kind == "image-caption")
            .expect("image caption");
        assert_eq!(caption.text.as_ref(), "Figure caption.");
        let image = caption.image.as_ref().expect("caption image metadata");
        assert_eq!(image.src.as_ref(), "figure.jpg");
        assert_eq!(image.resolved_path.as_deref(), Some("EPUB/figure.jpg"));
        assert_eq!(image.alt.as_ref(), "Diagram alt");

        for chapter in first.chapters.iter().filter(|chapter| !chapter.is_noise) {
            let first_id = chapter.first_block_id.as_ref().expect("chapter start");
            let last_id = chapter.last_block_id.as_ref().expect("chapter end");
            assert!(first
                .blocks
                .iter()
                .any(|block| block.block_id == *first_id && block.starts_chapter));
            assert!(first
                .blocks
                .iter()
                .any(|block| block.block_id == *last_id && block.ends_chapter));
        }

        for chapter in &first.chapters {
            assert_eq!(chapter.content_hash.len(), 64);
        }
        for block in &first.blocks {
            assert_eq!(block.block_id.len(), 32);
            assert_eq!(block.text_fingerprint.len(), 64);
            assert!(block
                .text_fingerprint
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()));
            assert!(block
                .source_ref
                .cfi
                .as_deref()
                .is_some_and(|cfi| cfi.starts_with("epubcfi(")));
        }

        let first_identities = first
            .blocks
            .iter()
            .map(|block| (&block.block_id, &block.text_fingerprint))
            .collect::<Vec<_>>();
        let second_identities = second
            .blocks
            .iter()
            .map(|block| (&block.block_id, &block.text_fingerprint))
            .collect::<Vec<_>>();
        assert_eq!(first_identities, second_identities);

        let json = book_ir_to_json(&first);
        assert!(json.contains(r#""spine": ["#));
        assert!(json.contains(r#""text_fingerprint": "#));
        assert!(json.contains(r#""is_noise": true"#));
        assert!(json.contains(r#""kind": "blockquote""#));
        assert!(json.contains(r#""cfi": "epubcfi("#));
        assert!(json.contains(r#""heading_level": 2"#));
        assert!(json.contains(r#""footnote_refs": ["#));
        assert!(json.contains(r#""starts_chapter": true"#));

        let compiled = normalizer::compile(&first, crate::ir::Profile::Standard, "source");
        let blocks_jsonl = normalizer::blocks_jsonl(&first.blocks);
        let structure_json = normalizer::structure_json(&compiled.structure);
        assert_eq!(blocks_jsonl.lines().count(), first.blocks.len());
        assert!(blocks_jsonl
            .lines()
            .all(|line| line.starts_with('{') && line.ends_with('}')));
        assert!(blocks_jsonl.contains(r#""text_fingerprint": "#));
        assert!(blocks_jsonl.contains(r#""image": {"src": "figure.jpg""#));
        assert!(structure_json.contains(r#""toc": ["#));
        assert!(structure_json.contains(r#""spine": ["#));
        assert!(structure_json.contains(r#""first_block_id": "#));
    }

    #[test]
    fn block_identity_changes_when_text_changes() {
        let original =
            parse_epub(&epub_fixture("Plain paragraph.")).expect("parse original fixture");
        let changed =
            parse_epub(&epub_fixture("Changed paragraph.")).expect("parse changed fixture");

        let original_block = original
            .blocks
            .iter()
            .find(|block| block.text.as_ref() == "Plain paragraph.")
            .expect("original paragraph");
        let changed_block = changed
            .blocks
            .iter()
            .find(|block| block.text.as_ref() == "Changed paragraph.")
            .expect("changed paragraph");

        assert_ne!(
            original_block.text_fingerprint,
            changed_block.text_fingerprint
        );
        assert_ne!(original_block.block_id, changed_block.block_id);
    }

    struct TestEntry {
        path: &'static str,
        bytes: Vec<u8>,
    }

    fn epub_fixture(paragraph: &str) -> Vec<u8> {
        let package = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="bookid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="bookid">urn:codexia:fixture</dc:identifier>
    <dc:title>Codexia Fixture</dc:title>
    <dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="copyright" href="copyright.xhtml" media-type="application/xhtml+xml"/>
    <item id="contents" href="contents.xhtml" media-type="application/xhtml+xml"/>
    <item id="chapter-1" href="chapter-1.xhtml" media-type="application/xhtml+xml"/>
    <item id="chapter-2" href="chapter-2.xhtml" media-type="application/xhtml+xml"/>
    <item id="figure" href="figure.jpg" media-type="image/jpeg"/>
    <item id="advertisement" href="advertisement.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="copyright"/>
    <itemref idref="contents"/>
    <itemref idref="chapter-1"/>
    <itemref idref="chapter-2"/>
    <itemref idref="advertisement"/>
  </spine>
</package>"#;
        let nav = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <head><title>Contents</title></head>
  <body><nav epub:type="page-list"><ol><li><a href="chapter-1.xhtml">1</a></li></ol></nav>
  <nav epub:type="toc"><ol>
    <li><a href="chapter-1.xhtml">Part One</a><ol>
      <li><a href="chapter-1.xhtml">Chapter One</a><ol>
        <li><a href="chapter-2.xhtml">Chapter Two</a></li>
      </ol></li>
    </ol></li>
    <li><a href="chapter-1.xhtml">Part One</a></li>
  </ol></nav></body>
</html>"#;
        let copyright = xhtml(
            "Copyright",
            "<p>Copyright © 2026 Codexia. All rights reserved.</p>",
        );
        let contents = xhtml("Table of Contents", "<p>Chapter One</p><p>Chapter Two</p>");
        let chapter_one = xhtml(
            "Chapter One",
            &format!(
                r##"<p>Codexia Running Header</p>
<h2>Chapter One</h2>
<p>{paragraph}</p>
<p>Broken paragraph boundary</p>
<p>continues with lower-case text.</p>
<p>   </p>
<blockquote><p>Quoted argument.</p></blockquote>
<ul><li><p>First list item.</p></li></ul>
<figure><img src="figure.jpg" alt="Diagram alt" title="Diagram title"/><figcaption>Figure caption.</figcaption></figure>
<p>See <a epub:type="noteref" href="#note-1">note</a>.</p>
<aside epub:type="footnote" id="note-1"><p>Footnote detail.</p></aside>
<p>Codexia Running Footer</p>"##
            ),
        );
        let chapter_two = xhtml(
            "Chapter Two",
            "<p>Codexia Running Header</p><h2>Chapter Two</h2><h4>Deep Section</h4><p>Second chapter.</p><p>Codexia Running Footer</p>",
        );
        let advertisement = xhtml("Advertisement", "<p>Buy the sequel today.</p>");

        write_stored_zip(&[
            TestEntry {
                path: "mimetype",
                bytes: b"application/epub+zip".to_vec(),
            },
            TestEntry {
                path: "META-INF/container.xml",
                bytes: br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="EPUB/package.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#.to_vec(),
            },
            TestEntry {
                path: "EPUB/package.opf",
                bytes: package.as_bytes().to_vec(),
            },
            TestEntry {
                path: "EPUB/nav.xhtml",
                bytes: nav.as_bytes().to_vec(),
            },
            TestEntry {
                path: "EPUB/copyright.xhtml",
                bytes: copyright.into_bytes(),
            },
            TestEntry {
                path: "EPUB/contents.xhtml",
                bytes: contents.into_bytes(),
            },
            TestEntry {
                path: "EPUB/chapter-1.xhtml",
                bytes: chapter_one.into_bytes(),
            },
            TestEntry {
                path: "EPUB/chapter-2.xhtml",
                bytes: chapter_two.into_bytes(),
            },
            TestEntry {
                path: "EPUB/figure.jpg",
                bytes: vec![0xff, 0xd8, 0xff, 0xd9],
            },
            TestEntry {
                path: "EPUB/advertisement.xhtml",
                bytes: advertisement.into_bytes(),
            },
        ])
    }

    fn xhtml(title: &str, body: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>{title}</title></head><body>{body}</body></html>"#
        )
    }

    fn write_stored_zip(entries: &[TestEntry]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();

        for entry in entries {
            let offset = u32::try_from(out.len()).expect("fixture offset");
            let name = entry.path.as_bytes();
            let size = u32::try_from(entry.bytes.len()).expect("fixture size");
            let crc = crc32(&entry.bytes);

            write_u32(&mut out, 0x0403_4b50);
            write_u16(&mut out, 20);
            write_u16(&mut out, 0);
            write_u16(&mut out, 0);
            write_u16(&mut out, 0);
            write_u16(&mut out, 0);
            write_u32(&mut out, crc);
            write_u32(&mut out, size);
            write_u32(&mut out, size);
            write_u16(&mut out, u16::try_from(name.len()).expect("fixture name"));
            write_u16(&mut out, 0);
            out.extend_from_slice(name);
            out.extend_from_slice(&entry.bytes);

            write_u32(&mut central, 0x0201_4b50);
            write_u16(&mut central, 20);
            write_u16(&mut central, 20);
            write_u16(&mut central, 0);
            write_u16(&mut central, 0);
            write_u16(&mut central, 0);
            write_u16(&mut central, 0);
            write_u32(&mut central, crc);
            write_u32(&mut central, size);
            write_u32(&mut central, size);
            write_u16(
                &mut central,
                u16::try_from(name.len()).expect("fixture name"),
            );
            write_u16(&mut central, 0);
            write_u16(&mut central, 0);
            write_u16(&mut central, 0);
            write_u16(&mut central, 0);
            write_u32(&mut central, 0);
            write_u32(&mut central, offset);
            central.extend_from_slice(name);
        }

        let central_offset = u32::try_from(out.len()).expect("central offset");
        let central_size = u32::try_from(central.len()).expect("central size");
        out.extend_from_slice(&central);
        write_u32(&mut out, 0x0605_4b50);
        write_u16(&mut out, 0);
        write_u16(&mut out, 0);
        write_u16(&mut out, u16::try_from(entries.len()).expect("entry count"));
        write_u16(&mut out, u16::try_from(entries.len()).expect("entry count"));
        write_u32(&mut out, central_size);
        write_u32(&mut out, central_offset);
        write_u16(&mut out, 0);
        out
    }

    fn write_u16(out: &mut Vec<u8>, value: u16) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn write_u32(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xffff_ffff_u32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                let mask = 0_u32.wrapping_sub(crc & 1);
                crc = (crc >> 1) ^ (0xedb8_8320 & mask);
            }
        }
        !crc
    }
}
