// OpenCode plugin for memorize_mcp auto-recall
// Uses chat.params hook to inject RAG context before every LLM call

export default function(input) {
  const port = process.env.MEMORIZE_HOOK_PORT || "19533";
  const limit = process.env.MEMORIZE_RECALL_LIMIT || "5";

  return {
    "chat.params": async ({ input: params, output }) => {
      try {
        const msg = typeof params.message === "string"
          ? params.message
          : params.message?.content || "";
        if (!msg) return output;

        const url = `http://localhost:${port}/api/recall?context=${encodeURIComponent(msg)}&limit=${limit}`;
        const controller = new AbortController();
        const timeout = setTimeout(() => controller.abort(), 2000);

        const resp = await fetch(url, { signal: controller.signal });
        clearTimeout(timeout);

        if (!resp.ok) return output;
        const results = await resp.json();
        if (!results.length) return output;

        const lines = results.map(r => {
          if (r.type === "qa") return `Q: ${r.question}\nA: ${r.answer}`;
          if (r.type === "knowledge") return `Knowledge: ${r.text}`;
          return "";
        }).filter(Boolean);

        if (!lines.length) return output;
        const context = "[Memory Recall]\n" + lines.join("\n---\n");

        return {
          ...output,
          options: {
            ...output.options,
            system: (output.options?.system || "") + "\n\n" + context,
          },
        };
      } catch {
        return output;
      }
    },
  };
}
