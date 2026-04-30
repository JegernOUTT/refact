---
title: MCP Server Integration
description: Connect local and remote Model Context Protocol servers.
---

The MCP integration lets Refact Agent use tools provided by Model Context Protocol servers. MCP servers can expose external APIs, documentation search, local utilities, or organization-specific workflows as agent tools.

## Transports

Refact supports:

- **stdio MCP servers**: local commands such as `npx -y package-name`, a Python module, or another executable that speaks MCP over stdin/stdout.
- **HTTP/SSE-style MCP servers**: remote endpoints configured with a URL, headers, and optional authentication.

The unified MCP configuration accepts either a command or a URL and selects the transport from that configuration.

## Configuration

Common fields include:

- **Command** for local stdio servers.
- **URL** for remote HTTP/SSE servers.
- **Environment variables** for command-based servers.
- **Headers** for URL-based servers.
- **Authentication** for URL-based servers, including none, bearer token, or OAuth2 settings.
- **Init and request timeouts**.

## Lazy tool discovery

When an MCP server exposes many tools, Refact can switch to lazy MCP mode. Instead of sending every tool schema to the model, the agent receives two proxy tools:

- `mcp_tool_search` searches available MCP tools by name or description and returns matching schemas.
- `mcp_call` executes a selected MCP tool by exact name with the required arguments.

This keeps the model context smaller while still allowing access to all MCP tools.

## Confirmation rules

MCP tools can read or change external systems depending on the server. Keep confirmation rules strict until you understand what each server exposes. Prefer least-privilege tokens and review tool descriptions before allowing write operations.

## Troubleshooting

If a server does not appear, test the integration from settings, verify the command or URL, check required environment variables or headers, and confirm that the selected chat mode allows MCP tools.
