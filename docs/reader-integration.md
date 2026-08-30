# Third-party Reader Integration

## 1. Compile and wait

Send the EPUB bytes to `POST /v1/books`, then poll the returned `status_href`
until `state` is `ready`. Authenticate with `X-API-Key` or `Authorization:
Bearer …`.

## 2. Create a reader session

Create a session with the first visible `chapter_id`. Persist the returned
`session_id` and send session updates as the reader moves. `read_until` is
monotonic and is the trust boundary used by contextual actions.

## 3. Render the package

Use the package and chapter endpoints for content. Treat `block_id` as the DOM
anchor and keep its `text_fingerprint`; source navigation must resolve through
these stable IDs rather than fuzzy text search.

## 4. Invoke the Book Agent

`explain`, `ask`, `checkpoint`, and `reflect` require the current `ReaderState`.
Do not fabricate a wider `read_until`. Render the returned Reader Cards using
[`reader-card.schema.json`](../schemas/reader-card.schema.json), and make every
`source_ref` an explicit jump back to the source block.

## 5. Handle errors and limits

- `401`: missing or invalid API key;
- `403`: source or response crossed the spoiler boundary;
- `429`: retry after the rate-limit window;
- `502`: analyzer output failed the Reader Card contract.

Use `GET /v1/usage` for request, byte and estimated-cost totals. Webhooks emit
`book.processing.completed` or `book.processing.failed`; consumers should be
idempotent by `(event, book_id)`.

The TypeScript reference client lives in [`sdk/typescript`](../sdk/typescript/).
