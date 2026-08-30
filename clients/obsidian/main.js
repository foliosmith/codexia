const { Notice, Plugin, PluginSettingTab, Setting } = require("obsidian");

const DEFAULTS = { endpoint: "http://127.0.0.1:8790", apiKey: "", bookId: "" };

module.exports = class CodexiaPlugin extends Plugin {
  async onload() {
    this.settings = Object.assign({}, DEFAULTS, await this.loadData());
    this.addSettingTab(new CodexiaSettings(this.app, this));
    this.addCommand({
      id: "insert-book-map",
      name: "Insert current book map",
      editorCallback: async editor => {
        try {
          const map = await this.request(`/v1/books/${encodeURIComponent(this.settings.bookId)}/map`);
          editor.replaceSelection(`# ${map.central_question}\n\n${map.thesis}\n`);
        } catch (error) {
          new Notice(error.message);
        }
      },
    });
    this.addCommand({
      id: "insert-chapter-analysis",
      name: "Insert chapter analysis by ID",
      editorCallback: async editor => {
        const chapterId = window.prompt("Codexia chapter ID", "chapter_001");
        if (!chapterId) return;
        try {
          const card = await this.request(`/v1/books/${encodeURIComponent(this.settings.bookId)}/chapters/${encodeURIComponent(chapterId)}/analysis`);
          const summary = card.content.summary;
          editor.replaceSelection(`## ${card.title}\n\n${summary.short}\n`);
        } catch (error) {
          new Notice(error.message);
        }
      },
    });
  }

  async request(path) {
    if (!this.settings.apiKey || !this.settings.bookId) throw new Error("Configure API key and book ID first");
    const response = await fetch(`${this.settings.endpoint.replace(/\/$/, "")}${path}`, {
      headers: { "X-API-Key": this.settings.apiKey },
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error?.message || `HTTP ${response.status}`);
    return payload;
  }
};

class CodexiaSettings extends PluginSettingTab {
  constructor(app, plugin) { super(app, plugin); this.plugin = plugin; }
  display() {
    const { containerEl } = this; containerEl.empty();
    for (const [key, name, placeholder] of [
      ["endpoint", "API endpoint", DEFAULTS.endpoint],
      ["apiKey", "API key", "codexia key"],
      ["bookId", "Book ID", "book id"],
    ]) {
      new Setting(containerEl).setName(name).addText(text => text
        .setPlaceholder(placeholder)
        .setValue(this.plugin.settings[key])
        .onChange(async value => { this.plugin.settings[key] = value; await this.plugin.saveData(this.plugin.settings); }));
    }
  }
}
