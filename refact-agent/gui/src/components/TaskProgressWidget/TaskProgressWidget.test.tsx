import { http, HttpResponse } from "msw";
import { describe, expect, test } from "vitest";

import { render, screen, waitFor } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
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

describe("TaskProgressWidget goal projection", () => {
  test("renders in a goal-supporting fresh chat and exposes the goal editor", async () => {
    const { user } = renderWidget(
      makeRuntime({ goal: null, mode: "task_agent", toolUse: "explore" }),
    );

    expect(screen.getByText("Set a goal")).toBeInTheDocument();
    expect(screen.queryByText("Tasks cleared")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /Set a goal/i }));
    await user.click(screen.getByRole("button", { name: /Goal Not set/i }));

    expect(screen.getByLabelText("Goal text")).toBeInTheDocument();
  });

  test("empty collapsed header distinguishes fresh goal creation from cleared tasks", () => {
    const freshView = renderWidget(
      makeRuntime({ goal: null, mode: "quick_agent", toolUse: "explore" }),
    );

    expect(screen.getByText("Set a goal")).toBeInTheDocument();
    expect(screen.queryByText("Tasks cleared")).not.toBeInTheDocument();
    freshView.unmount();

    renderWidget(
      makeRuntime({
        goal: null,
        messages: taskMessages([]),
        mode: "quick_agent",
        toolUse: "explore",
      }),
    );

    expect(screen.getByText("Tasks cleared")).toBeInTheDocument();
    expect(screen.queryByText("Set a goal")).not.toBeInTheDocument();
  });

  test("does not render in a non-goal mode without tasks or a goal", () => {
    renderWidget(
      makeRuntime({ goal: null, mode: "explore", toolUse: "explore" }),
    );

    expect(screen.queryByText("Set a goal")).not.toBeInTheDocument();
    expect(screen.queryByText("Tasks cleared")).not.toBeInTheDocument();
    expect(screen.queryByText("Task Progress")).not.toBeInTheDocument();
  });

  test("collapsed widget shows the goal-set indicator and status", () => {
    renderWidget(makeRuntime({ goal: makeGoal() }));

    expect(screen.getByText("Goal set")).toBeInTheDocument();
    expect(screen.getByText("Active")).toBeInTheDocument();
  });

  test("expanded widget starts with the goal row", () => {
    renderWidget(makeRuntime({ goal: makeGoal(), expanded: true }));

    expect(screen.getByText("Task Progress")).toBeInTheDocument();
    expect(screen.getByText("Goal")).toBeInTheDocument();
    expect(screen.getByText("Active")).toBeInTheDocument();
  });

  test("expanded goal block renders verifier attempts, events, and finite budget ratios", () => {
    renderWidget(
      makeRuntime({ goal: makeGoal(), expanded: true, goalExpanded: true }),
    );

    expect(
      screen.getByText("2/5 turns · 1200/5000 tokens"),
    ).toBeInTheDocument();
    expect(screen.queryByText("No budget limits")).not.toBeInTheDocument();
    expect(screen.getByText("Verifier attempts")).toBeInTheDocument();
    expect(screen.getByText("needs_work")).toBeInTheDocument();
    expect(screen.getByText("missing verifier coverage")).toBeInTheDocument();
    expect(screen.getByText("Add one more verifier test.")).toBeInTheDocument();
    expect(screen.getByText("Goal events")).toBeInTheDocument();
    expect(screen.getByText("Asked for progress")).toBeInTheDocument();
  });

  test("expanded goal block renders omitted budget limits as unlimited", () => {
    const { container } = renderWidget(
      makeRuntime({
        goal: makeGoal({
          budget: {
            cooldown_ms: 1500,
            no_progress_token_threshold: 50,
          },
        }),
        expanded: true,
        goalExpanded: true,
      }),
    );

    expect(
      screen.getByText("2 turns · 1200 tokens · No budget limits"),
    ).toBeInTheDocument();
    expect(container.textContent).not.toContain("/0");
    expect(container.textContent).not.toContain("/undefined");
    expect(container.textContent).not.toContain("/null");
    expect(container.textContent).not.toContain("NaN");
    expect(container.textContent).not.toContain("undefined");
  });

  test("expanded goal block treats zero budget limits as unlimited", () => {
    const { container } = renderWidget(
      makeRuntime({
        goal: makeGoal({
          budget: {
            max_turns: 0,
            max_minutes: 0,
            max_tokens: 0,
            cooldown_ms: 1500,
            no_progress_token_threshold: 50,
            no_progress_turns: 0,
          },
        }),
        expanded: true,
        goalExpanded: true,
      }),
    );

    expect(
      screen.getByText("2 turns · 1200 tokens · No budget limits"),
    ).toBeInTheDocument();
    expect(container.textContent).not.toContain("0/0");
    expect(container.textContent).not.toContain("/0");
  });

  test("editing an existing goal dispatches update_goal", async () => {
    const commands = captureCommands();
    const { user } = renderWidget(
      makeRuntime({ goal: makeGoal(), expanded: true, goalExpanded: true }),
    );

    const input = screen.getByLabelText("Goal text");
    await user.clear(input);
    await user.type(input, "Ship the shiny frog pond");
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(commands).toHaveLength(1));
    expect(commands[0]).toMatchObject({
      type: "update_goal",
      note: "Ship the shiny frog pond",
    });
  });

  test("editing an empty goal block dispatches set_goal", async () => {
    const commands = captureCommands();
    const { user } = renderWidget(
      makeRuntime({
        goal: null,
        expanded: true,
        goalExpanded: true,
        messages: taskMessages([]),
      }),
    );

    await user.type(screen.getByLabelText("Goal text"), "Start a fresh goal");
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(commands).toHaveLength(1));
    expect(commands[0]).toMatchObject({
      type: "set_goal",
      content: "Start a fresh goal",
    });
    expect(commands[0]).not.toHaveProperty("budget");
  });

  test("fresh goal dispatches set_goal with a positive max turns budget", async () => {
    const commands = captureCommands();
    const { user } = renderWidget(
      makeRuntime({
        goal: null,
        expanded: true,
        goalExpanded: true,
        messages: taskMessages([]),
      }),
    );

    await user.type(
      screen.getByLabelText("Goal text"),
      "Start a budgeted goal",
    );
    await user.type(screen.getByLabelText("Max turns"), "5");
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(commands).toHaveLength(1));
    expect(commands[0]).toMatchObject({
      type: "set_goal",
      content: "Start a budgeted goal",
    });
    expect(commands[0].budget).toEqual({ max_turns: 5 });
  });

  test("existing goal dispatches set_goal_budget from budget inputs", async () => {
    const commands = captureCommands();
    const { user } = renderWidget(
      makeRuntime({ goal: makeGoal(), expanded: true, goalExpanded: true }),
    );

    await user.clear(screen.getByLabelText("Max turns"));
    await user.type(screen.getByLabelText("Max turns"), "7");
    await user.clear(screen.getByLabelText("Max minutes"));
    await user.clear(screen.getByLabelText("Max tokens"));
    await user.click(screen.getByRole("button", { name: "Apply budget" }));

    await waitFor(() => expect(commands).toHaveLength(1));
    expect(commands[0]).toMatchObject({ type: "set_goal_budget" });
    expect(commands[0].budget).toEqual({ max_turns: 7 });
  });

  test("clearing existing goal budget inputs dispatches unlimited budget", async () => {
    const commands = captureCommands();
    const { user } = renderWidget(
      makeRuntime({ goal: makeGoal(), expanded: true, goalExpanded: true }),
    );

    await user.clear(screen.getByLabelText("Max turns"));
    await user.clear(screen.getByLabelText("Max minutes"));
    await user.clear(screen.getByLabelText("Max tokens"));
    await user.click(screen.getByRole("button", { name: "Apply budget" }));

    await waitFor(() => expect(commands).toHaveLength(1));
    expect(commands[0]).toMatchObject({ type: "set_goal_budget" });
    expect(commands[0].budget).toEqual({});
  });

  test("goal controls dispatch pause, resume, and stop", async () => {
    const commands = captureCommands();
    const activeView = renderWidget(
      makeRuntime({ goal: makeGoal(), expanded: true, goalExpanded: true }),
    );

    await activeView.user.click(screen.getByRole("button", { name: "Pause" }));
    await activeView.user.click(screen.getByRole("button", { name: "Stop" }));

    await waitFor(() => expect(commands).toHaveLength(2));
    activeView.unmount();

    const pausedView = renderWidget(
      makeRuntime({
        goal: makeGoal({ active: false, status: "paused" }),
        expanded: true,
        goalExpanded: true,
      }),
    );
    await pausedView.user.click(screen.getByRole("button", { name: "Resume" }));

    await waitFor(() => expect(commands).toHaveLength(3));
    expect(commands.map((command) => command.action)).toEqual([
      "pause",
      "stop",
      "resume",
    ]);
    expect(commands.every((command) => command.type === "goal_control")).toBe(
      true,
    );
  });

  test("collapsed widget keeps goal controls clickable without expanding", async () => {
    const commands = captureCommands();
    const { user } = renderWidget(makeRuntime({ goal: makeGoal() }));

    expect(screen.getByText("Goal set")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Pause goal" }));

    await waitFor(() => expect(commands).toHaveLength(1));
    expect(commands[0]).toMatchObject({
      type: "goal_control",
      action: "pause",
    });
    expect(screen.queryByText("Task Progress")).not.toBeInTheDocument();
  });

  test("collapsed goal section keeps controls clickable without expanding", async () => {
    const commands = captureCommands();
    const { user } = renderWidget(
      makeRuntime({
        goal: makeGoal({ active: false, status: "paused" }),
        expanded: true,
        goalExpanded: false,
      }),
    );

    await user.click(screen.getByRole("button", { name: "Resume goal" }));

    await waitFor(() => expect(commands).toHaveLength(1));
    expect(commands[0]).toMatchObject({
      type: "goal_control",
      action: "resume",
    });
    expect(screen.queryByLabelText("Goal text")).not.toBeInTheDocument();
  });

  test("collapsed goal controls reflect goal status availability", () => {
    renderWidget(
      makeRuntime({ goal: makeGoal({ active: false, status: "stopped" }) }),
    );

    expect(screen.getByRole("button", { name: "Pause goal" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Stop goal" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Resume goal" })).toBeEnabled();
  });
});
