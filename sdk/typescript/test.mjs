import assert from "node:assert/strict";
import { BookAgentClient } from "./src/index.js";

const replies = [
  { book_id: "book-1", status_href: "/v1/books/book-1/status" },
  { state: "processing" },
  { state: "ready" },
  { book_id: "book-1", format_version: "0.1", entries: [] },
];
const calls = [];
const client = new BookAgentClient({
  baseUrl: "http://localhost:8790/",
  apiKey: "test-key",
  fetch: async (url, options) => {
    calls.push({ url, options });
    return new Response(JSON.stringify(replies.shift()), {
      status: url.endsWith("/v1/books") ? 202 : 200,
      headers: { "Content-Type": "application/json" },
    });
  },
});

const book = await client.compile(new Uint8Array([1, 2, 3]), { pollIntervalMs: 0 });
assert.equal(book.book_id, "book-1");
assert.equal(calls.length, 4);
assert.equal(calls[0].options.headers["X-API-Key"], "test-key");
assert.equal(calls[0].options.headers["X-Codexia-Profile"], "standard");
