import React, { useState } from "react";
import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";
import { setUpStore } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { AbortControllerProvider } from "../../src/contexts/AbortControllers";
import { QuestionResponseForm } from "../../src/components/ChatContent/ToolCard/QuestionResponseForm";
import { AskQuestionsTool } from "../../src/components/ChatContent/ToolCard/AskQuestionsTool";
import {
  applyChatEvent,
  newChatAction,
} from "../../src/features/Chat/Thread/actions";
import type { QuestionDraft } from "../../src/utils/questionResponses";
import type {
  PendingQuestionSet,
  QuestionResponse,
} from "../../src/services/refact/types";
import "../../src/lib/render/web.css";
import "./questions-showcase.css";

function Showcase() {
  const [drafts, setDrafts] = useState<Partial<Record<string, QuestionDraft>>>(
    {},
  );
  const [comments, setComments] = useState("");
  const [response, setResponse] = useState<QuestionResponse | null>(null);
  return (
    <main className="questions-showcase">
      <h1>Questions for you</h1>
      <QuestionResponseForm
        instance="browser-fixture"
        drafts={drafts}
        comments={comments}
        onDrafts={setDrafts}
        onComments={setComments}
        onSubmit={setResponse}
        disabled={response !== null}
        questions={[
          {
            id: "direction",
            type: "single_select",
            text: "Which direction should the dashboard take?",
            options: ["Compact and focused", "Roomy and contextual"],
          },
          {
            id: "features",
            type: "multi_select",
            text: "What should be visible at a glance?",
            options: ["In-flight work", "Recent changes", "Usage and costs"],
          },
          {
            id: "preview",
            type: "free_text",
            text: "Tune the panel until it feels right.\n\n```html\n<button onclick=\"window.refactQuestion.setAnswer('Mint accent, 12px spacing')\">Use Mint layout</button>\n<button onclick=\"window.refactQuestion.setAnswer('Violet accent, 8px spacing')\">Use Violet layout</button>\n<script>fetch('https://blocked.invalid/network-probe').catch(()=>{});</script>\n```",
          },
        ]}
      />
      {response && (
        <pre data-testid="question-response">
          {JSON.stringify(response, null, 2)}
        </pre>
      )}
    </main>
  );
}

const native = new URLSearchParams(window.location.search).get("native");
const nativeCall = {
  id:
    native === "approval"
      ? "agent-approval:browser"
      : "native-browser-question",
  index: 0,
  function: { name: "ask_questions", arguments: "{}" },
};
const store = setUpStore();
if (native) {
  store.dispatch(newChatAction(undefined));
  const id = store.getState().chat.current_thread_id;
  const status =
    native === "answered" || native === "cancelled" ? native : "pending";
  const set: PendingQuestionSet = {
    tool_call_id: nativeCall.id,
    asked_at_ms: 42,
    response_protocol_version: 2,
    status,
    questions: [
      {
        id: "direction",
        type: "single_select",
        text: "Choose the next direction",
        options: ["Compact", "Roomy"],
      },
      { id: "permission", type: "yes_no", text: "Continue with this request?" },
      {
        id: "artifact",
        type: "free_text",
        text: "Preview the layout.\n\n```html\n<button onclick=\"window.refactQuestion.setAnswer('Mint layout')\">Use Mint layout</button>\n```",
      },
    ],
    ...(status === "answered"
      ? {
          response: {
            answers: [
              {
                id: "direction",
                disposition: "custom",
                answer: "Combine both",
                explanation: "A deliberate choice",
              },
              { id: "permission", disposition: "skipped" },
              {
                id: "artifact",
                disposition: "answered",
                answer: "Mint layout",
              },
            ],
            overall_comments: "Keep the full history",
          },
        }
      : {}),
  };
  store.dispatch(
    applyChatEvent({
      type: "snapshot",
      chat_id: id,
      seq: "1",
      thread: {
        id,
        title: "Native questions",
        model: "",
        mode: "agent",
        tool_use: "agent",
        boost_reasoning: false,
        context_tokens_cap: null,
        include_project_info: true,
        checkpoints_enabled: true,
        is_title_generated: false,
      },
      runtime: {
        state: "idle",
        paused: false,
        error: null,
        queue_size: 0,
        pause_reasons: [],
        queued_items: [],
      },
      messages: [],
      background_agents: [],
      pending_questions: [set],
    }),
  );
}

function NativeShowcase() {
  return (
    <main className="questions-showcase">
      <h1>Compact native questions</h1>
      <button>Before question</button>
      <AskQuestionsTool toolCall={nativeCall} compact />
      <button>After question</button>
    </main>
  );
}

const root = document.getElementById("root");
if (root)
  createRoot(root).render(
    <Provider store={store}>
      <Theme>
        <AbortControllerProvider>
          {native ? <NativeShowcase /> : <Showcase />}
        </AbortControllerProvider>
      </Theme>
    </Provider>,
  );
