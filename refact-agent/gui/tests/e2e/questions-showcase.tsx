import React, { useState } from "react";
import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";
import { setUpStore } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { AbortControllerProvider } from "../../src/contexts/AbortControllers";
import { QuestionResponseForm } from "../../src/components/ChatContent/ToolCard/QuestionResponseForm";
import type { QuestionDraft } from "../../src/utils/questionResponses";
import type { QuestionResponse } from "../../src/services/refact/types";
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

const root = document.getElementById("root");
if (root)
  createRoot(root).render(
    <Provider store={setUpStore()}>
      <Theme>
        <AbortControllerProvider>
          <Showcase />
        </AbortControllerProvider>
      </Theme>
    </Provider>,
  );
