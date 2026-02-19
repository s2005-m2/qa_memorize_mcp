# Memorize MCP — OpenCode Agent Setup Guide

You are an AI assistant running inside OpenCode. Help the user configure memorize-mcp step by step.

## Step 0: Ask User Preferences

Before doing anything, ask the user:

1. **Scope**: "Do you want to install memorize-mcp for this project only, or globally for all projects?"
   - **Project-level**: config goes in `./opencode.json`, plugin file in `./.opencode/plugins/`
   - **User-level**: config goes in `~/.config/opencode/opencode.json`, plugin file in `~/.config/opencode/plugins/`

Wait for the user's answer before proceeding.

## Step 1: Add MCP Server

Based on the user's scope choice, determine the config file path:
- Project-level: `./opencode.json`
- User-level: `~/.config/opencode/opencode.json`

Check if the config file already exists. If it does, read it and merge — do NOT overwrite existing fields. If it does not exist, create it.

Add the `mcp.memorize` entry:

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

## Step 2: Install Auto-Recall Plugin

### 2a. Download the plugin file

For project-level:
```bash
mkdir -p .opencode/plugins
curl -o .opencode/plugins/opencode-plugin.mjs https://raw.githubusercontent.com/s2005-m2/qa_memorize_mcp/main/hooks/opencode-plugin.mjs
```

For user-level:
```bash
mkdir -p ~/.config/opencode/plugins
curl -o ~/.config/opencode/plugins/opencode-plugin.mjs https://raw.githubusercontent.com/s2005-m2/qa_memorize_mcp/main/hooks/opencode-plugin.mjs
```

### 2b. Register the plugin in opencode.json

Add the `plugin` field to the same config file from Step 1.

For project-level:
```json
"plugin": [
  "file://./.opencode/plugins/opencode-plugin.mjs"
]
```

For user-level (use the actual absolute path on the user's system):
```json
"plugin": [
  "file:///home/<username>/.config/opencode/plugins/opencode-plugin.mjs"
]
```

If the config file already has a `plugin` array, append to it instead of replacing.

## Step 3: Verify & Remind

After completing the setup, tell the user:
- Restart OpenCode for changes to take effect.
- Run `npx -y qa-memorize-mcp --help` to verify the MCP binary is accessible.

## Troubleshooting

- **MCP server not found**: Ensure `npx` is available in PATH.
- **No memory recall**: Check that `--hook-port 19533` is in the command array AND the `plugin` array points to the correct plugin file path.
- **Plugin not loading**: OpenCode loads plugins at startup only. A restart is required after changes.
