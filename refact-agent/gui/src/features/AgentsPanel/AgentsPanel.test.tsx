import { readFileSync } from "node:fs";

import { beforeEach, describe, expect, test, vi } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import { http, HttpResponse } from "msw";

import type { BackgroundAgentSummary } from "../../services/refact";
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
  return { chat };
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

  test("shows an empty state when the focused chat has no agents", () => {
    render(<AgentsSection chatId={chatId} />, {
      preloadedState: panelState([]),
    });

    expect(screen.getByText("No agents")).toBeInTheDocument();
    expect(screen.getByText("No agents are running.")).toBeInTheDocument();
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

    expect(screen.getByText("Root agent")).toBeInTheDocument();
    expect(screen.getByTestId("agents-aggregate-usage")).toHaveTextContent(
      "2.0k tokens · $0.34",
    );
    await user.click(screen.getByLabelText("Expand Root agent"));
    expect(screen.getByText("Child agent")).toBeInTheDocument();
    await user.click(screen.getByLabelText("Expand Child agent"));
    expect(screen.getByText("Grandchild agent")).toBeInTheDocument();
  });

  test("filters terminal agents from the active tab and restores them in all", async () => {
    const running = agent("running", chatId, null, { title: "Running agent" });
    const done = agent("done", chatId, null, {
      status: "completed",
      title: "Finished agent",
    });
    const { user, store } = render(<AgentsSection chatId={chatId} />, {
      preloadedState: panelState([running, done]),
    });

    expect(
      screen.getByRole("radio", { name: "Active (1)" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "All (2)" })).toBeInTheDocument();
    expect(screen.getByText("Running agent")).toBeInTheDocument();
    expect(screen.queryByText("Finished agent")).not.toBeInTheDocument();

    await user.click(screen.getByRole("radio", { name: /^All/ }));
    expect(screen.getByText("Finished agent")).toBeInTheDocument();
    expect(store.getState().agentsPanel.tab).toBe("all");
  });

  test("navigates from a row, posts a message, copies agent details, and confirms cancellation", async () => {
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
    const { user } = render(
      <AgentsSection chatId={chatId} onNavigate={onNavigate} />,
      {
        preloadedState: panelState([worker]),
      },
    );

    await user.click(screen.getByTestId("agent-row-worker"));
    expect(onNavigate).toHaveBeenCalledWith("worker-chat");
    await user.click(screen.getByLabelText("Copy agent ID"));
    expect(
      screen.getByLabelText("Copy agent ID").querySelector("svg"),
    ).toBeInTheDocument();
    await user.click(screen.getByLabelText("Copy branch"));
    expect(
      screen.getByLabelText("Copy branch").querySelector("svg"),
    ).toBeInTheDocument();
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
    });
    await user.click(screen.getByLabelText("Cancel Worker"));
    expect(screen.getByText("Cancel this agent subtree?")).toBeInTheDocument();
    await user.click(screen.getByText("Cancel subtree"));
    await waitFor(() => {
      expect(cancelBody).toEqual({ chat_id: chatId, subtree: true });
    });
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
    expect(screen.getByText("Finished agent")).toBeInTheDocument();
    expect(
      screen.queryByLabelText("Message Finished agent"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByLabelText("Cancel Finished agent"),
    ).not.toBeInTheDocument();
    expect(screen.getByLabelText("Copy agent ID")).toBeInTheDocument();
  });

  test("strips agent-kind prefixes and markdown emphasis from displayed titles", () => {
    const worker = agent("worker", chatId, "worker-chat", {
      title: "Subagent: **Fix the flaky test**",
    });
    render(<AgentsSection chatId={chatId} />, {
      preloadedState: panelState([worker]),
    });

    const title = screen.getByRole("button", { name: "Fix the flaky test" });
    expect(title).toBeInTheDocument();
    expect(title).toHaveAttribute("title", "Subagent: **Fix the flaky test**");
    expect(
      screen.getByLabelText("Message Subagent: **Fix the flaky test**"),
    ).toBeInTheDocument();
  });

  test("formats aggregate usage above a million tokens with an M suffix", () => {
    const heavy = agent("heavy", chatId, null, {
      title: "Heavy agent",
      tokens_used: 4_996_000,
    });
    render(<AgentsSection chatId={chatId} />, {
      preloadedState: panelState([heavy]),
    });

    expect(screen.getByTestId("agents-aggregate-usage")).toHaveTextContent(
      "5.0M tokens",
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
