# Notion Export

Generate a Codexia Markdown export, then run:

```bash
NOTION_TOKEN=secret_xxx node export.mjs notes.md <parent-page-id>
```

The integration uses the native Notion API and batches children in groups of
100. It does not transmit EPUB files or API keys to Codexia.
