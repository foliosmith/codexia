# `@foliosmith/codexia`

Zero-dependency ESM client for the Codexia Public API.

```ts
import { BookAgentClient } from "@foliosmith/codexia";

const client = new BookAgentClient({
  baseUrl: "http://127.0.0.1:8790",
  apiKey: process.env.CODEXIA_API_KEY!,
});

const book = await client.compile(await file.arrayBuffer());
```

Every contextual action requires an explicit `ReaderState`; the SDK does not
silently widen spoiler boundaries.
