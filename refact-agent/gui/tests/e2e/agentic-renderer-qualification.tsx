import React from "react";
import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";
import { setUpStore } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { AbortControllerProvider } from "../../src/contexts/AbortControllers";
import { ToolContent } from "../../src/components/ChatContent/ToolsContent";
import { TaskDoneTool } from "../../src/components/ChatContent/ToolCard/TaskDoneTool";
import { AskQuestionsTool } from "../../src/components/ChatContent/ToolCard/AskQuestionsTool";
import { CollapsibleStoreProvider } from "../../src/components/ChatContent/useStoredOpen";
import { ChatThreadProvider } from "../../src/features/Chat/Thread/ChatThreadContext";
import {
  makeChatSlice,
  makeChatThread,
} from "../../src/__stories__/chatStoryState";
import type {
  PendingQuestionSet,
  ToolCall,
  ToolMessage,
} from "../../src/services/refact/types";
import "../../src/lib/render/web.css";

const config = {
  host: "web" as const,
  dev: true,
  lspUrl: window.location.origin,
  lspPort: Number(window.location.port),
  apiKey: null,
  themeProps: {
    appearance:
      new URLSearchParams(window.location.search).get("theme") === "dark"
        ? ("dark" as const)
        : ("light" as const),
  },
};
const report = `# Complete report\n\n${"Evidence preserved. ".repeat(
  80,
)}\n\n[Source reference](#report-source)\n\nEND OF REPORT`;
const questions: PendingQuestionSet["questions"] = [
  { id: "continue", type: "yes_no", text: "Continue with this scope?" },
  {
    id: "scope",
    type: "single_select",
    text: "Choose the renderer scope",
    options: ["Compact choice", "Expanded choice"],
  },
];
const openChoices = new Map<string, boolean>();
const collapseStore = {
  get: (key: string) => openChoices.get(key),
  set: (key: string, value: boolean) => {
    openChoices.set(key, value);
  },
};

function Case({
  id,
  outcome,
  mode = "compact",
}: {
  id: string;
  outcome?: "success" | "failure" | "unknown" | "malformed" | "missing";
  mode?: "compact" | "standalone" | "v1" | "v2" | "legacy" | "v2-open";
}) {
  const [fixture] = React.useState(() => {
    const isQuestion = outcome === undefined;
    const call: ToolCall = {
      id,
      index: 0,
      type: "function",
      function: {
        name: isQuestion ? "ask_questions" : "task_done",
        arguments: "{}",
      },
    };
    const content = isQuestion
      ? JSON.stringify({ type: "ask_questions", tool_call_id: id, questions })
      : outcome === "malformed"
        ? "{broken"
        : JSON.stringify({
            type: "task_done",
            summary: "Renderer delivery reported",
            report,
            files_changed: ["src/renderer.ts"],
            ...(outcome === "success"
              ? { success: true }
              : outcome === "failure"
                ? { success: false }
                : {}),
          });
    const messages: ToolMessage[] =
      outcome === "missing"
        ? []
        : [{ role: "tool", tool_call_id: id, content }];
    const pending: PendingQuestionSet = {
      tool_call_id: id,
      asked_at_ms: 42,
      status: "pending",
      response_protocol_version: mode === "v1" ? 1 : 2,
      questions,
    };
    return {
      call,
      store: setUpStore({
        config,
        chat: makeChatSlice(
          makeChatThread({
            id,
            messages,
            ...(isQuestion && mode !== "legacy"
              ? { pending_questions: [pending] }
              : {}),
          }),
        ),
      }),
    };
  });
  return (
    <article data-testid={id}>
      <Provider store={fixture.store}>
        <ChatThreadProvider chatId={id}>
          {mode === "standalone" ? (
            <TaskDoneTool toolCall={fixture.call} />
          ) : mode === "v2-open" ? (
            <AskQuestionsTool toolCall={fixture.call} />
          ) : (
            <ToolContent toolCalls={[fixture.call]} threadId={id} />
          )}
        </ChatThreadProvider>
      </Provider>
    </article>
  );
}

const themeStore = setUpStore({ config });
const root = document.getElementById("root");
if (!root) throw new Error("Renderer qualification root is missing");
createRoot(root).render(
  <Provider store={themeStore}>
    <Theme>
      <AbortControllerProvider>
        <CollapsibleStoreProvider value={collapseStore}>
          <main aria-label="Production renderer qualification">
            <Case id="q2-unknown" outcome="unknown" />
            <Case id="q2-failure" outcome="failure" />
            <Case id="q2-success" outcome="success" />
            <Case id="q2-malformed" outcome="malformed" />
            <Case id="q2-missing" outcome="missing" />
            <Case id="q2-standalone" outcome="failure" mode="standalone" />
            <Case id="question-v1" mode="v1" />
            <Case id="question-v2" mode="v2" />
            <Case id="question-legacy" mode="legacy" />
            <Case id="question-v2-open" mode="v2-open" />
          </main>
        </CollapsibleStoreProvider>
      </AbortControllerProvider>
    </Theme>
  </Provider>,
);
