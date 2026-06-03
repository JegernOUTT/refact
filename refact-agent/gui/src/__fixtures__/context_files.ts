import type { ChatContextFile } from "../services/refact";

const some_text = `import { CapsResponse } from "../services/refact";

export const STUB_CAPS_RESPONSE: CapsResponse = {
  caps_version: 0,
  chat_default_model: "gpt-3.5-turbo",
  chat_models: {
    "gpt-3.5-turbo": {
      default_scratchpad: "",
      n_ctx: 4096,
      similar_models: [],
      supports_scratchpads: {
        PASSTHROUGH: {
          default_system_message:
            "You are a coding assistant that outputs short answers, gives links to documentation.",
        },
      },
    },
    "test-model": {
      default_scratchpad: "",
      n_ctx: 4096,
      similar_models: [],
      supports_scratchpads: {
        PASSTHROUGH: {
          default_system_message:
            "You are a coding assistant that outputs short answers, gives links to documentation.",
        },
      },
    },
  },
  completion_default_model: "qwen2.5/coder/1.5b/base",
  chat_model_2: "",
  task_planner_agent_model: "",
  completion_models: {
    "qwen2.5/coder/1.5b/base": {
      default_scratchpad: "FIM-SPM",
      n_ctx: 4096,
      similar_models: ["qwen2.5/coder/1.5b/base", "qwen2.5/coder/1.5b/base"],
      supports_scratchpads: {
        "FIM-PSM": {},
        "FIM-SPM": {},
      },
    },
  },
  code_completion_n_ctx: 2048,
  endpoint_chat_passthrough:
    "http://127.0.0.1:8001/v1/chat/completions",
  endpoint_style: "openai",
  endpoint_template: "http://127.0.0.1:8001/v1/completions",
  running_models: ["qwen2.5/coder/1.5b/base", "gpt-3.5-turbo"],
  tokenizer_path_template:
    "https://huggingface.co/$MODEL/resolve/main/tokenizer.json",
  tokenizer_rewrite_path: {},
};
`;

export const CONTEXT_FILES: ChatContextFile[] = [
  {
    file_name:
      "/Users/refact/Projects/localai/refact-chat-js/src/components/ChatForm/index.tsx",
    file_content: some_text,
    line1: 1,
    line2: 100,
  },
  {
    file_name:
      "/Users/refact/Projects/localai/refact-chat-js/src/components/ChatForm/ChatForm.stories.tsx",
    file_content: some_text,
    line1: 1,
    line2: 100,
  },
  {
    file_name:
      "/Users/refact/Projects/localai/refact-chat-js/src/components/ChatForm/FilesPreview.tsx",
    file_content: some_text,
    line1: 1,
    line2: 100,
  },
  {
    file_name:
      "/Users/refact/Projects/localai/refact-chat-js/src/components/ChatForm/CharForm.test.tsx",
    file_content: some_text,
    line1: 1,
    line2: 100,
  },
  {
    file_name:
      "/Users/refact/Projects/localai/refact-chat-js/src/components/ChatForm/RetryForm.tsx",
    file_content: some_text,
    line1: 1,
    line2: 100,
  },
  {
    file_name:
      "/Users/refact/Projects/localai/refact-chat-js/src/components/ChatForm/ChatForm.module.css",
    file_content: some_text,
    line1: 1,
    line2: 100,
  },
  {
    file_name:
      "/Users/refact/Projects/localai/refact-chat-js/src/components/ChatForm/ChatForm.tsx",
    file_content: some_text,
    line1: 1,
    line2: 100,
  },
  {
    file_name:
      "/Users/refacts/Projects/localai/refact-chat-js/src/components/ChatForm/Form.tsx",
    file_content: some_text,
    line1: 1,
    line2: 100,
  },
];
