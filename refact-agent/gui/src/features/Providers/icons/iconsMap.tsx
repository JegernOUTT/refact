import { AnthropicIcon } from "./Anthropic";
import { CustomIcon } from "./Custom";
import { DeepSeekIcon } from "./DeepSeek";
import { DoubaoIcon } from "./Doubao";
import { GeminiIcon } from "./Gemini";
import { GitHubCopilotIcon } from "./GitHubCopilot";
import { GroqIcon } from "./Groq";
import { KimiIcon } from "./Kimi";
import { LMStudioIcon } from "./LMStudio";
import { MiniMaxIcon } from "./MiniMax";
import { OllamaIcon } from "./Ollama";
import { OpenAIIcon } from "./OpenAI";
import { OpenRouterIcon } from "./OpenRouter";
import { QwenIcon } from "./Qwen";
import { VllmIcon } from "./Vllm";
import { XaiIcon } from "./Xai";
import { ZhipuIcon } from "./Zhipu";

export const iconsMap: Record<string, JSX.Element> = {
  openai: <OpenAIIcon />,
  openai_responses: <OpenAIIcon />,
  openai_codex: <OpenAIIcon />,
  anthropic: <AnthropicIcon />,
  claude_code: <AnthropicIcon />,
  google_gemini: <GeminiIcon />,
  openrouter: <OpenRouterIcon />,
  deepseek: <DeepSeekIcon />,
  groq: <GroqIcon />,
  ollama: <OllamaIcon />,
  lmstudio: <LMStudioIcon />,
  vllm: <VllmIcon />,
  xai: <XaiIcon />,
  xai_responses: <XaiIcon />,
  qwen: <QwenIcon />,
  kimi: <KimiIcon />,
  zhipu: <ZhipuIcon />,
  minimax: <MiniMaxIcon />,
  doubao: <DoubaoIcon />,
  github_copilot: <GitHubCopilotIcon />,
  custom: <CustomIcon />,
};

export function getProviderIcon(provider: {
  name: string;
  base_provider?: string;
}): JSX.Element | null {
  const providerName = provider.base_provider ?? provider.name;
  if (providerName in iconsMap) return iconsMap[providerName];
  return null;
}
