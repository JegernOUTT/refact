import { http, HttpResponse } from "msw";
import { expect, it } from "vitest";
import { applyChatEvent } from "../Chat/Thread/actions";
import {
  createDefaultChatState,
  render,
  waitFor,
} from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { BROWSER_SURFACE_KEY } from "./surfaceKey";
import { useProjectBrowserAutoOpen } from "./useProjectBrowserAutoOpen";
import { closeTab, selectBrowserSurfaceOpen } from "./workspaceSlice";

function Subscriber() {
  useProjectBrowserAutoOpen();
  return null;
}

it("verifies live migration signals once and respects a later workspace close", async () => {
  const chat = createDefaultChatState();
  const id = chat.current_thread_id;
  chat.threads[id].snapshot_received = true;
  let requests = 0;
  server.use(
    http.post("*/v1/browser/project/status", () => {
      requests += 1;
      return HttpResponse.json({
        project_id: "test-project",
        running: false,
        connected: false,
        tabs: [],
        migration: { state: "available" },
      });
    }),
  );
  const { store } = render(<Subscriber />, { preloadedState: { chat } });
  const result = (seq: string, extra: Record<string, unknown>) =>
    applyChatEvent({
      chat_id: id,
      seq,
      type: "message_added",
      index: 0,
      message: { role: "tool", tool_call_id: seq, content: "result", extra },
    });
  const hint = {
    browser_project: { open_browser_tab: true, decision_required: true },
  };
  store.dispatch(result("1", {}));
  expect(selectBrowserSurfaceOpen(store.getState())).toBe(false);
  store.dispatch(result("2", hint));
  await waitFor(() =>
    expect(selectBrowserSurfaceOpen(store.getState())).toBe(true),
  );
  store.dispatch(closeTab(BROWSER_SURFACE_KEY));
  store.dispatch(result("3", hint));
  expect(selectBrowserSurfaceOpen(store.getState())).toBe(false);
  expect(requests).toBe(1);
});
