import { http, HttpResponse } from "msw";
import { describe, expect, test, vi } from "vitest";
import { act } from "react-dom/test-utils";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { applyChatEvent } from "../../features/Chat/Thread";
import type { AppStore } from "../../app/store";
import { selectRuntimeById } from "../../features/Chat/Thread/selectors";
import type {
  Chat,
  ChatThreadRuntime,
  TodoItem,
  ToolUse,
} from "../../features/Chat/Thread/types";
import type { ChatMessages, GoalSnapshot } from "../../services/refact/types";
import { TaskProgressWidget } from "./TaskProgressWidget";

const threadId = "goal-widget-chat";

function makeGoal(overrides: Partial<GoalSnapshot> = {}): GoalSnapshot {
  return {
    content: "Ship the frog pond",
    version: 1,
    active: true,
    status: "active",
    budget: {
      max_turns: 5,
      max_minutes: 10,
      max_tokens: 5000,
      cooldown_ms: 1500,
      no_progress_token_threshold: 50,
      no_progress_turns: 2,
    },
    progress: {
      turns_used: 2,
      tokens_used: 1200,
      started_at_ms: 1000,
      no_progress_turns: 1,
      last_nudge_at_ms: 2000,
    },
    attempts: [
      {
        at_ms: 3000,
        trigger: "manual",
        verdict: "needs_work",
        gaps: ["missing verifier coverage"],
        verifier_reply: "Add one more verifier test.",
      },
    ],
    events: [{ at_ms: 4000, kind: "nudge", text: "Asked for progress" }],
    transferred_from: null,
    transferred_to: null,
    ...overrides,
  };
}

function taskMessages(tasks: TodoItem[]): ChatMessages {
  return [
    {
      role: "assistant",
      content: "tasks",
      message_id: "assistant-tasks",
      tool_calls: [
        {
          id: "call-tasks",
          index: 0,
          type: "function",
          function: {
            name: "tasks_set",
            arguments: JSON.stringify({ tasks }),
          },
        },
      ],
    },
    {
      role: "tool",
      content: "ok",
      message_id: "tool-tasks",
      tool_call_id: "call-tasks",
      tool_failed: false,
    },
  ];
}

function makeRuntime(options: {
  goal?: GoalSnapshot | null;
  messages?: ChatMessages;
  expanded?: boolean;
  goalExpanded?: boolean;
  mode?: string;
  toolUse?: ToolUse;
}): ChatThreadRuntime {
  return {
    thread: {
      id: threadId,
      messages: options.messages ?? [],
      title: "Goal Widget Chat",
      model: "gpt-4",
      mode: options.mode ?? "agent",
      tool_use: options.toolUse ?? "agent",
      new_chat_suggested: { wasSuggested: false },
      boost_reasoning: false,
      increase_max_tokens: false,
      include_project_info: true,
      auto_enrichment_enabled: false,
      goal: options.goal ?? null,
    },
    streaming: false,
    waiting_for_response: false,
    prevent_send: false,
    error: null,
    queued_items: [],
    send_immediately: false,
    attached_images: [],
    attached_text_files: [],
    background_agents: {},
    confirmation: {
      pause: false,
      pause_reasons: [],
      status: { wasInteracted: false, confirmationStatus: true },
    },
    snapshot_received: true,
    task_widget_expanded: options.expanded ?? false,
    task_goal_expanded: options.goalExpanded ?? false,
    memory_enrichment_user_touched: false,
    manual_preview_items: [],
    manual_preview_ran: false,
  };
}

function makeChat(runtime: ChatThreadRuntime): Chat {
  return {
    current_thread_id: threadId,
    open_thread_ids: [threadId],
    threads: { [threadId]: runtime },
    system_prompt: {},
    tool_use: "agent",
    sse_refresh_requested: null,
    stream_version: 0,
  };
}

function renderWidget(runtime: ChatThreadRuntime) {
  return render(<TaskProgressWidget />, {
    preloadedState: { chat: makeChat(runtime) },
  });
}

function captureCommands() {
  const commands: Record<string, unknown>[] = [];
  server.use(
    http.post("*/v1/chats/:id/commands", async ({ request }) => {
      commands.push((await request.json()) as Record<string, unknown>);
      return HttpResponse.json({ status: "queued" });
    }),
  );
  return commands;
}

function nextSeq(store: AppStore) {
  return String(
    Number(
      selectRuntimeById(store.getState(), threadId)?.last_applied_seq ?? 0,
    ) + 1,
  );
}

function acknowledge(store: AppStore, command: Record<string, unknown>) {
  act(() => {
    store.dispatch(
      applyChatEvent({
        chat_id: threadId,
        seq: nextSeq(store),
        type: "ack",
        client_request_id: String(command.client_request_id),
        accepted: true,
        result: { queued: true },
      }),
    );
  });
}

function applyGoalStatus(store: AppStore, status: GoalSnapshot["status"]) {
  act(() => {
    store.dispatch(
      applyChatEvent({
        chat_id: threadId,
        seq: nextSeq(store),
        type: "runtime_updated",
        state: "idle",
        goal_status: status,
      }),
    );
  });
}

const plan = {
  content: "Implement the approved plan",
  mode: "agent",
  version: 2,
  created_at_ms: 1,
  delta_count: 1,
  checkpoint: null,
  history: [{ seq: 1, content: "Retain plan history" }],
};

const tasks: TodoItem[] = [
  { id: "a", content: "Inspect props", status: "completed" },
  { id: "b", content: "Write fixtures", status: "in_progress" },
  { id: "c", content: "Review states", status: "pending" },
];

describe("TaskProgressWidget", () => {
  test("collapsed header shows headline and task count without goal controls", () => {
    renderWidget(
      makeRuntime({ goal: makeGoal(), messages: taskMessages(tasks) }),
    );

    expect(screen.getByTestId("transcript-target")).toHaveTextContent(
      "Ship the frog pond",
    );
    expect(screen.getByLabelText("1 of 3 tasks completed")).toHaveTextContent(
      "1/3",
    );
    expect(
      screen.queryByRole("button", { name: "Pause goal" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Stop goal" }),
    ).not.toBeInTheDocument();
  });

  test("does not render in a non-goal mode without tasks or a goal", () => {
    renderWidget(
      makeRuntime({ goal: null, mode: "explore", toolUse: "explore" }),
    );
    expect(screen.queryByTestId("transcript-row")).not.toBeInTheDocument();
  });

  test("expanding reveals tasks and Pause dispatches goal_control", async () => {
    const commands = captureCommands();
    const { user } = renderWidget(
      makeRuntime({ goal: makeGoal(), messages: taskMessages(tasks) }),
    );

    await user.click(screen.getByTestId("transcript-toggle"));
    expect(screen.getByText("Write fixtures")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Pause goal" }));

    await waitFor(() => expect(commands).toHaveLength(1));
    expect(commands[0]).toMatchObject({
      type: "goal_control",
      action: "pause",
    });
  });

  test("paused goal offers Resume and Stop dispatches stop", async () => {
    const commands = captureCommands();
    const { user } = renderWidget(
      makeRuntime({
        goal: makeGoal({ active: false, status: "paused" }),
        expanded: true,
      }),
    );

    expect(screen.getByRole("button", { name: "Resume goal" })).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "Stop goal" }));

    await waitFor(() => expect(commands).toHaveLength(1));
    expect(commands[0]).toMatchObject({ type: "goal_control", action: "stop" });
  });

  test("details dialog shows plan and usage without ordinary limit controls", async () => {
    const commands = captureCommands();
    const { user } = renderWidget(
      makeRuntime({ goal: makeGoal({ plan }), expanded: true }),
    );

    await user.click(screen.getByRole("button", { name: "Goal details" }));
    const dialog = screen.getByRole("dialog", { name: "Goal details" });
    expect(
      within(dialog).getByText("Implement the approved plan"),
    ).toBeVisible();
    expect(within(dialog).getByText("Retain plan history")).toBeVisible();
    expect(
      within(dialog).getByText("Add one more verifier test."),
    ).toBeVisible();

    expect(
      within(dialog).queryByRole("switch", { name: "Max turns" }),
    ).not.toBeInTheDocument();
    expect(
      within(dialog).queryByRole("button", { name: "Apply limits" }),
    ).not.toBeInTheDocument();
    expect(commands).toHaveLength(0);
  });

  test("editor sets a fresh goal and appends updates to an existing one", async () => {
    const commands = captureCommands();
    const fresh = renderWidget(makeRuntime({ goal: null, expanded: true }));

    await fresh.user.click(screen.getByRole("button", { name: "Create goal" }));
    await fresh.user.type(screen.getByLabelText("Goal text"), "Ship it");
    await fresh.user.click(screen.getByRole("button", { name: "Set goal" }));
    await waitFor(() => expect(commands).toHaveLength(1));
    expect(commands[0]).toMatchObject({ type: "set_goal", content: "Ship it" });
    expect(commands[0]).not.toHaveProperty("budget");
    fresh.unmount();

    const existing = renderWidget(
      makeRuntime({ goal: makeGoal(), expanded: true }),
    );
    await existing.user.click(
      screen.getByRole("button", { name: "Edit goal" }),
    );
    await existing.user.clear(screen.getByLabelText("Goal text"));
    await existing.user.type(screen.getByLabelText("Goal text"), "Add tests");
    await existing.user.click(
      screen.getByRole("button", { name: "Update goal" }),
    );
    await waitFor(() => expect(commands).toHaveLength(2));
    expect(commands[1]).toMatchObject({
      type: "update_goal",
      note: "Add tests",
    });
    expect(commands[1]).not.toHaveProperty("content");
  });

  test("a rejected new goal keeps its draft without sending effort limits", async () => {
    const commands: Record<string, unknown>[] = [];
    server.use(
      http.post("*/v1/chats/:id/commands", async ({ request }) => {
        commands.push((await request.json()) as Record<string, unknown>);
        return HttpResponse.text("This chat cannot create a goal", {
          status: 403,
        });
      }),
    );
    const { user } = renderWidget(makeRuntime({ goal: null, expanded: true }));

    await user.click(screen.getByRole("button", { name: "Create goal" }));
    const dialog = screen.getByRole("dialog", { name: "Set a goal" });
    await user.type(within(dialog).getByLabelText("Goal text"), "Keep it");
    await user.click(within(dialog).getByRole("button", { name: "Set goal" }));

    await waitFor(() => expect(within(dialog).getByText(/403/)).toBeVisible());
    expect(commands).toHaveLength(1);
    expect(commands[0]).toMatchObject({
      type: "set_goal",
      content: "Keep it",
    });
    expect(within(dialog).getByLabelText("Goal text")).toHaveValue("Keep it");
    expect(commands[0]).not.toHaveProperty("budget");
    expect(
      within(dialog).queryByRole("switch", { name: "Max turns" }),
    ).not.toBeInTheDocument();
    expect(
      within(dialog).getByRole("button", { name: "Set goal" }),
    ).toBeEnabled();
  });

  test("an unconfirmed goal command offers a status check and never replays", async () => {
    vi.useFakeTimers();
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(new Response(null, { status: 200 }));
    const view = renderWidget(
      makeRuntime({ goal: makeGoal(), expanded: true }),
    );
    try {
      fireEvent.click(screen.getByRole("button", { name: "Pause goal" }));
      await act(async () => {
        await vi.advanceTimersByTimeAsync(15_001);
      });
      expect(screen.getByRole("alert")).toHaveTextContent(
        "Outcome not confirmed",
      );
      expect(screen.getByRole("button", { name: "Pause goal" })).toBeDisabled();
      const body = JSON.parse(
        String(fetchMock.mock.calls[0][1]?.body),
      ) as Record<string, unknown>;
      fireEvent.click(screen.getByRole("button", { name: "Check status" }));
      expect(fetchMock).toHaveBeenCalledTimes(1);
      acknowledge(view.store, body);
      applyGoalStatus(view.store, "paused");
      await act(async () => {
        await Promise.resolve();
      });
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Resume goal" })).toBeEnabled();
      expect(fetchMock).toHaveBeenCalledTimes(1);
    } finally {
      view.unmount();
      fetchMock.mockRestore();
      vi.useRealTimers();
    }
  });

  test("details usage updates without exposing retired limit editors", async () => {
    const { user, store } = renderWidget(
      makeRuntime({ goal: makeGoal(), expanded: true }),
    );
    await user.click(screen.getByRole("button", { name: "Goal details" }));
    const dialog = screen.getByRole("dialog", { name: "Goal details" });

    act(() => {
      store.dispatch(
        applyChatEvent({
          chat_id: threadId,
          seq: nextSeq(store),
          type: "runtime_updated",
          state: "generating",
          goal_tokens_used: 4000,
        }),
      );
    });

    expect(within(dialog).getByText(/4k tokens/)).toBeVisible();
    expect(
      within(dialog).queryByRole("button", { name: "Apply limits" }),
    ).not.toBeInTheDocument();
  });
});
