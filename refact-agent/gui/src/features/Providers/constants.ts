export const BEAUTIFUL_PROVIDER_NAMES: Record<string, string> = {
  openai: "OpenAI",
  openai_responses: "OpenAI (Responses API)",
  openai_codex: "OpenAI Codex",
  openrouter: "OpenRouter",
  groq: "Groq",
  anthropic: "Anthropic",
  claude_code: "Claude Code",
  deepseek: "DeepSeek",
  google_gemini: "Google Gemini",
  ollama: "Ollama",
  opencode: "OpenCode Go",
  lmstudio: "LM Studio",
  vllm: "vLLM",
  xai: "xAI",
  xai_responses: "xAI (Responses API)",
  qwen: "Qwen",
  kimi: "Kimi / Moonshot AI",
  zhipu: "Z.AI / Zhipu",
  minimax: "MiniMax",
  doubao: "Doubao / Volcengine",
  github_copilot: "GitHub Copilot",
  custom: "Custom Provider",
};

export const HIDDEN_PROVIDER_BASES = [
  "openai_responses",
  "xai_responses",
] as const;
