#!/usr/bin/env python3
"""Universal hook script for memorize_mcp auto-recall.
Works with Claude Code (UserPromptSubmit) and Gemini CLI (BeforeAgent).
"""
import json, sys, os, urllib.request, urllib.error, urllib.parse


def main():
    try:
        data = json.loads(sys.stdin.read())
    except Exception:
        return

    hook_event = data.get("hook_event_name", "")
    prompt = data.get("prompt", "")
    if not prompt:
        return

    port = os.environ.get("MEMORIZE_HOOK_PORT", "19533")
    limit = os.environ.get("MEMORIZE_RECALL_LIMIT", "5")
    url = f"http://localhost:{port}/api/recall?q={urllib.parse.quote(prompt)}&limit={limit}"

    try:
        with urllib.request.urlopen(urllib.request.Request(url), timeout=2) as resp:
            results = json.loads(resp.read().decode())
    except Exception:
        return

    if not results:
        return

    lines = []
    for r in results:
        if r.get("type") == "qa":
            lines.append(f"Q: {r['question']}\nA: {r['answer']}")
        elif r.get("type") == "knowledge":
            lines.append(f"Knowledge: {r['text']}")

    if not lines:
        return

    context = "[Memory Recall]\n" + "\n---\n".join(lines)

    if hook_event == "UserPromptSubmit":
        print(context)
    elif hook_event == "BeforeAgent":
        output = {
            "hookSpecificOutput": {
                "hookEventName": "BeforeAgent",
                "additionalContext": context
            }
        }
        print(json.dumps(output))


if __name__ == "__main__":
    main()
