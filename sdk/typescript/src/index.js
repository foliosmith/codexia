export class CodexiaError extends Error {
  constructor(status, code, message, retryable = false) {
    super(message);
    this.name = "CodexiaError";
    this.status = status;
    this.code = code;
    this.retryable = retryable;
  }
}

export class BookAgentClient {
  constructor({ baseUrl, apiKey, fetch: fetchImplementation = globalThis.fetch }) {
    if (!baseUrl || !apiKey || !fetchImplementation) {
      throw new TypeError("baseUrl, apiKey, and fetch are required");
    }
    this.baseUrl = baseUrl.replace(/\/$/, "");
    this.apiKey = apiKey;
    this.fetch = fetchImplementation;
  }

  async compile(epub, { profile = "standard", pollIntervalMs = 250 } = {}) {
    const accepted = await this.request("/v1/books", {
      method: "POST",
      headers: {
        "Content-Type": "application/epub+zip",
        "X-Codexia-Profile": profile,
      },
      body: epub,
    });
    await this.waitUntilReady(accepted.book_id, pollIntervalMs);
    return this.getBookPackage(accepted.book_id);
  }

  async waitUntilReady(bookId, pollIntervalMs = 250) {
    for (;;) {
      const status = await this.request(`/v1/books/${encodeURIComponent(bookId)}/status`);
      if (status.state === "ready") return status;
      if (status.state === "failed") {
        throw new CodexiaError(422, "compile_failed", status.error || "Compilation failed");
      }
      await new Promise(resolve => setTimeout(resolve, pollIntervalMs));
    }
  }

  getBookPackage(bookId) {
    return this.request(`/v1/books/${encodeURIComponent(bookId)}/package`);
  }

  explain(bookId, location, readerState, { intent = "explain", spoilerMode = "read_range" } = {}) {
    return this.action(bookId, "explain", {
      selected_text: location.selected_text,
      source_ref: location.source_ref,
      reader_state: readerState,
      spoiler_mode: spoilerMode,
      intent,
    });
  }

  ask(bookId, question, readerState, { spoilerMode = "read_range" } = {}) {
    return this.action(bookId, "ask", {
      question,
      reader_state: readerState,
      spoiler_mode: spoilerMode,
    });
  }

  checkpoint(bookId, chapterId, readerState, { spoilerMode = "read_range" } = {}) {
    return this.request(
      `/v1/books/${encodeURIComponent(bookId)}/chapters/${encodeURIComponent(chapterId)}/checkpoint`,
      this.jsonBody({ reader_state: readerState, spoiler_mode: spoilerMode }),
    );
  }

  reflect(bookId, chapterId, userAnswer, readerState) {
    return this.request(
      `/v1/books/${encodeURIComponent(bookId)}/chapters/${encodeURIComponent(chapterId)}/reflect`,
      this.jsonBody({ ...userAnswer, answer: userAnswer.answer, reader_state: readerState }),
    );
  }

  action(bookId, action, body) {
    return this.request(`/v1/books/${encodeURIComponent(bookId)}/${action}`, this.jsonBody(body));
  }

  jsonBody(value) {
    return {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(value),
    };
  }

  async request(path, options = {}) {
    const response = await this.fetch(`${this.baseUrl}${path}`, {
      ...options,
      headers: {
        "X-API-Key": this.apiKey,
        ...options.headers,
      },
    });
    const contentType = response.headers.get("content-type") || "";
    const payload = contentType.includes("json") ? await response.json() : await response.arrayBuffer();
    if (!response.ok) {
      const error = payload.error || {};
      throw new CodexiaError(response.status, error.code || "request_failed", error.message || response.statusText, error.retryable);
    }
    return payload;
  }
}
