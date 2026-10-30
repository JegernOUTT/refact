import { readFileSync } from "node:fs";

import { beforeEach, describe, expect, test, vi } from "vitest";
import { act } from "react-dom/test-utils";
import { screen, waitFor } from "@testing-library/react";
import { delay, http, HttpResponse } from "msw";

import type { BackgroundAgentSummary } from "../../services/refact";
import {
  backgroundAgentsApi,
  receiveAgentRosterEvent,
  rejectAgentRosterStream,
} from "../../services/refact/backgroundAgents";
import { applyChatEvent } from "../Chat/Thread";
import { agentsPanelSlice, selectAgentRosterSource } from "./agentsPanelSlice";
import { setApiKey, updateConfig } from "../Config/configSlice";
import { setCurrentProjectInfo } from "../Chat/currentProject";
import { clearError } from "../Errors/errorsSlice";
import { clearInformation } from "../Errors/informationSlice";
import { createDefaultChatState, render } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { AgentsSection } from "./AgentsPanel";

const chatId = "parent-chat";

const agentsPanelCss = readFileSync(
  "src/features/AgentsPanel/AgentsPanel.module.css",
  "utf8",
);
const globalTokensCss = [
  readFileSync("src/styles/tokens.css", "utf8"),
  readFileSync("src/styles/motion.css", "utf8"),
].join("\n");

const LOCALLY_DEFINED_CUSTOM_PROPS = ["--rf-agent-row-indent"];

function undefinedTokensIn(css: string): string[] {
  const used = new Set(
    [...css.matchAll(/var\((--rf-[a-z0-9-]+)/gu)].map((match) => match[1]),
  );
  return [...used].filter(
    (token) =>
      !LOCALLY_DEFINED_CUSTOM_PROPS.includes(token) &&
      !new RegExp(`${token}\\s*:`, "u").test(globalTokensCss) &&
      !new RegExp(`${token}\\s*:`, "u").test(css),
  );
}

function agent(
  id: string,
  parentChatId: string,
  childChatId: string | null,
  overrides: Partial<BackgroundAgentSummary> = {},
): BackgroundAgentSummary {
  return {
    agent_id: id,
    parent_chat_id: parentChatId,
    child_chat_id: childChatId,
    kind: "subagent",
    status: "running",
    title: id,
    progress: null,
    step_count: 0,
    last_activity: "2026-08-31T10:00:00Z",
    target_files: [],
    edited_files: [],
    diff_summary: null,
    conflict_summary: null,
    result_summary: null,
    error: null,
    started_at: null,
    finished_at: null,
    change_seq: 1,
    ...overrides,
  };
}

function panelState(agents: BackgroundAgentSummary[]) {
  const chat = createDefaultChatState();
  const [sourceRuntime] = Object.values(chat.threads);
  chat.current_thread_id = chatId;
  chat.open_thread_ids = [chatId];
  chat.threads = {
    [chatId]: {
      ...sourceRuntime,
      thread: { ...sourceRuntime.thread, id: chatId },
      background_agents: Object.fromEntries(
        agents.map((item) => [item.agent_id, item]),
      ),
    },
  };
  return {
    chat,
    config: { host: "ide" as const, lspPort: 8001, themeProps: {} },
    agentsPanel: {
      ...agentsPanelSlice.getInitialState(),
      authorized: true,
      liveAgents: Object.fromEntries(
        agents.map((item) => [item.agent_id, item]),
      ),
    },
  };
}

describe("AgentsSection", () => {
  beforeEach(() => {
    server.use(http.get("*/v1/background-agents", () => HttpResponse.json([])));
  });

  test("prompts for a chat when the dock has no focused chat", () => {
    render(<AgentsSection chatId={null} />);

    expect(
      screen.getByText("Open a chat to see its agents"),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("radiogroup", { name: "Agents filter" }),
    ).not.toBeInTheDocument();
  });

  test("shows loading before an empty state when the focused chat has no agents", async () => {
    server.use(
      http.get("*/v1/background-agents", async () => {
        await delay(50);
        return HttpResponse.json([]);
      }),
    );
    render(<AgentsSection chatId={chatId} />, {
      preloadedState: panelState([]),
    });

    expect(
      screen.getByRole("status", { name: "Loading agents…" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("No agents")).not.toBeInTheDocument();
    expect(await screen.findByText("No agents")).toBeInTheDocument();
    expect(screen.getByText("No agents are running.")).toBeInTheDocument();
  });

  test("reconciles REST and live rows, counts, and usage by sequence with terminal ties", async () => {
    const running = agent("worker", chatId, "worker-chat", {
      title: "Worker",
      current_tool: "cat",
      tokens_used: 100,
    });
    const restOnly = agent("rest-only", chatId, null, { tokens_used: 200 });
    const liveDone = agent("live-done", chatId, null, { status: "completed" });
    const restDone = agent("rest-done", chatId, null);
    server.use(
      http.get("*/v1/background-agents", () =>
        HttpResponse.json([
          running,
          restOnly,
          { ...liveDone, status: "running" },
          { ...restDone, status: "completed" },
        ]),
      ),
    );
    const { user, store } = render(<AgentsSection chatId={chatId} />, {
      preloadedState: panelState([running, liveDone, restDone]),
    });

    expect(await screen.findByText("rest-only")).toBeInTheDocument();
    expect(
      screen.getByRole("radio", { name: "Active (2)" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "All (4)" })).toBeInTheDocument();
    expect(screen.getByTestId("agents-aggregate-usage")).toHaveTextContent(
      "300 tokens",
    );
    act(() => {
      store.dispatch(
        receiveAgentRosterEvent(selectAgentRosterSource(store.getState()), {
          type: "background_agent_updated",
          chat_id: chatId,
          seq: "1",
          agent: {
            ...running,
            status: "completed",
            change_seq: 2,
            tokens_used: 800,
          },
        }),
      );
    });
    expect(
      screen.getByRole("radio", { name: "Active (1)" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("Worker")).not.toBeInTheDocument();
    expect(screen.getByTestId("agents-aggregate-usage")).toHaveTextContent(
      "1.0k tokens",
    );
    await user.click(screen.getByRole("radio", { name: /^All/ }));
    expect(screen.getByText("Worker")).toBeInTheDocument();
    expect(screen.getAllByLabelText("completed")).toHaveLength(3);
    expect(store.getState().agentsPanel.tab).toBe("all");
    expect(screen.queryByText("cat")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Message Worker")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Cancel Worker")).not.toBeInTheDocument();
  });

  test("does not carry REST rows or expanded details into another chat or revive stale cached records on return", async () => {
    const worker = agent("worker", chatId, null, {
      progress: "Old chat progress",
    });
    const otherChat = "other-chat";
    server.use(
      http.get("*/v1/background-agents", async ({ request }) => {
        if (new URL(request.url).searchParams.get("chat_id") === otherChat) {
          await delay(50);
          return HttpResponse.json([]);
        }
        return HttpResponse.json([worker]);
      }),
    );
    const { user, store, rerender } = render(
      <AgentsSection chatId={chatId} />,
      {
        preloadedState: panelState([]),
      },
    );
    await screen.findByText("worker");
    await user.click(screen.getByLabelText("Expand worker"));
    rerender(<AgentsSection chatId={otherChat} />);
    expect(screen.queryByText("worker")).not.toBeInTheDocument();
    expect(screen.queryByText("Old chat progress")).not.toBeInTheDocument();
    await screen.findByText("No agents");
    act(() => {
      store.dispatch(
        receiveAgentRosterEvent(selectAgentRosterSource(store.getState()), {
          type: "background_agent_updated",
          chat_id: chatId,
          seq: "1",
          agent: { ...worker, status: "completed", change_seq: 2 },
        }),
      );
    });
    rerender(<AgentsSection chatId={chatId} />);
    expect(
      screen.getByRole("radio", { name: "Active (0)" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("worker")).not.toBeInTheDocument();
    await user.click(screen.getByRole("radio", { name: /^All/ }));
    expect(await screen.findByLabelText("Expand worker")).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(screen.queryByLabelText("Cancel worker")).not.toBeInTheDocument();
  });

  test("shows fetch errors and retry without an empty-state claim, then retains cached rows after a failed refresh", async () => {
    let fail = true;
    server.use(
      http.get("*/v1/background-agents", () =>
        fail
          ? HttpResponse.json({ detail: "Unavailable" }, { status: 503 })
          : HttpResponse.json([agent("cached", chatId, null)]),
      ),
    );
    const { user, store } = render(<AgentsSection chatId={chatId} />, {
      preloadedState: panelState([]),
    });
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Agent data is unavailable",
    );
    expect(screen.queryByText("No agents")).not.toBeInTheDocument();
    fail = false;
    await user.click(screen.getByRole("button", { name: "Retry" }));
    await screen.findByText("cached");
    fail = true;
    act(() => {
      store.dispatch(
        backgroundAgentsApi.util.invalidateTags(["BackgroundAgents"]),
      );
    });
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Showing last known agent data",
    );
    expect(screen.getByText("cached")).toBeInTheDocument();
    expect(screen.queryByText("No agents")).not.toBeInTheDocument();
    fail = false;
    await user.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() =>
      expect(screen.queryByRole("alert")).not.toBeInTheDocument(),
    );
    expect(screen.getByText("cached")).toBeInTheDocument();
    act(() => {
      store.dispatch(
        rejectAgentRosterStream(
          selectAgentRosterSource(store.getState()),
          new Error("SSE idle timeout"),
        ),
      );
    });
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Showing last known agent data",
    );
    expect(screen.getByText("cached")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() =>
      expect(screen.queryByRole("alert")).not.toBeInTheDocument(),
    );
  });

  test("fences endpoint, token and project switches on the same chat, including old live receipts and controls", async () => {
    const worker = agent("private-worker", chatId, "private-child", {
      tokens_used: 900,
      cost_usd: 7,
    });
    let withheld = false;
    const requests: { url: string; authorization: string | null }[] = [];
    server.use(
      http.get("*/v1/background-agents", ({ request }) => {
        requests.push({
          url: request.url,
          authorization: request.headers.get("authorization"),
        });
        return withheld
          ? HttpResponse.json({ code: "scope_denied" }, { status: 403 })
          : HttpResponse.json([worker]);
      }),
    );
    const onNavigate = vi.fn();
    const { user, store } = render(
      <AgentsSection chatId={chatId} onNavigate={onNavigate} />,
      { preloadedState: panelState([]) },
    );
    await screen.findByText("private-worker");
    const oldSource = selectAgentRosterSource(store.getState());
    const oldTitle = screen.getByRole("button", { name: "private-worker" });
    const oldCancel = screen.getByLabelText("Cancel private-worker");
    withheld = true;
    act(() => {
      store.dispatch(updateConfig({ lspUrl: "http://127.0.0.1:8124" }));
    });
    expect(screen.queryByText("private-worker")).not.toBeInTheDocument();
    expect(screen.getByTestId("agents-aggregate-usage")).not.toHaveTextContent(
      "$7",
    );
    await user.click(oldTitle);
    await user.click(oldCancel);
    expect(onNavigate).not.toHaveBeenCalled();
    expect(screen.queryByText("Cancel subtree")).not.toBeInTheDocument();
    await screen.findByRole("alert");
    withheld = false;
    await user.click(screen.getByRole("button", { name: "Retry" }));
    await screen.findByText("private-worker");
    withheld = true;
    act(() => {
      store.dispatch(setApiKey("new-secret-principal"));
    });
    expect(screen.queryByText("private-worker")).not.toBeInTheDocument();
    await screen.findByRole("alert");
    act(() => {
      store.dispatch(
        receiveAgentRosterEvent(oldSource, {
          type: "background_agent_updated",
          chat_id: chatId,
          seq: "200",
          agent: { ...worker, change_seq: 200 },
        }),
      );
      store.dispatch(
        applyChatEvent({
          type: "background_agent_updated",
          chat_id: chatId,
          seq: "201",
          agent: worker,
        }),
      );
    });
    expect(screen.queryByText("private-worker")).not.toBeInTheDocument();
    withheld = false;
    await user.click(screen.getByRole("button", { name: "Retry" }));
    await screen.findByText("private-worker");
    withheld = true;
    act(() => {
      store.dispatch(
        setCurrentProjectInfo({
          name: "other-project",
          projectRoot: "/other-project",
        }),
      );
    });
    expect(screen.queryByText("private-worker")).not.toBeInTheDocument();
    expect(JSON.stringify(store.getState().backgroundAgentsApi)).not.toContain(
      "new-secret-principal",
    );
    expect(store.getState().chat.threads[chatId]).toBeUndefined();
    await user.click(oldTitle);
    await user.click(oldCancel);
    expect(onNavigate).not.toHaveBeenCalled();
    expect(screen.queryByText("Cancel subtree")).not.toBeInTheDocument();
    await screen.findByRole("alert");
    expect(requests.at(-1)?.authorization).toBe("Bearer new-secret-principal");
    expect(
      new URL(requests.at(-1)?.url ?? "").searchParams.get("chat_id"),
    ).toBe(chatId);
  });

  test("denial clears REST and live rows and old receipts cannot restore them before or after fresh recovery", async () => {
    const worker = agent("rest-private", chatId, null, { cost_usd: 5 });
    const live = agent("live-private", chatId, null, { cost_usd: 3 });
    let denied = false;
    server.use(
      http.get("*/v1/background-agents", () =>
        denied
          ? HttpResponse.json({ code: "policy_unavailable" }, { status: 503 })
          : HttpResponse.json([worker]),
      ),
    );
    const { user, store } = render(<AgentsSection chatId={chatId} />, {
      preloadedState: panelState([]),
    });
    await screen.findByText("rest-private");
    const source = selectAgentRosterSource(store.getState());
    act(() => {
      store.dispatch(
        receiveAgentRosterEvent(source, {
          type: "background_agent_updated",
          chat_id: chatId,
          seq: "1",
          agent: live,
        }),
      );
    });
    expect(screen.getByText("live-private")).toBeInTheDocument();
    denied = true;
    act(() => {
      store.dispatch(
        backgroundAgentsApi.util.invalidateTags(["BackgroundAgents"]),
      );
    });
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Agent data is hidden",
    );
    act(() => {
      store.dispatch(
        receiveAgentRosterEvent(source, {
          type: "background_agent_updated",
          chat_id: chatId,
          seq: "2",
          agent: { ...live, change_seq: 2 },
        }),
      );
    });
    expect(screen.queryByText("rest-private")).not.toBeInTheDocument();
    expect(screen.queryByText("live-private")).not.toBeInTheDocument();
    expect(screen.getByTestId("agents-aggregate-usage")).toHaveTextContent(
      "0 tokens",
    );
    denied = false;
    await user.click(screen.getByRole("button", { name: "Retry" }));
    await screen.findByText("rest-private");
    act(() => {
      store.dispatch(
        receiveAgentRosterEvent(source, {
          type: "background_agent_updated",
          chat_id: chatId,
          seq: "3",
          agent: { ...live, change_seq: 3 },
        }),
      );
    });
    expect(screen.queryByText("live-private")).not.toBeInTheDocument();
    act(() => {
      store.dispatch(
        receiveAgentRosterEvent(selectAgentRosterSource(store.getState()), {
          type: "background_agent_updated",
          chat_id: chatId,
          seq: "4",
          agent: { ...live, change_seq: 4 },
        }),
      );
    });
    expect(screen.getByText("live-private")).toBeInTheDocument();
  });

  test("renders a three-level hierarchy and aggregate usage", async () => {
    const root = agent("root", chatId, "child-chat", {
      title: "Root agent",
      tokens_used: 1200,
      cost_usd: 0.34,
    });
    const child = agent("child", "child-chat", "grandchild-chat", {
      title: "Child agent",
      tokens_used: 800,
    });
    const grandchild = agent("grandchild", "grandchild-chat", null, {
      title: "Grandchild agent",
    });
    const { user } = render(<AgentsSection chatId={chatId} />, {
      preloadedState: panelState([root, child, grandchild]),
    });

    expect(await screen.findByText("Root agent")).toBeInTheDocument();
    expect(screen.getByTestId("agents-aggregate-usage")).toHaveTextContent(
      "2.0k tokens · $0.34",
    );
    await user.click(screen.getByLabelText("Expand Root agent"));
    expect(screen.getByText("Child agent")).toBeInTheDocument();
    await user.click(screen.getByLabelText("Expand Child agent"));
    expect(screen.getByText("Grandchild agent")).toBeInTheDocument();
    const collapse = screen.getByLabelText("Collapse Root agent");
    await user.click(collapse);
    expect(collapse).toHaveAttribute("aria-expanded", "false");
    expect(
      screen.queryByRole("button", { name: "Grandchild agent" }),
    ).not.toBeInTheDocument();
  });

  test("expands leaf details with the keyboard independently of navigation and retains summaries and every file", async () => {
    const onNavigate = vi.fn();
    const worker = agent("leaf", chatId, "leaf-chat", {
      progress: "Checking the source",
      goal_summary: "Repair the sidebar",
      error: "Safe failure detail",
      result_summary: "Result remains inspectable",
      diff_summary: "Five files changed",
      conflict_summary: "Merge conflict detail",
      target_files: ["one.ts", "two.ts", "three.ts", "four.ts", "five.ts"],
      questions: [{ id: "q1", text: "Which model?" }],
      started_at: "2026-08-31T09:00:00Z",
      finished_at: "2026-08-31T11:00:00Z",
      merge_status: "conflict",
    });
    const { user } = render(
      <AgentsSection chatId={chatId} onNavigate={onNavigate} />,
      {
        preloadedState: panelState([worker]),
      },
    );
    const expand = await screen.findByRole("button", { name: "Expand leaf" });
    expand.focus();
    await user.keyboard("{Enter}");
    expect(onNavigate).not.toHaveBeenCalled();
    expect(screen.getByText("Checking the source")).toBeInTheDocument();
    expect(screen.getByText("Repair the sidebar")).toBeInTheDocument();
    expect(screen.getByText("Safe failure detail")).toBeInTheDocument();
    expect(screen.getByText("Result remains inspectable")).toBeInTheDocument();
    expect(screen.getByText("Five files changed")).toBeInTheDocument();
    expect(screen.getByText("Merge conflict detail")).toBeInTheDocument();
    expect(screen.getByText("five.ts")).toBeInTheDocument();
    expect(screen.getByText("Which model?")).toBeInTheDocument();
    expect(screen.getByText("Awaiting answer")).toBeInTheDocument();
    expect(screen.getByText(/^Started /)).toBeInTheDocument();
    expect(screen.getByText(/^Active \d/)).toBeInTheDocument();
    expect(screen.getByText(/^Finished /)).toBeInTheDocument();
    expect(screen.getByText("conflict")).toBeInTheDocument();
    const details = document.getElementById(
      expand.getAttribute("aria-controls") ?? "",
    );
    await user.keyboard("{Enter}");
    expect(details).toHaveAttribute("aria-hidden", "true");
    expect(details).toHaveAttribute("inert");
    await user.click(screen.getByRole("button", { name: "leaf" }));
    expect(onNavigate).toHaveBeenCalledExactlyOnceWith("leaf-chat");
  });

  test("navigates once from the title, posts a message, copies agent details, and confirms cancellation", async () => {
    const onNavigate = vi.fn();
    let cancelBody: unknown;
    let messageBody: unknown;
    server.use(
      http.post("*/v1/background-agents/worker/cancel", async ({ request }) => {
        cancelBody = await request.json();
        return HttpResponse.json({});
      }),
      http.post(
        "*/v1/background-agents/worker/message",
        async ({ request }) => {
          messageBody = await request.json();
          return HttpResponse.json({});
        },
      ),
    );
    const worker = agent("worker", chatId, "worker-chat", {
      title: "Worker",
      worktree_branch: "refact/task/worker",
    });
    const { user, store } = render(
      <AgentsSection chatId={chatId} onNavigate={onNavigate} />,
      {
        preloadedState: panelState([worker]),
      },
    );

    await user.click(await screen.findByRole("button", { name: "Worker" }));
    expect(onNavigate).toHaveBeenCalledExactlyOnceWith("worker-chat");
    await user.click(screen.getByLabelText("Copy agent ID"));
    expect(
      screen.getByLabelText("Copy agent ID").querySelector("svg"),
    ).toBeInTheDocument();
    await user.click(screen.getByLabelText("Copy branch"));
    expect(
      screen.getByLabelText("Copy branch").querySelector("svg"),
    ).toBeInTheDocument();
    act(() => {
      store.dispatch(clearInformation());
    });
    await user.click(screen.getByLabelText("Message Worker"));
    await user.type(
      screen.getByLabelText("Message for Worker"),
      "Please check tests",
    );
    await user.click(screen.getByLabelText("Send message"));
    await waitFor(() => {
      expect(messageBody).toEqual({
        chat_id: chatId,
        text: "Please check tests",
      });
      expect(store.getState().information.message).toBe(
        "Message accepted for delivery to agent.",
      );
    });
    act(() => {
      store.dispatch(clearInformation());
    });
    await user.click(screen.getByLabelText("Cancel Worker"));
    expect(screen.getByText("Cancel this agent subtree?")).toBeInTheDocument();
    expect(cancelBody).toBeUndefined();
    await user.click(screen.getByText("Cancel subtree"));
    await waitFor(() => {
      expect(cancelBody).toEqual({ chat_id: chatId, subtree: true });
      expect(store.getState().information.message).toBe(
        "Cancellation requested for this agent subtree.",
      );
    });
    expect(onNavigate).toHaveBeenCalledTimes(1);
    expect(screen.getByLabelText("Cancel Worker")).toBeInTheDocument();
  });

  test("retains backend mutation errors and message drafts without implying successful control", async () => {
    const token = "a2-ui-fixture-credential";
    const opaque = "Q7bR9nVm4kXa2cDf8pLs6tHy3wUz5eGj";
    const messageDetail = `Message not accepted: ownership changed ${token} ${opaque}`;
    const cancelDetail = `Cancellation denied: ownership changed ${token} ${opaque}`;
    expect(token.length).toBeGreaterThan(0);
    expect(messageDetail).toContain(token);
    expect(cancelDetail).toContain(token);
    server.use(
      http.post("*/v1/background-agents/worker/message", () =>
        HttpResponse.json(
          { detail: messageDetail, nested: { credential: token } },
          { status: 409 },
        ),
      ),
      http.post("*/v1/background-agents/worker/cancel", () =>
        HttpResponse.json(
          { error: cancelDetail, nested: { credential: token } },
          { status: 401 },
        ),
      ),
    );
    const initial = panelState([agent("worker", chatId, null)]);
    const { user, store } = render(<AgentsSection chatId={chatId} />, {
      preloadedState: {
        ...initial,
        config: { ...initial.config, apiKey: token },
      },
    });
    await user.click(await screen.findByLabelText("Message worker"));
    await user.type(
      screen.getByLabelText("Message for worker"),
      "Keep this draft",
    );
    await user.click(screen.getByLabelText("Send message"));
    await waitFor(() => {
      expect(store.getState().error.message).toBe(
        "Message not accepted: ownership changed [REDACTED] [REDACTED]",
      );
    });
    expect(screen.getByLabelText("Message for worker")).toHaveValue(
      "Keep this draft",
    );
    expect(store.getState().information.message).toBeNull();
    expect(JSON.stringify(store.getState().backgroundAgentsApi)).not.toContain(
      token,
    );
    expect(store.getState().error.message).not.toContain(token);
    expect(store.getState().error.message).not.toContain(opaque);
    expect(document.body.textContent).not.toContain(token);
    act(() => {
      store.dispatch(clearError());
    });
    await user.click(screen.getByLabelText("Cancel worker"));
    await user.click(screen.getByText("Cancel subtree"));
    await waitFor(() => {
      expect(store.getState().error.message).toBe(
        "Cancellation denied: ownership changed [REDACTED] [REDACTED]",
      );
      expect(screen.getByRole("alert")).toHaveTextContent(
        "Agent data is hidden",
      );
    });
    expect(store.getState().information.message).toBeNull();
    expect(screen.queryByLabelText("Cancel worker")).not.toBeInTheDocument();
    expect(JSON.stringify(store.getState().backgroundAgentsApi)).not.toContain(
      token,
    );
    expect(store.getState().error.message).not.toContain(token);
    expect(store.getState().error.message).not.toContain(opaque);
    expect(document.body.textContent).not.toContain(token);
  });

  test("hides message and cancel actions for terminal agents", async () => {
    const done = agent("done", chatId, "done-chat", {
      status: "completed",
      title: "Finished agent",
    });
    const { user } = render(<AgentsSection chatId={chatId} />, {
      preloadedState: panelState([done]),
    });

    await user.click(screen.getByRole("radio", { name: /^All/ }));
    expect(await screen.findByText("Finished agent")).toBeInTheDocument();
    expect(
      screen.queryByLabelText("Message Finished agent"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByLabelText("Cancel Finished agent"),
    ).not.toBeInTheDocument();
    expect(screen.getByLabelText("Copy agent ID")).toBeInTheDocument();
  });

  test("strips agent-kind prefixes and markdown emphasis from displayed titles", async () => {
    const worker = agent("worker", chatId, "worker-chat", {
      title: "Subagent: **Fix the flaky test**",
    });
    render(<AgentsSection chatId={chatId} />, {
      preloadedState: panelState([worker]),
    });

    const title = await screen.findByRole("button", {
      name: "Fix the flaky test",
    });
    expect(title).toBeInTheDocument();
    expect(title).toHaveAttribute("title", "Subagent: **Fix the flaky test**");
    expect(
      screen.getByLabelText("Message Subagent: **Fix the flaky test**"),
    ).toBeInTheDocument();
  });

  test("formats aggregate usage above a million tokens with an M suffix", async () => {
    const heavy = agent("heavy", chatId, null, {
      title: "Heavy agent",
      tokens_used: 4_996_000,
    });
    render(<AgentsSection chatId={chatId} />, {
      preloadedState: panelState([heavy]),
    });

    await waitFor(() =>
      expect(screen.getByTestId("agents-aggregate-usage")).toHaveTextContent(
        "5.0M tokens",
      ),
    );
    expect(screen.getByText("5.0M")).toBeInTheDocument();
  });

  test("hides terminal tool state and costs without a positive finite value", async () => {
    const done = agent("done", chatId, "done-chat", {
      status: "completed",
      current_tool: "shell",
      cost_usd: 0,
      title: "Finished agent",
    });
    const { user } = render(<AgentsSection chatId={chatId} />, {
      preloadedState: panelState([done]),
    });

    await user.click(screen.getByRole("radio", { name: /^All/ }));
    expect(screen.queryByText("shell")).not.toBeInTheDocument();
    expect(screen.getByTestId("agents-aggregate-usage")).not.toHaveTextContent(
      "$",
    );
  });

  test("reserves no chevron slot on leaf rows and overlays row actions", () => {
    expect(agentsPanelCss).not.toMatch(/\.expandSpacer/u);
    expect(agentsPanelCss).toMatch(
      /\.nodeActions\s*\{[\s\S]*position: absolute[\s\S]*pointer-events: none/u,
    );
    expect(agentsPanelCss).toMatch(
      /@media \(hover: none\), \(pointer: coarse\)[\s\S]*\.nodeActions[\s\S]*pointer-events: auto/u,
    );
  });

  test("honours reduced motion through both the media query and the html attribute", () => {
    expect(agentsPanelCss).toMatch(
      /@media \(prefers-reduced-motion: reduce\)[\s\S]*\.nodeRow,[\s\S]*transition: none/u,
    );
    expect(agentsPanelCss).toMatch(
      /html\[data-reduced-motion="on"\] \.nodeRow/u,
    );
  });

  test("lets the dock own the panel chrome instead of positioning itself", () => {
    expect(agentsPanelCss).toMatch(/\.panel\s*\{[\s\S]*flex: 1 1 auto/u);
    expect(agentsPanelCss).not.toMatch(/\.panel\[data-state="closed"\]/u);
    expect(agentsPanelCss).not.toMatch(/--rf-agents-panel-w/u);
    expect(agentsPanelCss).not.toMatch(/position: fixed/u);
  });

  test("styles the agents module with defined design tokens only", () => {
    expect(undefinedTokensIn(agentsPanelCss)).toEqual([]);
  });
});
