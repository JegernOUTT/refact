import type { ChatMessages, ToolCall } from "../services/refact";

function call(
  id: string,
  name: string,
  args: Record<string, unknown>,
): ToolCall {
  return {
    id,
    index: 0,
    type: "function",
    function: { name, arguments: JSON.stringify(args) },
  };
}

export const SUBAGENT_MESSAGES: ChatMessages = [
  {
    role: "user",
    content:
      "Research how chat tool results are associated with their originating calls.",
  },
  {
    role: "assistant",
    content: "I’ll ask a focused research agent to trace the data flow.",
    tool_calls: [
      call("agentic-subagent", "subagent", {
        task: "Trace tool-result association from messages to rendered cards",
        expected_result:
          "Name the selectors and grouping logic, with source locations",
        tools: "cat, search_pattern",
        max_steps: "8",
      }),
    ],
  },
  {
    role: "tool",
    tool_call_id: "agentic-subagent",
    tool_failed: false,
    content:
      "# Subagent Result\n\n**Task:** Trace tool-result association from messages to rendered cards\n\n## Response\n\nTool results are indexed by `tool_call_id`. Chat display-item construction collects adjacent supplemental messages, while thread selectors expose the matching result to each card.\n\nKey locations:\n- `ChatContentDisplayItems.ts` groups context and diff messages.\n- `features/Chat/Thread/selectors.ts` resolves tool results by id.",
    extra: {
      background_agent_id: "bg-research-tool-results",
      background_agent_kind: "subagent",
      background_agent_status: "completed",
      parent_chat_id: "storybook-chat",
      child_chat_id: "storybook-subagent-trajectory",
      title: "Trace tool-result rendering",
      progress: "Research complete",
      step_count: 6,
      last_activity: "Summarized selector and display-item flow",
      target_files: [
        "src/components/ChatContent/ChatContentDisplayItems.ts",
        "src/features/Chat/Thread/selectors.ts",
      ],
      edited_files: [],
      result_summary:
        "Located id-based result selection and supplemental grouping.",
      started_at: "2025-03-08T10:12:00.000Z",
      finished_at: "2025-03-08T10:14:20.000Z",
      change_seq: 7,
    },
  },
];

export const DELEGATE_MESSAGES: ChatMessages = [
  {
    role: "user",
    content: "Delegate the accessibility-label cleanup to a focused editor.",
  },
  {
    role: "assistant",
    content: "I’ll delegate the isolated component and test updates.",
    tool_calls: [
      call("agentic-delegate", "delegate", {
        task: "Improve accessible labels for the chat stop and retry controls",
        expected_result: "Product and test edits with a concise diff summary",
        tools: "cat, patch",
        max_steps: "10",
      }),
    ],
  },
  {
    role: "tool",
    tool_call_id: "agentic-delegate",
    tool_failed: false,
    content:
      "# Subagent Result\n\n**Task:** Improve accessible labels for the chat stop and retry controls\n\n## Response\n\nUpdated both controls to use action-oriented labels and added focused assertions covering their accessible names. No unrelated files were changed.",
    extra: {
      background_agent_id: "bg-delegate-a11y",
      background_agent_kind: "delegate",
      background_agent_status: "completed",
      parent_chat_id: "storybook-chat",
      child_chat_id: "storybook-delegate-trajectory",
      title: "Chat control accessibility cleanup",
      progress: "Edits completed",
      step_count: 8,
      last_activity: "Reviewed the final diff",
      target_files: [
        "src/components/ChatContent/MessageFooter.tsx",
        "src/components/ChatContent/MessageFooter.test.tsx",
      ],
      edited_files: [
        "src/components/ChatContent/MessageFooter.tsx",
        "src/components/ChatContent/MessageFooter.test.tsx",
      ],
      diff_summary:
        "+14 −6 across 2 files; labels clarified and assertions added",
      conflict_summary: null,
      result_summary: "Accessible names now describe stop and retry actions.",
      started_at: "2025-03-08T11:00:00.000Z",
      finished_at: "2025-03-08T11:05:42.000Z",
      change_seq: 12,
    },
  },
];

export const SET_TASKS_MESSAGES: ChatMessages = [
  {
    role: "assistant",
    content: "I’ll keep the implementation steps visible as work progresses.",
    tool_calls: [
      call("agentic-set-tasks", "set_tasks", {
        tasks: [
          {
            id: "inspect",
            content: "Inspect the existing tool result and diff message shapes",
            status: "completed",
          },
          {
            id: "fixtures",
            content: "Author realistic strict message fixtures",
            status: "in_progress",
          },
          {
            id: "stories",
            content: "Render each family through the real chat path",
            status: "pending",
          },
          {
            id: "review",
            content: "Review card arguments and result envelopes",
            status: "pending",
          },
        ],
      }),
    ],
  },
  {
    role: "tool",
    tool_call_id: "agentic-set-tasks",
    content: "Task list updated.",
    tool_failed: false,
  },
];

export const FINISH_MESSAGES: ChatMessages = [
  {
    role: "assistant",
    content: "The requested story coverage is complete.",
    tool_calls: [
      call("agentic-finish", "finish", {
        report:
          "Added agentic, browser, editing, and engine-analysis fixtures and stories. All examples travel through ChatContent rather than rendering cards directly.",
        summary: "Agentic tool-card stories completed",
      }),
    ],
  },
  {
    role: "tool",
    tool_call_id: "agentic-finish",
    tool_failed: false,
    content: JSON.stringify({
      type: "task_done",
      summary: "Agentic tool-card stories completed",
      report:
        "## Delivered\n\nCreated realistic fixtures for background agents, tasks, completion, wait/question flows, grouped edits, browser screenshots, and codegraph analysis.\n\nEach story renders through the shared chat harness.",
      files_changed: [
        "src/__fixtures__/toolCardsAgentic.ts",
        "src/components/ChatContent/ToolCardsAgentic.stories.tsx",
      ],
    }),
  },
];

export const SLEEP_ASK_MESSAGES: ChatMessages = [
  {
    role: "assistant",
    content:
      "I’ll wait briefly for the preview service, then confirm one choice.",
    tool_calls: [
      call("agentic-sleep", "sleep", {
        duration_ms: 5000,
        description: "Wait for the preview server health check",
      }),
    ],
  },
  {
    role: "tool",
    tool_call_id: "agentic-sleep",
    tool_failed: false,
    content: JSON.stringify({ slept_ms: 5000, interrupted: false }),
  },
  {
    role: "assistant",
    content: "The preview is ready. I need your preference before continuing.",
    tool_calls: [call("agentic-ask", "ask", {})],
  },
  {
    role: "tool",
    tool_call_id: "agentic-ask",
    tool_failed: false,
    content: JSON.stringify({
      type: "ask_questions",
      tool_call_id: "agentic-ask",
      questions: [
        {
          id: "viewport",
          type: "single_select",
          text: "Which viewport should be the primary visual baseline?",
          options: ["Desktop 1440px", "Tablet 768px", "Mobile 390px"],
        },
        {
          id: "dark-mode",
          type: "yes_no",
          text: "Should the baseline include dark mode?",
        },
      ],
    }),
  },
];

export const PATCH_WITH_DIFF_MESSAGES: ChatMessages = [
  {
    role: "assistant",
    content: "I’ll update the formatter and its focused test together.",
    tool_calls: [
      call("agentic-patch", "patch", {
        path: "src/utils/formatDuration.ts",
        old_str:
          "export const formatDuration = (seconds: number) => `${seconds}s`;",
        replacement:
          "export const formatDuration = (seconds: number) =>\n  seconds >= 60 ? `${Math.floor(seconds / 60)}m ${seconds % 60}s` : `${seconds}s`;",
      }),
    ],
  },
  {
    role: "diff",
    tool_call_id: "agentic-patch",
    content: [
      {
        file_name: "src/utils/formatDuration.ts",
        file_action: "edit",
        line1: 1,
        line2: 2,
        lines_remove:
          "export const formatDuration = (seconds: number) => `${seconds}s`;\n",
        lines_add:
          "export const formatDuration = (seconds: number) =>\n  seconds >= 60 ? `${Math.floor(seconds / 60)}m ${seconds % 60}s` : `${seconds}s`;\n",
      },
      {
        file_name: "src/utils/formatDuration.test.ts",
        file_action: "edit",
        line1: 8,
        line2: 11,
        lines_remove: '  expect(formatDuration(45)).toBe("45s");\n',
        lines_add:
          '  expect(formatDuration(45)).toBe("45s");\n  expect(formatDuration(125)).toBe("2m 5s");\n',
      },
    ],
  },
  {
    role: "tool",
    tool_call_id: "agentic-patch",
    content: "Updated 2 files.",
    tool_failed: false,
  },
];

export const CHROME_MESSAGES: ChatMessages = [
  {
    role: "assistant",
    content: "I’ll open the local preview and capture its current state.",
    tool_calls: [
      call("agentic-chrome", "chrome", {
        commands:
          "open_tab preview 1\nnavigate_to http://localhost:5173/chat 1\nscreenshot 1",
      }),
    ],
  },
  {
    role: "tool",
    tool_call_id: "agentic-chrome",
    tool_failed: false,
    content: [
      {
        m_type: "text",
        m_content:
          "opened a new tab: tab_id `1` device `desktop` uri `about:blank`\n\nnavigate_to successful: tab_id `1` device `desktop` uri `http://localhost:5173/chat`\nmade a screenshot of tab_id `1` device `desktop` uri `http://localhost:5173/chat`",
      },
      {
        m_type: "image/png",
        m_content:
          "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
      },
    ],
  },
];

export const ENGINE_ANALYSIS_MESSAGES: ChatMessages = [
  {
    role: "assistant",
    content: "I’ll inspect the codegraph index and its most central paths.",
    tool_calls: [
      call("agentic-codegraph", "codegraph_overview", {
        path: "src/components/ChatContent",
      }),
    ],
  },
  {
    role: "tool",
    tool_call_id: "agentic-codegraph",
    tool_failed: false,
    content: JSON.stringify({
      tool: "codegraph_overview",
      summary:
        "ChatContent forms one connected component with two entry points.",
      counts: { nodes: 48, edges: 83, files: 12 },
      index_state: {
        queued: 0,
        cross_file_edges: 41,
        cross_file_ready: true,
      },
      scc_count: 3,
      largest_scc: 8,
      component_count: 1,
      top_pagerank: [
        {
          symbol: "ChatContent",
          path: "src/components/ChatContent/ChatContent.tsx",
          score: 0.1842,
        },
      ],
      top_betweenness: [
        {
          symbol: "buildChatContentDisplayItems",
          path: "src/components/ChatContent/ChatContentDisplayItems.ts",
          score: 0.126,
        },
      ],
      file_centrality: {
        top_pagerank: [
          {
            path: "src/components/ChatContent/ChatContent.tsx",
            score: 0.205,
          },
        ],
        top_betweenness: [
          {
            path: "src/components/ChatContent/ToolsContent.tsx",
            score: 0.173,
          },
        ],
      },
      community_count: 2,
      dead_code_count: 0,
      partial: false,
      communities: [
        { label: "message rendering", member_count: 29, cohesion: 0.82 },
      ],
      execution_flows: [{ entry: "ChatContent", reaches: 39, depth: 6 }],
      dead_code: [],
      entry_points: ["src/components/ChatContent/ChatContent.tsx"],
      api_contract_files: ["src/services/refact/types.ts"],
    }),
  },
];
