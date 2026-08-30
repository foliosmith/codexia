# Codexia Export Formats 0.1

Exports are created with `POST /v1/books/{book_id}/exports`.

```json
{
  "format": "json",
  "scope": "whole_book",
  "chapter_ids": [],
  "session_id": null
}
```

`scope` is one of:

- `whole_book`: every readable chapter;
- `chapters`: exactly the supplied `chapter_ids`;
- `read_range`: chapters through the persisted session's `read_until`.

## JSON

One UTF-8 JSON object containing `manifest`, `structure`, selected chapters and
chapter analyses, plus `book_map`, concepts, claims, entities, and checkpoints.
IDs and source references are unchanged from the Book Package.

## Markdown

One UTF-8 `.md` file. It starts with the book title, central question and
thesis, then emits selected chapters in spine order with summary, key ideas and
the requesting reader session's notes.

## Obsidian

A folder containing `Book.md` and one Markdown file per selected chapter.
Chapter files include YAML `book_id`/`chapter_id` properties and use ordinary
`[[wikilinks]]`; no Obsidian-specific plugin syntax is required.

Export paths are server-local. File exports return `href`; folder exports
return `local_path` because the v0.1 server deliberately does not invent an
archive format.
