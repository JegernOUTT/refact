import "@radix-ui/themes/styles.css";
import "../../src/styles/tokens.css";
import "../../src/styles/base.css";
import "../../src/styles/glass.css";
import "../../src/styles/motion.css";
import "../../src/styles/responsive.css";
import "../../src/styles/scrollbar.css";
import "../../src/components/Theme/theme-config.css";
import "../../src/components/shared/tokens.css";

import React from "react";
import { createRoot } from "react-dom/client";
import { Theme } from "@radix-ui/themes";
import { Provider } from "react-redux";
import { setUpStore } from "../../src/app/store";
import { AbortControllerProvider } from "../../src/contexts/AbortControllers";
import type { ChatThreadRuntime } from "../../src/features/Chat/Thread/types";
import type { ToolCall } from "../../src/services/refact/types";
import { AgentStatusView } from "../../src/components/ChatContent/AgentStatusView";
import { AgentPulseView } from "../../src/components/ChatContent/AgentPulseView";
import { AgentDiffView } from "../../src/components/ChatContent/AgentDiffView";

const fallback = new URLSearchParams(window.location.search).has("fallback");
if (fallback) {
  const setAttribute = Element.prototype.setAttribute;
  const removeAttribute = Element.prototype.removeAttribute;
  Element.prototype.setAttribute = function (name, value) {
    setAttribute.call(this, name === "inert" ? "data-test-inert" : name, value);
  };
  Element.prototype.removeAttribute = function (name) {
    removeAttribute.call(this, name === "inert" ? "data-test-inert" : name);
  };
}

const statusCall: ToolCall = {
  id: "dedicated-status",
  index: 0,
  function: { name: "check_agents", arguments: "{}" },
};
const pulseCall: ToolCall = {
  id: "dedicated-pulse",
  index: 1,
  function: { name: "agent_pulse", arguments: '{"card_id":"T-1"}' },
};
const diffCall: ToolCall = {
  id: "dedicated-diff",
  index: 2,
  function: { name: "agent_diff", arguments: '{"card_id":"T-1"}' },
};

function DedicatedFocusFixture() {
  const [store] = React.useState(() => {
    const runtime: ChatThreadRuntime = {
      thread: {
        id: "dedicated-focus",
        messages: [
          {
            role: "tool",
            tool_call_id: statusCall.id,
            content: JSON.stringify({
              agents: [
                {
                  card_id: "T-1",
                  title: "Focus inspection",
                  state: "running",
                  priority: "P1",
                },
              ],
            }),
          },
          {
            role: "tool",
            tool_call_id: pulseCall.id,
            content: [
              "# Agent Pulse: T-1",
              "**Card:** Focus inspection",
              "**State:** paused",
              "**Last activity:** 3m ago",
              "**Currently editing:** src/one.ts",
              "## Last assistant message",
              "> Checking nested focus",
              "## Last tool call",
              "`cat src/one.ts`",
            ].join("\n"),
          },
          {
            role: "tool",
            tool_call_id: diffCall.id,
            content: [
              "# Agent Diff for T-1",
              "**Card:** Focus inspection",
              "**Branch:** refact/focus",
              "**Base:** commit abc123",
              "```diff",
              ...["one", "two"].flatMap((file) => [
                `diff --git a/src/${file}.ts b/src/${file}.ts`,
                `--- a/src/${file}.ts`,
                `+++ b/src/${file}.ts`,
                "@@ -1 +1 @@",
                `-old ${file}`,
                `+new ${file}`,
              ]),
              "```",
            ].join("\n"),
          },
        ],
        title: "Dedicated focus",
        model: "",
        last_user_message_id: "",
        tool_use: "explore",
        new_chat_suggested: { wasSuggested: false },
        boost_reasoning: false,
        increase_max_tokens: false,
        include_project_info: false,
      },
      streaming: false,
      waiting_for_response: false,
      prevent_send: false,
      error: null,
      queued_items: [],
      send_immediately: false,
      attached_images: [],
      attached_text_files: [],
      background_agents: {},
      confirmation: {
        pause: false,
        pause_reasons: [],
        status: { wasInteracted: false, confirmationStatus: true },
      },
      snapshot_received: true,
      task_widget_expanded: false,
      memory_enrichment_user_touched: false,
      manual_preview_items: [],
      manual_preview_ran: false,
    };
    return setUpStore({
      chat: {
        current_thread_id: runtime.thread.id,
        open_thread_ids: [runtime.thread.id],
        threads: { [runtime.thread.id]: runtime },
        system_prompt: {},
        tool_use: "explore",
        sse_refresh_requested: null,
        stream_version: 0,
      },
    });
  });
  return (
    <Provider store={store}>
      <Theme appearance="light" data-host="web">
        <AbortControllerProvider>
          <main data-focus-mode={fallback ? "fallback" : "native"}>
            <section aria-label="Dedicated status">
              <AgentStatusView toolCall={statusCall} />
              <button>After status</button>
            </section>
            <section aria-label="Dedicated pulse">
              <AgentPulseView toolCall={pulseCall} />
              <button>After pulse</button>
            </section>
            <section aria-label="Dedicated diff">
              <AgentDiffView toolCall={diffCall} />
              <button>After diff</button>
            </section>
          </main>
        </AbortControllerProvider>
      </Theme>
    </Provider>
  );
}

const root = document.getElementById("root");
if (!root) throw new Error("Compact focus root is missing");
createRoot(root).render(<DedicatedFocusFixture />);
