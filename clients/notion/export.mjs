#!/usr/bin/env node
import { readFile } from "node:fs/promises";

const [filePath, parentPageId = process.env.NOTION_PARENT_PAGE_ID] = process.argv.slice(2);
const token = process.env.NOTION_TOKEN;
if (!filePath || !parentPageId || !token) {
  throw new Error("Usage: NOTION_TOKEN=… export.mjs <markdown-file> <parent-page-id>");
}

const markdown = await readFile(filePath, "utf8");
const title = markdown.match(/^#\s+(.+)$/m)?.[1] || "Codexia Export";
const children = markdownBlocks(markdown);
const page = await notion("/v1/pages", {
  parent: { page_id: parentPageId },
  properties: { title: { title: [richText(title)] } },
  children: children.slice(0, 100),
});
for (let index = 100; index < children.length; index += 100) {
  await notion(`/v1/blocks/${page.id}/children`, { children: children.slice(index, index + 100) }, "PATCH");
}
process.stdout.write(`${page.url}\n`);

function markdownBlocks(value) {
  return value.split(/\r?\n/).filter(Boolean).map(line => {
    const heading = line.match(/^(#{1,3})\s+(.+)$/);
    if (heading) return block(`heading_${heading[1].length}`, heading[2]);
    const bullet = line.match(/^[-*]\s+(.+)$/);
    if (bullet) return block("bulleted_list_item", bullet[1]);
    return block("paragraph", line.replace(/^>\s?/, ""));
  });
}

function block(type, text) { return { object: "block", type, [type]: { rich_text: [richText(text)] } }; }
function richText(content) { return { type: "text", text: { content: content.slice(0, 2000) } }; }
async function notion(path, body, method = "POST") {
  const response = await fetch(`https://api.notion.com${path}`, {
    method,
    headers: { Authorization: `Bearer ${token}`, "Notion-Version": "2022-06-28", "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const payload = await response.json();
  if (!response.ok) throw new Error(payload.message || `Notion HTTP ${response.status}`);
  return payload;
}
