import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";
import { setUpStore } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { buildDisplayItems } from "../../src/components/ChatContent/ChatContentDisplayItems";
import { ToolJobResult } from "../../src/components/ChatContent/ToolCard/ToolJobResult";
import { EventRow } from "../../src/components/ChatContent/EventRow/EventRow";
import { restoreChat, selectThreadById } from "../../src/features/Chat/Thread";
import type { ChatMessages, ToolCall } from "../../src/services/refact/types";
import "../../src/lib/render/web.css";

const png =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=";
const calls: ToolCall[] = [
  {
    id: "read-call",
    index: 0,
    function: {
      name: "app",
      arguments: '{"action":"read_file","path":"ui/index.html"}',
    },
  },
  {
    id: "capture-call",
    index: 1,
    function: { name: "app_screenshot", arguments: '{"action":"capture"}' },
  },
];

const completion = (call: ToolCall, content: unknown) => ({
  role: "event",
  content,
  message_id: `tool_job:${call.id}:2:0`,
  event: {
    subkind: "system_notice",
    source: "tool_job.completed",
    payload: { actor: "tool_completion", job_id: call.id },
  },
  tool_job_result: {
    actor: "tool_completion",
    job_id: call.id,
    revision: 2,
    original_role: "tool",
    tool_call_id: call.id,
    tool_name: call.function.name,
  },
  delivery: { source: "tool_job.completed", id: `tool_job:${call.id}:2` },
});

const store = setUpStore();
store.dispatch(
  restoreChat({
    id: "app-render-regression",
    title: "App rendering regression",
    messages: [
      { role: "assistant", content: "", tool_calls: calls },
      completion(
        calls[0],
        JSON.stringify({
          revision: "resource-revision",
          path: "ui/index.html",
          content:
            '<!doctype html><html><body><script>const secret={"host_view_token":"HIDDEN_TOKEN"}</script><h1>Sample app</h1></body></html>',
        }),
      ),
      completion(calls[1], [
        {
          m_type: "text",
          m_content: '{"revision":"capture-revision","status":"ok"}',
        },
        { m_type: "image/png", m_content: png },
      ]),
      {
        role: "event",
        message_id: "image-notice",
        content: [
          { m_type: "text", m_content: "Multimodal event notice" },
          { m_type: "image/png", m_content: png },
        ],
        event: { subkind: "system_notice", source: "fixture.notice" },
      },
    ] as ChatMessages,
  }),
);
const items = buildDisplayItems(
  selectThreadById(store.getState(), "app-render-regression")?.messages ?? [],
  false,
);
const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <Provider store={store}>
      <Theme>
        <main>
          {items.map((item) =>
            item.type === "tool_job_result" ? (
              <ToolJobResult key={item.key} result={item.result} />
            ) : item.type === "event" ? (
              <EventRow key={item.key} event={item.event} />
            ) : null,
          )}
        </main>
      </Theme>
    </Provider>,
  );
}
