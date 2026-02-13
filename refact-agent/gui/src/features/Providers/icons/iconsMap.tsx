import { AnthropicIcon } from "./Anthropic";
import { CustomIcon } from "./Custom";
import { DeepSeekIcon } from "./DeepSeek";
import { GeminiIcon } from "./Gemini";
import { GroqIcon } from "./Groq";
import { LMStudioIcon } from "./LMStudio";
import { OllamaIcon } from "./Ollama";
import { OpenAIIcon } from "./OpenAI";
import { OpenRouterIcon } from "./OpenRouter";
import { RefactIcon } from "./Refact";
import { XaiIcon } from "./Xai";

export const iconsMap: Record<string, JSX.Element> = {
  refact: <RefactIcon />,
  refact_self_hosted: <RefactIcon />,
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
  xai: <XaiIcon />,
  xai_responses: <XaiIcon />,
  custom: <CustomIcon />,
};
