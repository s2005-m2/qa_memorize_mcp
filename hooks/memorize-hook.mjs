#!/usr/bin/env node
// Universal hook script for memorize_mcp auto-recall.
// Works with Claude Code (UserPromptSubmit) and Gemini CLI (BeforeAgent).
// Replaces memorize_hook.py — no Python dependency needed.

import { stdin } from "node:process";
import http from "node:http";

function recall(prompt, port, limit) {
  const url = `http://localhost:${port}/api/recall?q=${encodeURIComponent(prompt)}&limit=${limit}`;
  return new Promise((resolve) => {
    const req = http.get(url, { timeout: 2000 }, (res) => {
      let body = "";
      res.on("data", (chunk) => { body += chunk; });
      res.on("end", () => {
        try { resolve(JSON.parse(body)); } catch { resolve([]); }
      });
    });
    req.on("error", () => resolve([]));
    req.on("timeout", () => { req.destroy(); resolve([]); });
  });
}

function formatResults(results) {
  const lines = results.map((r) => {
    if (r.type === "qa") return `Q: ${r.question}\nA: ${r.answer}`;
    if (r.type === "knowledge") return `Knowledge: ${r.text}`;
    return "";
  }).filter(Boolean);
  if (!lines.length) return null;
  return "[Memory Recall]\n" + lines.join("\n---\n");
}

async function main() {
  const chunks = [];
  for await (const chunk of stdin) chunks.push(chunk);
  const raw = Buffer.concat(chunks).toString();

  let data;
  try { data = JSON.parse(raw); } catch { return; }

  const hookEvent = data.hook_event_name || "";
  const prompt = data.prompt || "";
  if (!prompt) return;

  const port = process.env.MEMORIZE_HOOK_PORT || "19533";
  const limit = process.env.MEMORIZE_RECALL_LIMIT || "5";

  const results = await recall(prompt, port, limit);
  const context = formatResults(results);
  if (!context) return;

  if (hookEvent === "UserPromptSubmit") {
    process.stdout.write(context);
  } else if (hookEvent === "BeforeAgent") {
    process.stdout.write(JSON.stringify({
      hookSpecificOutput: {
        hookEventName: "BeforeAgent",
        additionalContext: context,
      },
    }));
  }
}

main();
