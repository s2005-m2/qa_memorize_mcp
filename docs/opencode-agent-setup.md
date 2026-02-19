# Memorize MCP — OpenCode Agent Setup Guide

You are an AI assistant running inside OpenCode. Follow these steps exactly to configure memorize-mcp for the user.

## Step 1: Add MCP Server

Read the user's `opencode.json` (project root) or `~/.config/opencode/opencode.json` (global). If the file exists, merge the `mcp` field — do NOT overwrite the entire file.

Add this to the `mcp` section:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "memorize": {
      "type": "local",
      "command": ["npx", "-y", "qa-memorize-mcp", "--hook-port", "19533"],
      "enabled": true
    }
  }
}
```

## Step 2: Install Auto-Recall Plugin (optional but recommended)

The plugin injects relevant memories into the system prompt before every LLM call via the `experimental.chat.system.transform` hook.

Download the plugin script and register it in `opencode.json`:

```bash
# Project-level (current project only)
mkdir -p .opencode/plugins
curl -o .opencode/plugins/opencode-plugin.mjs https://raw.githubusercontent.com/s2005-m2/qa_memorize_mcp/main/hooks/opencode-plugin.mjs
```

Or for all projects:

```bash
# User-level (all projects)
mkdir -p ~/.config/opencode/plugins
curl -o ~/.config/opencode/plugins/opencode-plugin.mjs https://raw.githubusercontent.com/s2005-m2/qa_memorize_mcp/main/hooks/opencode-plugin.mjs
```

Then add the `plugin` field to the same `opencode.json` from Step 1:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "memorize": {
      "type": "local",
      "command": ["npx", "-y", "qa-memorize-mcp", "--hook-port", "19533"],
      "enabled": true
    }
  },
  "plugin": [
    "file://./.opencode/plugins/opencode-plugin.mjs"
  ]
}
```

For user-level install, use the absolute path instead:

```json
"plugin": [
  "file:///home/<username>/.config/opencode/plugins/opencode-plugin.mjs"
]
```

## Troubleshooting

- **MCP server not found**: Ensure `npx` is available in PATH. Run `npx -y qa-memorize-mcp --help` in terminal to verify.
- **No memory recall**: Check that `--hook-port 19533` is in the command array AND the `plugin` array in `opencode.json` points to the correct plugin file path.
- **Plugin not loading**: OpenCode loads plugins at startup only. A restart is required after adding the plugin.
