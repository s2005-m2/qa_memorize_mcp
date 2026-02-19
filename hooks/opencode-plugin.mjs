// OpenCode plugin for memorize_mcp auto-recall
// Uses experimental.chat.system.transform to inject RAG context into system prompt

export default function({ client }) {
  const port = process.env.MEMORIZE_HOOK_PORT || "19533";
  const limit = process.env.MEMORIZE_RECALL_LIMIT || "5";
  let lastUserMessage = "";

  return {
    "chat.message": async (input, output) => {
      lastUserMessage = (output.parts || [])
        .filter(p => p.type === "text")
        .map(p => p.text || "")
        .join("") || "";
    },
    "experimental.chat.system.transform": async (input, output) => {
      try {
        if (!lastUserMessage) return;

        const url = `http://localhost:${port}/api/recall?context=${encodeURIComponent(lastUserMessage)}&limit=${limit}`;
        const controller = new AbortController();
        const timeout = setTimeout(() => controller.abort(), 2000);

        const resp = await fetch(url, { signal: controller.signal });
        clearTimeout(timeout);

        if (!resp.ok) return;
        const results = await resp.json();
        if (!results.length) return;

        const lines = results.map(r => {
          if (r.type === "qa") return `Q: ${r.question}\nA: ${r.answer}`;
          if (r.type === "knowledge") return `Knowledge: ${r.text}`;
          return "";
        }).filter(Boolean);

        if (lines.length) {
          output.system.push("[Memory Recall]\n" + lines.join("\n---\n"));
        }
      } catch {}
    },
  };
}
