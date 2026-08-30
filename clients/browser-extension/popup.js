const endpoint = document.querySelector("#endpoint");
const apiKey = document.querySelector("#api-key");
const status = document.querySelector("#status");
const result = document.querySelector("#result");

chrome.storage.local.get(["endpoint", "apiKey"], saved => {
  if (saved.endpoint) endpoint.value = saved.endpoint;
  if (saved.apiKey) apiKey.value = saved.apiKey;
});

document.querySelector("#explain").addEventListener("click", async () => {
  status.textContent = "读取选区…"; result.value = "";
  await chrome.storage.local.set({ endpoint: endpoint.value, apiKey: apiKey.value });
  try {
    const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
    const [{ result: selection }] = await chrome.scripting.executeScript({
      target: { tabId: tab.id },
      func: () => {
        const selected = getSelection();
        if (!selected || selected.rangeCount !== 1 || !selected.toString().trim()) return null;
        const range = selected.getRangeAt(0);
        const element = (range.commonAncestorContainer.nodeType === 1 ? range.commonAncestorContainer : range.commonAncestorContainer.parentElement)?.closest?.("[data-block-id][data-fingerprint]");
        const readerState = document.body.dataset.codexiaReaderState;
        if (!element || !readerState) return null;
        const prefix = range.cloneRange(); prefix.selectNodeContents(element); prefix.setEnd(range.startContainer, range.startOffset);
        return { text: selected.toString(), blockId: element.dataset.blockId, fingerprint: element.dataset.fingerprint, start: Array.from(prefix.toString()).length, readerState: JSON.parse(readerState), bookId: document.body.dataset.codexiaBookId };
      },
    });
    if (!selection) throw new Error("页面未暴露 Codexia block/reader state");
    const response = await fetch(`${endpoint.value.replace(/\/$/, "")}/v1/books/${selection.bookId}/explain`, {
      method: "POST",
      headers: { "X-API-Key": apiKey.value, "Content-Type": "application/json" },
      body: JSON.stringify({ selected_text: selection.text, source_ref: { block_id: selection.blockId, start_char: selection.start, end_char: selection.start + Array.from(selection.text).length, text_fingerprint: selection.fingerprint }, reader_state: selection.readerState, spoiler_mode: "read_range", intent: "explain" }),
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error?.message || `HTTP ${response.status}`);
    result.value = payload.cards[0]?.content?.explanation || JSON.stringify(payload.cards[0]?.content, null, 2);
    status.textContent = "完成";
  } catch (error) { status.textContent = error.message; }
});
