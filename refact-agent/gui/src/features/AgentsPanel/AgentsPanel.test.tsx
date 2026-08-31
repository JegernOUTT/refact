import { beforeEach, describe, expect, test, vi } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import { http, HttpResponse } from "msw";

import type { BackgroundAgentSummary } from "../../services/refact";
import { createDefaultChatState, render } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { AgentsPanel } from "./AgentsPanel";

const chatId = "parent-chat";

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

describe("AgentsPanel", () => {
  beforeEach(() => {
    server.use(http.get("*/v1/background-agents", () => HttpResponse.json([])));
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
    const { user } = render(<AgentsPanel chatId={chatId} />, {
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
    const { user } = render(<AgentsPanel chatId={chatId} />, {
      preloadedState: panelState([running, done]),
    });

    expect(screen.getByText("Running agent")).toBeInTheDocument();
    expect(screen.queryByText("Finished agent")).not.toBeInTheDocument();
    await user.click(screen.getByRole("radio", { name: /all/i }));
    expect(screen.getByText("Finished agent")).toBeInTheDocument();
  });

  test("navigates, posts a message, copies agent details, and confirms cancellation", async () => {
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
      <AgentsPanel chatId={chatId} onNavigate={onNavigate} />,
      {
        preloadedState: panelState([worker]),
      },
    );

    await user.click(screen.getByRole("button", { name: "Worker" }));
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
    await user.click(screen.getByLabelText("Message Worker"));
    await user.click(screen.getByLabelText("Cancel Worker"));
    expect(screen.getByText("Cancel this agent subtree?")).toBeInTheDocument();
    await user.click(screen.getByText("Cancel subtree"));
    await waitFor(() => {
      expect(cancelBody).toEqual({ chat_id: chatId, subtree: true });
    });
  });

  test("uses a drawer when requested", () => {
    render(<AgentsPanel chatId={chatId} narrow />, {
      preloadedState: panelState([agent("worker", chatId, null)]),
    });

    expect(document.querySelector('[role="dialog"]')).toBeInTheDocument();
  });
});
