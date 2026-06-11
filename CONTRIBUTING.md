## 🤝 Contributing to Refact Agent

Thanks for your interest in contributing to Refact Agent! Refact is an open-source, local-first agentic coding engine: the engine runs on your machine, project state lives under `<project>/.refact/`, and model access is configured through BYOK providers or local runtimes.

Whether you are fixing a bug, adding a model, improving docs, or extending tools and integrations, your work helps make Refact better for everyone.

## 🌱 How You Can Contribute

- Try Refact locally and open issues when you hit bugs or have feature ideas.
- Add or update a model/provider definition.
- Improve agent tools, MCP integrations, IDE integrations, or the React GUI.
- Improve documentation and examples.

If you have something else in mind, start a GitHub Discussion or issue in [JegernOUTT/refact](https://github.com/JegernOUTT/refact). We are happy to help shape a useful contribution.

## 📚 Table of Contents

- [🚀 Quick Start](#-quick-start)
- [🛠️ Development Environment Setup](#️-development-environment-setup)
- [🔑 Model Providers: BYOK or Local Runtime](#-model-providers-byok-or-local-runtime)
- [🧠 Adding Chat Models](#-adding-chat-models)
- [⚡ Adding Completion Models](#-adding-completion-models)
- [🔌 Adding New Providers](#-adding-new-providers)
- [🧪 Testing Your Contributions](#-testing-your-contributions)
- [📋 Best Practices](#-best-practices)
- [🐛 Troubleshooting](#-troubleshooting)
- [💡 Examples](#-examples)
- [🎯 Next Steps](#-next-steps)

## 🚀 Quick Start

Before diving deep, here are the moving parts:

1. **Engine**: Rust binary `refact-lsp`, serving HTTP and/or LSP locally.
2. **GUI**: React frontend in `refact-agent/gui/`, connecting to the local engine on port `8001` during development.
3. **Chat Models**: conversational/agentic models such as Claude, GPT, DeepSeek, or local OpenAI-compatible runtimes.
4. **Completion Models**: code-completion models, preferably FIM models such as Qwen Coder, StarCoder, or DeepSeek Coder.
5. **Providers**: YAML definitions and user configuration that tell Refact how to reach a model host.

For deeper docs, see the GitHub Wiki pages for [BYOK](https://github.com/JegernOUTT/refact/wiki/BYOK), [Providers](https://github.com/JegernOUTT/refact/wiki/Providers), [Supported Models](https://github.com/JegernOUTT/refact/wiki/Supported-Models), and [Agent Tools](https://github.com/JegernOUTT/refact/wiki/Agent-Tools).

## 🛠️ Development Environment Setup

### Prerequisites

- **Rust** latest stable toolchain.
- **Node.js** and **npm** for the React GUI.
- **Chrome/Chromium** for browser/tooling features that need it.
- **Git**.
- A model provider configured through BYOK, or a local runtime such as Ollama, LM Studio, or vLLM.

### Setting Up the Rust Backend (Engine)

```bash
# Clone the repository
git clone https://github.com/JegernOUTT/refact.git
cd refact

# Install Rust if needed
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# Build and check the engine
cd refact-agent/engine
cargo check
cargo build

# Run a local HTTP + LSP dev server for the current workspace
cargo run -- --http-port 8001 --lsp-port 8002 --logs-stderr --vecdb --ast --workspace-folder ../..
```

Useful current engine flags are defined in `refact-agent/engine/src/global_context.rs` and wired through `refact-agent/engine/src/main.rs`/`src/lib.rs`:

- `--http-port 8001` starts the local HTTP API used by the GUI and IDE integrations.
- `--lsp-port 8002` starts a TCP LSP server; use `--lsp-stdin-stdout 1` for stdio LSP mode instead.
- `--logs-stderr` prints logs in the terminal; otherwise logs go under `~/.cache/refact/logs/`.
- `--vecdb` enables vector search indexing when an embedding model is configured.
- `--ast` enables AST indexing.
- `--workspace-folder <path>` seeds the workspace files for AST/VecDB and project state.

### Setting Up the React Frontend (GUI)

```bash
# In a new terminal from the repository root
cd refact-agent/gui
npm ci
npm run dev
```

The development frontend connects to the local engine on `http://127.0.0.1:8001`.

## 🔑 Model Providers: BYOK or Local Runtime

Refact no longer depends on a hosted Refact service for development. Configure model access in one of these ways:

- Use the GUI Provider Setup to add your own provider credentials.
- Add provider YAML under `~/.config/refact/providers.d/*.yaml` for user-local configuration.
- Run a local OpenAI-compatible runtime such as Ollama, LM Studio, or vLLM and point a provider config at it.

User config lives under `~/.config/refact/`, cache/log data under `~/.cache/refact/`, and project-local state under `<project>/.refact/`.

## 🧠 Adding Chat Models

Chat models are used for conversational and agentic interactions.

### Step 1: Add to Provider Configuration

For existing providers, edit the appropriate YAML file in `refact-agent/engine/src/yaml_configs/default_providers/`:

```yaml
# Example: anthropic.yaml
running_models:
  - claude-3-7-sonnet-latest
  - claude-3-5-sonnet-latest
  - your-new-model

chat_models:
  your-new-model:
    n_ctx: 200000
    supports_tools: true
    supports_multimodality: true
    supports_agent: true
    tokenizer: anthropic
```

For related defaults, also review completion and embedding presets in the engine YAML/config directories.

### Step 2: Test the Model

Once configured, test the model in the Refact GUI:

- Does it appear in `/v1/caps`?
- Can it run a normal chat?
- Can it call tools if `supports_tools` is enabled?
- Does multimodality work if enabled?
- Do reasoning and context-window settings match the provider's real behavior?

## ⚡ Adding Completion Models

Completion models are used for code completion. FIM (Fill-in-the-Middle) models work best.

### Step 1: Understand FIM Tokens

FIM models use special tokens:

- `fim_prefix`: text before the cursor.
- `fim_suffix`: text after the cursor.
- `fim_middle`: where the generated completion goes.
- `eot`: end-of-text token.

### Step 2: Add to Known Models

Add the model to a provider YAML or the relevant known-model JSON:

```json
{
  "completion_models": {
    "your-completion-model": {
      "n_ctx": 8192,
      "scratchpad_patch": {
        "fim_prefix": "<|fim_prefix|>",
        "fim_suffix": "<|fim_suffix|>",
        "fim_middle": "<|fim_middle|>",
        "eot": "<|endoftext|>",
        "extra_stop_tokens": ["<|repo_name|>", "<|file_sep|>"],
        "context_format": "your-format",
        "rag_ratio": 0.5
      },
      "scratchpad": "FIM-PSM",
      "tokenizer": "hf://your-tokenizer-path",
      "similar_models": []
    }
  }
}
```

### Step 3: Test Code Completion

Use the Refact IDE plugin in xDebug/local-development mode so it connects to your local `refact-lsp` server. Trigger completions in a real workspace and verify latency, stop tokens, and FIM placement.

## 🔌 Adding New Providers

To add a completely new OpenAI-compatible provider:

### Step 1: Create Provider Configuration

Create `refact-agent/engine/src/yaml_configs/default_providers/your-provider.yaml`:

```yaml
chat_endpoint: https://api.your-provider.com/v1/chat/completions
completion_endpoint: https://api.your-provider.com/v1/completions
embedding_endpoint: https://api.your-provider.com/v1/embeddings
supports_completion: true

api_key: your-api-key-format

running_models:
  - your-model-1
  - your-model-2

model_default_settings_ui:
  chat:
    n_ctx: 128000
    supports_tools: true
    supports_multimodality: false
    supports_agent: true
    tokenizer: hf://your-default-tokenizer
  completion:
    n_ctx: 8192
    tokenizer: hf://your-completion-tokenizer
```

### Step 2: Add to Provider List

Edit `refact-agent/engine/src/caps/providers.rs` and add your provider to the `PROVIDER_TEMPLATES` array:

```rust
const PROVIDER_TEMPLATES: &[(&str, &str)] = &[
    ("anthropic", include_str!("../yaml_configs/default_providers/anthropic.yaml")),
    ("openai", include_str!("../yaml_configs/default_providers/openai.yaml")),
    // ... existing providers ...
    ("your-provider", include_str!("../yaml_configs/default_providers/your-provider.yaml")),
];
```

### Step 3: Test Provider Integration

Use the GUI Provider Setup and `/v1/caps` to verify the provider can be configured, models appear correctly, and chat/completion requests use the expected endpoint and authorization format.

## 🧪 Testing Your Contributions

Run focused checks for the area you changed before opening a PR.

### Engine Checks

```bash
cd refact-agent/engine
cargo check
cargo test --lib
cargo test --doc
```

For release-build issues or performance-sensitive work, also run:

```bash
cargo build --release
```

### GUI Checks

```bash
cd refact-agent/gui
npm ci
npm run types
npm run lint
npm run test
npm run build
```

### Manual Testing Checklist

- Model/provider appears in the capabilities endpoint (`/v1/caps`).
- Chat functionality works.
- Code completion works for completion models.
- Tool calling works when supported.
- Multimodality works when supported.
- Errors are clear and recoverable.
- Performance is acceptable for realistic workspaces.

## 📋 Best Practices

### Model Configuration

1. **Context windows**: set realistic `n_ctx` values based on the model's real limits.
2. **Capabilities**: only enable features the model actually supports.
3. **Tokenizers**: use the correct tokenizer for accurate token counting.
4. **Similar models**: group models with similar behavior where appropriate.

### Provider Configuration

1. **Secrets**: keep credentials in user-local config or environment-specific secret handling; do not commit personal keys.
2. **Endpoints**: verify URLs and OpenAI compatibility before adding them to defaults.
3. **Error handling**: test invalid credentials, unavailable models, and provider rate limits.
4. **Local runtimes**: document any non-default base URL, model ID, or compatibility quirk.

### Code Quality

1. Keep changes scoped and easy to review.
2. Follow the relevant `AGENTS.md` for Rust, GUI, or plugin conventions.
3. Use clear commit messages.
4. Prefer small, focused tests over broad snapshots.

## 🐛 Troubleshooting

### Common Issues

**Model not appearing in capabilities:**

- Ensure the provider YAML is loaded.
- Check that the model is listed under `running_models` and the correct `chat_models` or `completion_models` section.
- Confirm required capability flags such as `supports_agent` are set for agentic mode.

**Tokenizer errors:**

- Verify the tokenizer path is correct.
- Use a supported fallback tokenizer only when appropriate for testing.

**Provider connection issues:**

- Verify endpoint URLs and authorization format.
- Check your BYOK credentials or local runtime health.
- Test the endpoint with `curl` or the provider's own examples.

**Completion not working:**

- Ensure FIM tokens are configured correctly.
- Check `scratchpad` type and context format.
- Verify the IDE extension is connected to the local LSP/HTTP server you started.

### Debug Commands

```bash
# Test local engine endpoints
curl http://127.0.0.1:8001/v1/caps
curl http://127.0.0.1:8001/v1/rag-status

# Validate engine code
cd refact-agent/engine
cargo check
```

## 💡 Examples

### Example 1: Adding Claude 4 (Hypothetical)

Make sure your model is listed in the config with all required fields:

```yaml
chat_models:
  claude-4:
    n_ctx: 200000
    supports_tools: true
    supports_multimodality: true
    supports_agent: true
    supports_reasoning: anthropic
    supports_boost_reasoning: true
    tokenizer: anthropic

  claude-3-7-sonnet-latest:
    n_ctx: 200000
    supports_tools: true
    supports_multimodality: true
    supports_agent: true
    supports_reasoning: anthropic
    supports_boost_reasoning: true
    tokenizer: anthropic
```

### Example 2: Adding a New FIM Model

```json
"new-coder-model": {
  "n_ctx": 16384,
  "scratchpad_patch": {
    "fim_prefix": "<PRE>",
    "fim_suffix": "<SUF>",
    "fim_middle": "<MID>",
    "eot": "<EOT>"
  },
  "scratchpad": "FIM-PSM",
  "tokenizer": "hf://company/new-coder-model"
}
```

### Example 3: Adding a Custom Provider

```yaml
# custom-ai.yaml
chat_endpoint: https://api.example.com/v1/chat/completions
supports_completion: false

api_key: sk-example-...

chat_models:
  example-agent-model:
    n_ctx: 200000
    supports_tools: true
    supports_multimodality: true
    supports_clicks: true
    supports_agent: true
    tokenizer: hf://example/tokenizer

model_default_settings_ui:
  chat:
    n_ctx: 200000
    supports_tools: true
    supports_multimodality: true
    supports_agent: true
    tokenizer: hf://example/tokenizer
```

## 🎯 Next Steps

1. Join community discussions at [GitHub Discussions](https://github.com/JegernOUTT/refact/discussions).
2. Check [GitHub Issues](https://github.com/JegernOUTT/refact/issues) for contribution opportunities.
3. Read the [GitHub Wiki](https://github.com/JegernOUTT/refact/wiki) for deeper guides.

Happy contributing! 🚀
