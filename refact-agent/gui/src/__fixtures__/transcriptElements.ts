import type { ChatMessages } from "../services/refact";
import { FIXTURE_IMAGE_DATA_URI } from "./images";

const longParagraph =
  "This deliberately detailed request describes the expected behavior, edge cases, accessibility requirements, and visual states so the transcript has enough real text to exercise its collapsed presentation. ";

export const LONG_USER_MESSAGE: ChatMessages = [
  {
    role: "user",
    message_id: "transcript-long-user",
    content: Array.from(
      { length: 14 },
      (_, index) => `Requirement ${index + 1}: ${longParagraph.repeat(2)}`,
    ).join("\n"),
  },
];

export const COMPRESSED_USER_MESSAGE: ChatMessages = [
  {
    role: "user",
    message_id: "transcript-compressed-user",
    content:
      "🗜️ Earlier discussion compressed: the user asked for an accessible transcript, stable fixtures, and clear coverage of all non-tool display elements.",
  },
];

export const USER_WITH_IMAGES: ChatMessages = [
  {
    role: "user",
    message_id: "transcript-user-image",
    content: [
      {
        type: "text",
        text: "Please describe the tiny reference image attached to this message.",
      },
      {
        type: "image_url",
        image_url: {
          url: FIXTURE_IMAGE_DATA_URI,
        },
      },
    ],
  },
];

export const REASONING_ASSISTANT: ChatMessages = [
  {
    role: "user",
    message_id: "transcript-reasoning-user",
    content: "Explain how you checked the transcript fixtures.",
  },
  {
    role: "assistant",
    message_id: "transcript-reasoning-assistant",
    reasoning_content:
      "I first checked the message union and followed each role through display-item construction.\n\nThen I compared the renderer expectations with the fixture fields.**Validation Result** The user, assistant, and supplemental message shapes all use their current contracts.\n\nFinally, I kept the normal answer separate from this reasoning summary.",
    content:
      "The fixtures follow the current message types and exercise the production transcript path.",
    finish_reason: "stop",
  },
];

export const THINKING_BLOCKS_ASSISTANT: ChatMessages = [
  {
    role: "user",
    message_id: "transcript-thinking-user",
    content: "Show a response whose reasoning comes from thinking blocks.",
  },
  {
    role: "assistant",
    message_id: "transcript-thinking-assistant",
    thinking_blocks: [
      {
        type: "thinking",
        thinking:
          "I will inspect the requested state first, then keep the visible response concise.",
        signature: "storybook-thinking-signature",
      },
      {
        type: "reasoning",
        thinking:
          "The second block demonstrates that multiple signed blocks are combined without duplicate text.",
        signature: "storybook-reasoning-signature",
      },
    ],
    content:
      "The transcript can derive its reasoning panel from thinking blocks.",
    finish_reason: "stop",
  },
];

export const EVENT_MESSAGES: ChatMessages = [
  {
    role: "user",
    message_id: "transcript-events-user",
    content:
      "The internal events after this message should not appear as ordinary chat turns.",
  },
  {
    role: "event",
    message_id: "transcript-event-process",
    content: "Background index refresh completed successfully.",
    subkind: "process_completed",
    source: "exec.registry",
    payload: {
      process_id: "storybook-background-process",
      status: "exited",
      exit_code: 0,
    },
    extra: {
      event: {
        subkind: "process_completed",
        source: "exec.registry",
        payload: {
          process_id: "storybook-background-process",
          status: "exited",
          exit_code: 0,
        },
      },
    },
  },
  {
    role: "event",
    message_id: "transcript-event-cron",
    content: "Scheduled transcript maintenance fired.",
    subkind: "cron_fire",
    source: "scheduler.cron",
    payload: { task_id: "storybook-maintenance" },
    extra: {
      event: {
        subkind: "cron_fire",
        source: "scheduler.cron",
        payload: { task_id: "storybook-maintenance" },
      },
    },
  },
  {
    role: "event",
    message_id: "transcript-event-notice",
    content: "Internal transcript synchronization notice.",
    subkind: "system_notice",
    source: "chat.runtime",
    payload: { severity: "info" },
    extra: {
      event: {
        subkind: "system_notice",
        source: "chat.runtime",
        payload: { severity: "info" },
      },
    },
  },
  {
    role: "assistant",
    message_id: "transcript-events-assistant",
    content:
      "The event messages remain hidden from ordinary turns, leaving this response readable.",
    finish_reason: "stop",
  },
];

export const COMPRESSION_REPORT: ChatMessages = [
  {
    role: "compression_report",
    message_id: "transcript-compression-report",
    content:
      "Older messages were compacted while preserving the active task and key decisions.",
    summarization_tier: "tier1_llm",
    summarized_token_estimate: 2100,
    extra: {
      compression_report: {
        kind: "chat_compression_report",
        compression_kind: "llm_segment_summary",
        insert_mode: "source_preserving",
        source_message_count: 18,
        source_message_ids: ["source-1", "source-2", "source-3"],
        summarized_source_message_ids: ["source-1", "source-2"],
        preserved_source_message_ids: ["source-3"],
        source_hash: "report-source-hash",
        summary_model: "storybook-summary-model",
        context_files_removed: 2,
        context_messages_dropped: 1,
        tool_results_truncated: 3,
        preserved_context_file_count: 1,
        compressed_tool_output_count: 2,
        tokens_before: 6400,
        tokens_after: 2300,
        estimated_tokens_saved: 4100,
        reduction_percent: 64.1,
      },
    },
  },
  {
    role: "user",
    message_id: "transcript-compression-follow-up",
    content: "Continue using the compacted context.",
  },
  {
    role: "assistant",
    message_id: "transcript-segment-summary",
    content:
      "The earlier segment established the fixture contract, rendering path, and accessibility expectations.",
    summarization_tier: "tier1_llm",
    summarized_token_estimate: 950,
    extra: {
      compression: {
        kind: "llm_segment_summary",
        schema_version: 1,
        insert_mode: "source_preserving",
        source_hash: "segment-source-hash",
        source_message_ids: ["segment-1", "segment-2"],
        summarized_source_message_ids: ["segment-1", "segment-2"],
        preserved_source_message_ids: [],
        created_at: "2025-01-15T12:00:00.000Z",
        summary_model: "storybook-summary-model",
      },
    },
    finish_reason: "stop",
  },
];

export const CONTEXT_FILES: ChatMessages = [
  {
    role: "user",
    message_id: "transcript-context-user",
    content: "Inspect the transcript renderer source before answering.",
  },
  {
    role: "assistant",
    message_id: "transcript-context-assistant",
    content: "I inspected the relevant source attachment.",
    tool_calls: [
      {
        id: "transcript-cat-call",
        index: 0,
        type: "function",
        function: {
          name: "cat",
          arguments:
            '{"paths":"src/components/ChatContent/ChatContentDisplayItems.ts"}',
        },
      },
    ],
    finish_reason: "tool_calls",
  },
  {
    role: "context_file",
    message_id: "transcript-context-attachment",
    tool_call_id: "transcript-cat-call",
    content: [
      {
        file_name: "src/components/ChatContent/ChatContentDisplayItems.ts",
        file_content:
          "export function buildDisplayItems(messages: ChatMessages): DisplayItem[] {\n  return buildDisplayItemsFromIndex(messages, false, new Set(), 0);\n}\n",
        line1: 455,
        line2: 457,
        usefulness: 0.98,
      },
    ],
  },
];

export const SKILL_CARDS: ChatMessages = [
  {
    role: "cd_instruction",
    message_id: "transcript-skill-activated",
    content:
      '💿 SKILL_ACTIVATED {"name":"storybook-authoring","allowed_tools":["cat","search_pattern"],"model_override":null}\nUse production message shapes, render through the shared harness, and keep stories focused.',
  },
  {
    role: "plain_text",
    message_id: "transcript-skill-report",
    content:
      "## Skill Report: storybook-authoring\n- Verified fixture fields against exported message types.\n- Exercised both activation and report cards through display-item construction.",
  },
];

export const ERROR_DISPLAY: ChatMessages = [
  {
    role: "error",
    message_id: "transcript-error",
    content: "The provider temporarily stopped before completing the response.",
    error_info: {
      category: "ProviderTransient",
      title: "Temporary provider interruption",
      explanation:
        "The model provider ended the request unexpectedly, but the transcript remains intact.",
      suggested_action: "Retry the response in a moment.",
      is_retryable: true,
      raw_error: "upstream connection closed",
    },
    retry_status: {
      attempt: 1,
      max_attempts: 3,
      delay_secs: 2,
      in_progress: false,
    },
  },
];
