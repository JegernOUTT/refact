import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";

import { setUpStore } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { AbortControllerProvider } from "../../src/contexts/AbortControllers";
import { applyChatEvent } from "../../src/features/Chat/Thread";
import { ReviewChatSurface } from "../../src/features/Review";
import surfacePaneStyles from "../../src/features/Workspace/SurfacePane.module.css";
import "../../src/lib/render/web.css";

/*
 * Browser fixture: production ReviewChatSurface with no review open, in one
 * pane or two side-by-side panes (?panes=2). Review endpoints return null.
 */

const params = new URLSearchParams(window.location.search);
const paneCount = params.get("panes") === "2" ? 2 : 1;
const chatIds = Array.from(
  { length: paneCount },
  (_, index) => `narrow-chat-${index + 1}`,
);

const config = {
  host: "web" as const,
  dev: true,
  lspUrl: window.location.origin,
  lspPort: Number(window.location.port),
  apiKey: null,
  themeProps: { appearance: "light" as const },
};

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

const realFetch = window.fetch.bind(window);
window.fetch = async (input, init) => {
  const request = input instanceof Request ? input : new Request(input, init);
  const url = new URL(request.url);
  if (
    url.pathname.startsWith("/v1/review-stacks/by-chat/") ||
    url.pathname.startsWith("/v1/reviews/by-chat/")
  )
    return json(null);
  if (url.pathname.startsWith("/v1/")) return json({ detail: "fixture" }, 404);
  return realFetch(input, init);
};

const store = setUpStore({ config });
for (const chatId of chatIds) {
  store.dispatch(
    applyChatEvent({
      chat_id: chatId,
      seq: "1",
      type: "snapshot",
      thread: {
        id: chatId,
        title: "Narrow chat fixture",
        model: "fixture-model",
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
      background_agents: [],
      messages: [],
    }),
  );
}

function ChatPlaceholder({ chatId }: { chatId: string }) {
  return (
    <section
      aria-label={`Chat ${chatId}`}
      className="fixture-chat"
      data-testid="chat-content"
    >
      <div className="fixture-chat-log">
        <p>Chat transcript placeholder for {chatId}.</p>
      </div>
      <textarea aria-label={`Composer ${chatId}`} rows={4} />
    </section>
  );
}

const root = document.getElementById("root");
if (!root) throw new Error("Narrow chat fixture root is missing");
createRoot(root).render(
  <Provider store={store}>
    <Theme>
      <AbortControllerProvider>
        <div className="fixture-panes">
          {chatIds.map((chatId) => (
            <div
              key={chatId}
              className="fixture-pane"
              data-testid="fixture-pane"
            >
              <div className={surfacePaneStyles.surfacePane}>
                <ReviewChatSurface chatId={chatId}>
                  <ChatPlaceholder chatId={chatId} />
                </ReviewChatSurface>
              </div>
            </div>
          ))}
        </div>
      </AbortControllerProvider>
    </Theme>
  </Provider>,
);
