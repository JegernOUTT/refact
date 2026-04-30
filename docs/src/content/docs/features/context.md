---
title: Context
description: How Refact builds local project context for chat, agents, and completion.
---

Refact uses a local engine to collect the context that helps models understand your project. Context is prepared on your machine and is sent only to the provider or local runtime selected for a specific request.

## Local context sources

Refact can use:

- Current file, cursor location, selected snippets, and open editors from the IDE.
- Project tree and file contents allowed by privacy settings.
- AST indexes for symbol definitions, references, and file symbols.
- Vector indexes for semantic search over code, markdown, and saved trajectories.
- Git state, checkpoints, and patch previews when agent workflows are enabled.
- Chat history, task metadata, saved knowledge, and previous trajectories.
- Tool results such as shell output, web pages, browser screenshots, database rows, and integration responses.

## Indexing

Syntax parsing and vector search run locally. Syntax parsing builds an AST-backed view of supported languages so Refact can find definitions and symbols. The vector database splits eligible files into chunks and stores embeddings for semantic search. These indexes improve retrieval, but they are optional and can be disabled.

## Privacy and control

Privacy settings decide which files are allowed as context. Provider settings decide where requests are sent. If you use a local runtime, model inference stays local to that runtime. If you use an external provider, the prepared prompt and selected context are sent to that provider.

## Context windows and compression

Each model has its own context window and tool capabilities. Refact fits local context into that window by selecting relevant files, compressing older chat history, and summarizing large tool results when needed. If the agent needs more context, ask it to inspect specific files or directories before acting.
