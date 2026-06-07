## Bring your own models — any provider, any runtime
Refact is BYOK all the way down: hosted frontier APIs, local runtimes, self-hosted OpenAI-compatible stacks, and OAuth-backed coding agents can all sit in the same cockpit. Pick the right brain per task, keep credentials and policy local, and let capability-aware routing do the boring glue work.

- 20+ provider families are first-class citizens: Anthropic, OpenAI, OpenAI Responses, OpenAI Codex, OpenRouter, Groq, DeepSeek, Doubao, xAI, xAI Responses, Google Gemini, Qwen, Kimi, Zhipu, MiniMax, GitHub Copilot, Claude Code, plus custom endpoints and local runtimes.
- Run fully local or self-hosted when you want the keys off the internet: Ollama, LM Studio, vLLM, and any OpenAI-compatible `/chat/completions`, `/responses`, `/completions`, or embeddings route can be wired in.
- Mix models by role instead of marrying one vendor: chat, agent, task planner, light chat, thinking, Buddy, code completion/FIM, and embeddings each have their own defaults.
- Capability metadata keeps selections honest: tool use, agent mode, reasoning, multimodality, context windows, cache control, tokenizer/FIM settings, and embeddings are resolved before a model is offered for work.
- OAuth is built into the provider surface for OpenAI Codex and Claude Code, while classic API-key providers stay plain BYOK through local provider config.
- Adding a new provider is intentionally tiny: create one YAML template for endpoints, wire format, defaults, and model caps, then add one entry to the provider template list.
- The result is model freedom without config soup: swap paid frontier models, cheap routers, local coders, and private deployments per task while Refact keeps the same agentic tool loop.

→ Deep dive: [BYOK](https://github.com/JegernOUTT/refact/wiki/BYOK), [Providers](https://github.com/JegernOUTT/refact/wiki/Providers), [Supported Models](https://github.com/JegernOUTT/refact/wiki/Supported-Models)
