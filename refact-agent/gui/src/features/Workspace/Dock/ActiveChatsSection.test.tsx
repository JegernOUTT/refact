import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { ChatActivity } from "../../../services/refact/trajectories";
import { server } from "../../../utils/mockServer";
import {
  createDefaultChatState,
  render,
  screen,
  waitFor,
} from "../../../utils/test-utils";
import { setBackendStatus } from "../../Connection";
import { setBrowserOnline } from "../../Connection/connectionSlice";
import { setApiKey, updateConfig } from "../../Config/configSlice";
import { setCurrentProjectInfo } from "../../Chat/currentProject";
import { openTab, setDockOpen, setDockSection } from "../workspaceSlice";
import { ActiveChatsSectionView } from "./ActiveChatsSection";
import { activeChatReasons, buildActiveChatEntries } from "./activeChatsModel";
import { Dock } from "./Dock";

const originalMatchMedia = window.matchMedia;

function activity(overrides: Partial<ChatActivity>): ChatActivity {
  return {
    id: "chat",
    title: "Chat",
    updated_at_ms: 1_000,
    session_state: null,
    recovery_state: null,
    schedule_count: 0,
    paused_schedule_count: 0,
    ...overrides,
  };
}

function mockNarrow(narrow: boolean) {
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: vi.fn(
      (query: string): MediaQueryList => ({
        matches: narrow && query === "(max-width: 767px)",
        media: query,
        onchange: null,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        addListener: vi.fn(),
        removeListener: vi.fn(),
        dispatchEvent: vi.fn(),
      }),
    ),
  });
}

afterEach(() => {
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: originalMatchMedia,
  });
  vi.restoreAllMocks();
});

describe("activeChatsModel", () => {
  it("maps engine states to semantic reasons", () => {
    expect(
      activeChatReasons(activity({ session_state: "generating" })),
    ).toEqual(["streaming"]);
    expect(
      activeChatReasons(activity({ session_state: "executing_tools" })),
    ).toEqual(["working"]);
    expect(
      activeChatReasons(activity({ recovery_state: "attempting" })),
    ).toEqual(["recovering"]);
    expect(activeChatReasons(activity({ recovery_state: "waiting" }))).toEqual([
      "recovering",
    ]);
    expect(
      activeChatReasons(activity({ recovery_state: "needs_action" })),
    ).toEqual(["needs_attention"]);
    expect(
      activeChatReasons(activity({ recovery_state: "exhausted" })),
    ).toEqual(["needs_attention"]);
    expect(
      activeChatReasons(
        activity({ schedule_count: 2, paused_schedule_count: 1 }),
      ),
    ).toEqual(["scheduled", "schedule_paused"]);
    expect(
      activeChatReasons(
        activity({ schedule_count: 1, paused_schedule_count: 1 }),
      ),
    ).toEqual(["schedule_paused"]);
    expect(
      activeChatReasons(
        activity({ session_state: "idle", recovery_state: "succeeded" }),
      ),
    ).toEqual([]);
  });

  it("keeps one row per chat, attention first, then most recent", () => {
    const entries = buildActiveChatEntries([
      activity({ id: "a", session_state: "generating", updated_at_ms: 5 }),
      activity({ id: "b", schedule_count: 1, updated_at_ms: 9 }),
      activity({ id: "c", recovery_state: "needs_action", updated_at_ms: 1 }),
      activity({
        id: "a",
        session_state: "generating",
        schedule_count: 1,
        updated_at_ms: 7,
      }),
      activity({ id: "idle", session_state: "idle", updated_at_ms: 99 }),
    ]);
    expect(entries.map((entry) => entry.id)).toEqual(["c", "a", "b"]);
    expect(entries[1].reasons).toEqual(["streaming", "scheduled"]);
  });
});

describe("ActiveChatsSectionView", () => {
  const noop = () => undefined;

  it("shows loading, empty, and error states with retry", async () => {
    const onRetry = vi.fn();
    const view = render(
      <ActiveChatsSectionView
        entries={[]}
        isError={false}
        isFetching={false}
        isLoading
        isStale={false}
        onOpenChat={noop}
        onRetry={onRetry}
      />,
    );
    expect(screen.getByLabelText("Loading active chats")).toBeInTheDocument();

    view.rerender(
      <ActiveChatsSectionView
        entries={[]}
        isError={false}
        isFetching={false}
        isLoading={false}
        isStale={false}
        onOpenChat={noop}
        onRetry={onRetry}
      />,
    );
    expect(screen.getByText("No active chats")).toBeInTheDocument();

    view.rerender(
      <ActiveChatsSectionView
        entries={[]}
        isError
        isFetching={false}
        isLoading={false}
        isStale={false}
        onOpenChat={noop}
        onRetry={onRetry}
      />,
    );
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Active chats unavailable",
    );
    await view.user.click(screen.getByRole("button", { name: "Retry" }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it("keeps last known rows visible with a truthful stale notice", () => {
    render(
      <ActiveChatsSectionView
        entries={buildActiveChatEntries([
          activity({ id: "a", title: "Alpha", session_state: "generating" }),
        ])}
        isError
        isFetching={false}
        isLoading={false}
        isStale
        onOpenChat={noop}
        onRetry={noop}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "Showing last known activity",
    );
    expect(
      screen.getByRole("button", { name: "Alpha — Streaming" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("labels offline staleness truthfully without a retry", () => {
    render(
      <ActiveChatsSectionView
        entries={buildActiveChatEntries([
          activity({ id: "a", title: "Alpha", session_state: "generating" }),
        ])}
        isError={false}
        isFetching={false}
        isLoading={false}
        isStale
        staleReason="offline"
        onOpenChat={noop}
        onRetry={noop}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "Showing last known activity; engine is offline.",
    );
    expect(screen.getByRole("status")).not.toHaveTextContent("refresh failed");
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
  });

  it("renders every reason label on a single chat row", async () => {
    const onOpenChat = vi.fn();
    const view = render(
      <ActiveChatsSectionView
        entries={buildActiveChatEntries([
          activity({
            id: "multi",
            title: "Multi",
            recovery_state: "attempting",
            schedule_count: 2,
            paused_schedule_count: 1,
          }),
        ])}
        isError={false}
        isFetching={false}
        isLoading={false}
        isStale={false}
        onOpenChat={onOpenChat}
        onRetry={noop}
      />,
    );
    const rows = screen.getAllByTestId("active-chat-row");
    expect(rows).toHaveLength(1);
    expect(rows[0]).toHaveTextContent("Recovering");
    expect(rows[0]).toHaveTextContent("Scheduled");
    expect(rows[0]).toHaveTextContent("Schedule paused");
    await view.user.click(rows[0]);
    expect(onOpenChat).toHaveBeenCalledWith("multi");
  });
});

describe("Active chats dock section", () => {
  it("never shows a previous project's activity after switching project, endpoint, or auth", async () => {
    mockNarrow(false);
    let releaseOld: () => void = () => undefined;
    const oldGate = new Promise<void>((resolve) => {
      releaseOld = resolve;
    });
    const seen: { auth: string | null; url: string }[] = [];
    server.use(
      http.get("*/v1/chats/activity", async ({ request }) => {
        const auth = request.headers.get("authorization");
        seen.push({ auth, url: request.url });
        if (seen.length === 1) {
          await oldGate;
          return HttpResponse.json({
            chats: [
              activity({
                id: "old",
                title: "Old project chat",
                session_state: "generating",
              }),
            ],
          });
        }
        return HttpResponse.json({
          chats: [
            activity({
              id: `new-${seen.length}`,
              title: `New context chat ${seen.length}`,
              schedule_count: 1,
            }),
          ],
        });
      }),
    );
    const view = render(<Dock />, {
      preloadedState: {
        config: {
          host: "web",
          dev: true,
          lspPort: 8001,
          apiKey: "key-a",
          themeProps: {},
        },
      },
    });
    view.store.dispatch(setCurrentProjectInfo({ name: "Project A" }));
    view.store.dispatch(setDockSection("activity"));
    view.store.dispatch(setDockOpen(true));
    await waitFor(() => expect(seen).toHaveLength(1));

    view.store.dispatch(setCurrentProjectInfo({ name: "Project B" }));
    expect(
      await screen.findByRole("button", {
        name: "New context chat 2 — Scheduled",
      }),
    ).toBeInTheDocument();
    releaseOld();
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(
      screen.queryByRole("button", { name: /Old project chat/ }),
    ).toBeNull();

    view.store.dispatch(setApiKey("key-b"));
    expect(
      await screen.findByRole("button", {
        name: "New context chat 3 — Scheduled",
      }),
    ).toBeInTheDocument();
    expect(seen[2].auth).toBe("Bearer key-b");
    expect(
      screen.queryByRole("button", { name: /New context chat 2/ }),
    ).toBeNull();

    view.store.dispatch(
      updateConfig({ dev: false, lspUrl: "http://127.0.0.1:8123" }),
    );
    expect(
      await screen.findByRole("button", {
        name: "New context chat 4 — Scheduled",
      }),
    ).toBeInTheDocument();
    expect(seen[3].url).toContain("127.0.0.1:8123");
    expect(
      screen.queryByRole("button", { name: /New context chat 3/ }),
    ).toBeNull();
  });

  it("never claims no active chats while offline without data", async () => {
    mockNarrow(false);
    server.use(http.get("*/v1/chats/activity", () => HttpResponse.error()));
    const view = render(<Dock />);
    view.store.dispatch(setBrowserOnline(false));
    view.store.dispatch(setDockSection("activity"));
    view.store.dispatch(setDockOpen(true));

    expect(await screen.findByText("Engine offline")).toBeInTheDocument();
    expect(screen.queryByText("No active chats")).toBeNull();
    expect(screen.queryByText("Active chats unavailable")).toBeNull();
  });

  it("does not label offline cached activity as a failed refresh", async () => {
    mockNarrow(false);
    server.use(
      http.get("*/v1/chats/activity", () =>
        HttpResponse.json({
          chats: [
            activity({
              id: "s",
              title: "Streamer",
              session_state: "generating",
            }),
          ],
        }),
      ),
    );
    const view = render(<Dock />);
    view.store.dispatch(setDockSection("activity"));
    view.store.dispatch(setDockOpen(true));
    await screen.findByRole("button", { name: "Streamer — Streaming" });

    view.store.dispatch(setBrowserOnline(false));
    expect(await screen.findByRole("status")).toHaveTextContent(
      "engine is offline",
    );
    expect(screen.getByRole("status")).not.toHaveTextContent("refresh failed");
    expect(
      screen.getByRole("button", { name: "Streamer — Streaming" }),
    ).toBeInTheDocument();
  });

  it("opens an unopened chat cold from the engine feed and closes the narrow Sheet", async () => {
    mockNarrow(true);
    const coldId = "cold-active-chat";
    let metadataReads = 0;
    let activityAuth: string | null = null;
    server.use(
      http.get("*/v1/chats/activity", ({ request }) => {
        activityAuth = request.headers.get("authorization");
        return HttpResponse.json({
          chats: [
            activity({
              id: coldId,
              title: "Cold streaming chat",
              session_state: "generating",
            }),
          ],
        });
      }),
      http.get(`*/v1/trajectories/${coldId}`, () => {
        metadataReads += 1;
        return HttpResponse.json({
          id: coldId,
          title: "Cold streaming chat",
          model: "",
          mode: "AGENT",
          tool_use: "agent",
        });
      }),
    );
    const chat = createDefaultChatState();
    const parentId = chat.current_thread_id;
    const view = render(<Dock />, {
      preloadedState: {
        chat,
        config: {
          host: "web",
          dev: true,
          lspPort: 8001,
          apiKey: "scoped-key",
          themeProps: {},
        },
      },
    });
    view.store.dispatch(setBackendStatus({ status: "online" }));
    view.store.dispatch(openTab(`chat:${parentId}`));
    view.store.dispatch(setDockSection("activity"));
    view.store.dispatch(setDockOpen(true));

    const row = await screen.findByRole("button", {
      name: "Cold streaming chat — Streaming",
    });
    expect(activityAuth).toBe("Bearer scoped-key");
    expect(view.store.getState().chat.threads[coldId]).toBeUndefined();
    await view.user.click(row);

    await waitFor(() =>
      expect(view.store.getState().chat.current_thread_id).toBe(coldId),
    );
    expect(metadataReads).toBe(1);
    expect(view.store.getState().workspace.dock?.open).toBe(false);
  });

  it("shows an error with retry that recovers on the next fetch", async () => {
    mockNarrow(false);
    let calls = 0;
    server.use(
      http.get("*/v1/chats/activity", () => {
        calls += 1;
        if (calls === 1) {
          return HttpResponse.json({ detail: "boom" }, { status: 500 });
        }
        return HttpResponse.json({
          chats: [
            activity({ id: "sched", title: "Nightly", schedule_count: 1 }),
          ],
        });
      }),
    );
    const view = render(<Dock />);
    view.store.dispatch(setDockSection("activity"));
    view.store.dispatch(setDockOpen(true));

    expect(
      await screen.findByText("Active chats unavailable"),
    ).toBeInTheDocument();
    await view.user.click(screen.getByRole("button", { name: "Retry" }));
    expect(
      await screen.findByRole("button", { name: "Nightly — Scheduled" }),
    ).toBeInTheDocument();
  });
});
