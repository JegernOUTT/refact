---
title: Code Completion
description: Code completion with the local Refact engine and your configured provider.
---

Refact code completion runs through the local engine in your IDE. The engine prepares completion context from the active file and sends the request only to the completion provider or local runtime you configure.

## How it works

1. The IDE sends the active file, cursor position, and nearby text to the local Refact engine.
2. The engine builds a fill-in-the-middle completion request using the allowed local context.
3. The configured completion model returns a suggestion.
4. The IDE shows the suggestion inline so you can accept, edit, or ignore it.

## Context used for completion

Completion context can include:

- Text before and after the cursor.
- Open-file and nearby-file context allowed by privacy settings.
- Local syntax information when parsing is enabled.
- Project-specific settings that control what files can be used.

Completion does not require the autonomous agent. It is optimized for quick inline suggestions and usually uses less context than a full chat or agent request.

## Providers and local runtimes

Choose a completion-capable model in provider settings. Completion can use hosted BYOK providers, OpenAI-compatible endpoints, or local runtimes such as Ollama, LM Studio, and vLLM when they expose a compatible completion model.

Model quality, latency, and maximum context depend on the selected provider or local runtime. If suggestions are slow or incomplete, check the provider connection, model choice, and context/privacy settings.

## Privacy

Completion requests are sent only to the configured provider or local runtime. Refact does not send snippet telemetry. Files excluded by privacy settings are not used as completion context.
