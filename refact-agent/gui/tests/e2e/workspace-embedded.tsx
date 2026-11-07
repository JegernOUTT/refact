import type { setUpStore } from "../../src/app/store";
import { makeSurfaceKey } from "../../src/features/Workspace/surfaceKey";
import {
  openTab,
  setActiveTab,
  splitTab,
} from "../../src/features/Workspace/workspaceSlice";

const params = new URLSearchParams(location.search);
const split = params.has("split");
const kind = params.get("surface") === "task" ? "task" : "buddy";
params.set("route", "chat-dnd");
params.set("toolbar_alignment", "1");
params.set("unsplit", "1");
history.replaceState(null, "", `?${params}`);
await import("./route-showcase");
const store = (
  window as unknown as {
    __toolbarAlignmentStore: ReturnType<typeof setUpStore>;
  }
).__toolbarAlignmentStore;
const fetchSeed = window.fetch.bind(window);
const json = (data: unknown) =>
  new Response(JSON.stringify(data), {
    headers: { "Content-Type": "application/json" },
  });
window.fetch = async (input, init) => {
  const url = new URL(
    input instanceof Request ? input.url : String(input),
    location.origin,
  );
  if (url.pathname === "/v1/tasks/embedded")
    return json({
      meta: {
        id: "embedded",
        name: "Embedded width regression",
        status: "planning",
        created_at: "2026-06-07",
        updated_at: "2026-06-07",
        cards_total: 0,
        cards_done: 0,
        cards_failed: 0,
        agents_active: 0,
      },
    });
  if (url.pathname === "/v1/tasks/embedded/board")
    return json({ schema_version: 1, rev: 1, columns: [], cards: [] });
  if (url.pathname.includes("/v1/tasks/embedded/trajectories"))
    return json(
      url.pathname.endsWith("/planner")
        ? [
            {
              id: "showcase-chat",
              title: "Embedded loaded planner",
              created_at: "2026-06-07",
              updated_at: "2026-06-07",
              mode: "task_planner",
            },
          ]
        : [],
    );
  return fetchSeed(input, init);
};
const key = makeSurfaceKey(kind, kind === "task" ? "embedded" : "home");
store.dispatch(openTab(key));
store.dispatch(setActiveTab(key));

if (split)
  store.dispatch(
    splitTab({
      tabId: store.getState().workspace.activeTabId!,
      dir: "row",
      surfaceKey: makeSurfaceKey("chat", "showcase-chat-b"),
    }),
  );
