import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";
import { setUpStore } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { ChatContent } from "../../src/components/ChatContent/ChatContent";
import { AbortControllerProvider } from "../../src/contexts/AbortControllers";
import {
  ChatThreadProvider,
  applyChatEvent,
  createChatWithId,
} from "../../src/features/Chat/Thread";
import { makeReview } from "../../src/features/Review/reviewFixtures";
import { reviewApi } from "../../src/services/refact/review";
import type { Review } from "../../src/services/refact/review";
import "../../src/lib/render/web.css";

// Synthetic UI transport: no engine, generation, or real backend is involved.
const params = new URLSearchParams(location.search);
const key = `synthetic-proposal-${params.get("key") ?? "default"}`;
const target = { candidate_id: "c1", step_id: "s1", version: 1 };
const initial = makeReview({
  tour: [
    {
      id: "s1",
      current: {
        ...target,
        title: "Original",
        body: "Original text",
        anchors: [],
      },
      history: [],
    },
  ],
  threads: [
    {
      id: "t1",
      anchors: [],
      annotation_id: null,
      resolved: false,
      contributions: [
        {
          id: "contribution",
          thread_id: "t1",
          actor: { kind: "agent", id: "agent" },
          body: "Canonical synthetic proposal",
          blocks: [],
          created_at_ms: 1,
          chat_message_id: "message",
        },
      ],
    },
  ],
  tour_revision_proposals: [
    {
      id: "p1",
      target,
      thread_id: "t1",
      proposal_contribution_id: "contribution",
      change_request: "Explain ordering",
      proposed_by: { kind: "agent", id: "agent" },
      proposed_at_ms: 1,
      acceptance: null,
      fulfilment: null,
    },
  ],
});
type FixtureState = {
  review: Review;
  posts: { path: string; body: unknown }[];
};
const saved = sessionStorage.getItem(key);
let state: FixtureState = saved
  ? (JSON.parse(saved) as FixtureState)
  : { review: initial, posts: [] };
const persist = () => sessionStorage.setItem(key, JSON.stringify(state));
const json = (value: unknown, status = 200) =>
  new Response(JSON.stringify(value), {
    status,
    headers: { "Content-Type": "application/json" },
  });
window.fetch = async (input, init) => {
  const request = input instanceof Request ? input : new Request(input, init);
  const path = new URL(request.url).pathname;
  if (request.method === "POST") {
    const body: unknown = await request.json().catch(() => null);
    if (path === "/v1/privacy/inspect")
      return json({
        chat_id: "chat-1",
        destination: {
          kind: "provider",
          id: "fixture",
          display_name: "fixture",
        },
        sendable: true,
        records: [],
        blocked: [],
        refusal: null,
      });
    state.posts.push({ path, body });
    persist();
    if (path !== "/v1/reviews/r1/tour-revision-proposals/p1/accept")
      return json({ error: "unexpected synthetic post" }, 404);
    const value = body as Record<string, unknown>;
    if (
      Object.keys(value).sort().join(",") !==
        "expected_revision,owner_chat_id,request_key,target" ||
      value.owner_chat_id !== "chat-1" ||
      value.expected_revision !== state.review.revision ||
      typeof value.request_key !== "string" ||
      !value.request_key ||
      JSON.stringify(value.target) !== JSON.stringify(target) ||
      state.review.tour_revision_proposals?.[0].acceptance
    )
      return json({ error: "invalid acceptance" }, 409);
    const proposal = state.review.tour_revision_proposals![0];
    state.review = {
      ...state.review,
      revision: state.review.revision + 1,
      tour_revision_proposals: [
        {
          ...proposal,
          acceptance: {
            actor: { kind: "user", id: "gui" },
            accepted_at_ms: 2,
            contribution_id: "receipt",
          },
        },
      ],
    };
    persist();
    return json(state.review);
  }
  if (path === "/v1/reviews/r1") return json(state.review);
  return json({ error: `unexpected synthetic request ${path}` }, 404);
};
const store = setUpStore({
  config: {
    host: "web",
    dev: true,
    lspUrl: location.origin,
    lspPort: Number(location.port),
    apiKey: null,
    themeProps: {
      appearance: params.get("theme") === "dark" ? "dark" : "light",
    },
  },
});
store.dispatch(createChatWithId({ id: "chat-1" }));
store.dispatch(
  applyChatEvent({
    chat_id: "chat-1",
    seq: "1",
    type: "snapshot",
    thread: {
      id: "chat-1",
      title: "Synthetic chat",
      model: "fixture",
      mode: "review",
      tool_use: "agent",
      boost_reasoning: false,
      context_tokens_cap: null,
      include_project_info: false,
      checkpoints_enabled: false,
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
    background_agents: [],
    messages: [
      {
        role: "assistant",
        message_id: "message",
        content: "Fallback proposal",
        extra: {
          review: {
            id: "r1",
            thread_id: "t1",
            contribution_id: "contribution",
            blocks: [],
            tour_revision_proposal_id: "p1",
          },
        },
      },
    ],
  }),
);
declare global {
  interface Window {
    __proposalFixture: { snapshot: () => FixtureState; fulfil: () => void };
  }
}
window.__proposalFixture = {
  snapshot: () => structuredClone(state),
  fulfil: () => {
    const step = state.review.tour[0];
    const proposal = state.review.tour_revision_proposals![0];
    if (!proposal.acceptance) throw new Error("accept first");
    state.review = {
      ...state.review,
      revision: state.review.revision + 1,
      tour: [
        {
          ...step,
          current: {
            ...step.current,
            version: 2,
            title: "Authored fixture update",
          },
          history: [step.current],
        },
      ],
      tour_revision_proposals: [
        {
          ...proposal,
          fulfilment: {
            actor: { kind: "agent", id: "fixture-author" },
            fulfilled_at_ms: 3,
            version: 2,
          },
        },
      ],
    };
    persist();
    store.dispatch(
      reviewApi.util.invalidateTags([{ type: "Review", id: "r1" }]),
    );
  },
};
const root = document.getElementById("root");
if (!root) throw new Error("Missing root");
createRoot(root).render(
  <Provider store={store}>
    <Theme>
      <AbortControllerProvider>
        <ChatThreadProvider chatId="chat-1">
          <main>
            <p>SYNTHETIC FIXTURE — no backend or model execution</p>
            <ChatContent
              onRetry={() => undefined}
              onStopStreaming={() => undefined}
            />
          </main>
        </ChatThreadProvider>
      </AbortControllerProvider>
    </Theme>
  </Provider>,
);
