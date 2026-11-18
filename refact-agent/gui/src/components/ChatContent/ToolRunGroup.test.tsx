import React from "react";
import { act } from "react-dom/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  createDefaultChatState,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "../../utils/test-utils";
import type { ChatMessages, ToolCall } from "../../services/refact";
import { buildDisplayItems, type DisplayItem } from "./ChatContentDisplayItems";
import { groupToolRuns, type DisplayItemToolRun } from "./toolRunGroups";
import { ToolRunGroup } from "./ToolRunGroup";
import { EventRow } from "./EventRow";
import { setToolUse } from "../../features/Chat/Thread/actions";
import { CollapsibleStoreProvider } from "./useStoredOpen";
import type { CollapsibleStore } from "./CollapsibleStore";
import { revealProcessOutput } from "./revealProcessOutput";
import { ProcessCompletedToasts } from "../../features/Notifications/Toast";
import { processCompleted } from "../../features/Notifications/notificationsSlice";

function call(id: string, name: string, args: Record<string, unknown>) {
  const toolCall: ToolCall = {
    id,
    index: 0,
    type: "function",
    function: { name, arguments: JSON.stringify(args) },
  };
  return toolCall;
}

function tool(
  id: string,
  content: string,
  extra?: Record<string, unknown>,
  failed = false,
): ChatMessages[number] {
  return {
    role: "tool",
    tool_call_id: id,
    content,
    tool_failed: failed,
    ...(extra ? { extra } : {}),
  };
}

const DEV = call("c-dev", "shell", {
  command: "npm run dev",
  run_in_background: true,
});
const BUILD = call("c-build", "shell", {
  command: "npm run build",
  run_in_background: true,
});

function messagesWith(devStatus: "running" | "exited"): ChatMessages {
  return [
    { role: "user", content: "go", message_id: "u1" },
    {
      role: "assistant",
      content: "",
      message_id: "a1",
      tool_calls: [call("c-cat", "cat", { paths: "a.ts" }), DEV, BUILD],
    },
    tool("c-cat", "a.ts"),
    tool("c-dev", "Process started in background", {
      exec: {
        process_id: "proc-dev",
        status: devStatus,
        mode: "background",
        command: "npm run dev",
      },
    }),
    tool("c-build", "Process started in background", {
      exec: {
        process_id: "proc-build",
        status: "failed",
        mode: "background",
        command: "npm run build",
        exit_code: 1,
      },
    }),
    { role: "assistant", content: "Done", message_id: "a2" },
  ];
}

function toolRunOf(messages: ChatMessages): DisplayItemToolRun {
  const run = groupToolRuns(buildDisplayItems(messages, false)).find(
    (item): item is DisplayItemToolRun => item.type === "tool_run",
  );
  if (!run) throw new Error("expected a tool run group");
  return run;
}

function renderMember(item: DisplayItem): React.ReactNode {
  if (item.type !== "assistant") return <div>{item.key}</div>;
  return (
    <div data-testid="member" data-key={item.key}>
      {(item.view?.toolCalls ?? []).map((entry) => (
        <div
          key={entry.id}
          data-exec-process-id={
            entry.id === DEV.id
              ? "proc-dev"
              : entry.id === BUILD.id
                ? "proc-build"
                : undefined
          }
        >
          {entry.function.name}
        </div>
      ))}
    </div>
  );
}

function mapStore(): CollapsibleStore & { values: Map<string, boolean> } {
  const values = new Map<string, boolean>();
  return {
    values,
    get: (key) => values.get(key),
    set: (key, open) => values.set(key, open),
  };
}

function renderGroup(
  devStatus: "running" | "exited",
  store: CollapsibleStore = mapStore(),
) {
  const messages = messagesWith(devStatus);
  const chat = createDefaultChatState();
  const threadId = chat.current_thread_id;
  chat.threads[threadId].thread.messages = messages;
  const group = toolRunOf(messages);
  const view = (
    <CollapsibleStoreProvider value={store}>
      <ToolRunGroup
        group={group}
        threadId={threadId}
        renderMember={renderMember}
      />
    </CollapsibleStoreProvider>
  );
  return { ...render(view, { preloadedState: { chat } }), group, view };
}

function processCards(id: string): Element[] {
  return Array.from(
    document.querySelectorAll(`[data-exec-process-id="${id}"]`),
  );
}

const TRANSCRIPT_SETTLE_MS = 1000;

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("ToolRunGroup", () => {
  it("starts collapsed with a summary row and no member rows", () => {
    renderGroup("exited");
    const toggle = screen.getByTestId("tool-run-toggle");
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByTestId("tool-run-members")).not.toBeInTheDocument();
    expect(screen.getByTestId("tool-run-summary")).toHaveTextContent(
      "Ran 2 commands · Read 1 file",
    );
  });

  it("toggles with click and keyboard and reports aria-expanded", async () => {
    const { user } = renderGroup("exited");
    const toggle = screen.getByTestId("tool-run-toggle");
    await user.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByTestId("tool-run-members")).toBeInTheDocument();
    toggle.focus();
    await user.keyboard("{Enter}");
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    await user.keyboard(" ");
    expect(toggle).toHaveAttribute("aria-expanded", "true");
  });

  it("persists the open state in the collapsible store under run:<key>", async () => {
    const store = mapStore();
    const first = renderGroup("exited", store);
    expect(first.group.key).toBe("run:a1");
    await first.user.click(screen.getByTestId("tool-run-toggle"));
    expect(store.values.get("run:a1")).toBe(true);
    first.unmount();

    renderGroup("exited", store);
    expect(screen.getByTestId("tool-run-toggle")).toHaveAttribute(
      "aria-expanded",
      "true",
    );
  });

  it("counts exec failures from the exit status, not only tool_failed", () => {
    renderGroup("exited");
    expect(screen.getByTestId("tool-run-failed")).toHaveTextContent("1 failed");
  });

  it("uses one summary text for the visible row, tooltip and accessible name", () => {
    const edit = call("c-edit", "update_textdoc", { path: "a.ts" });
    const messages: ChatMessages = [
      { role: "user", content: "go", message_id: "u1" },
      {
        role: "assistant",
        content: "",
        message_id: "a1",
        tool_calls: [edit, call("c-ls", "shell", { command: "ls" })],
      },
      tool("c-edit", "ok"),
      {
        role: "diff",
        content: [
          {
            file_name: "a.ts",
            file_action: "edit",
            line1: 1,
            line2: 1,
            lines_add: "x\n",
            lines_remove: "",
          },
        ],
        tool_call_id: "c-edit",
        message_id: "d1",
      },
      tool("c-ls", "a.ts"),
      { role: "assistant", content: "Done", message_id: "a2" },
    ];
    const chat = createDefaultChatState();
    chat.threads[chat.current_thread_id].thread.messages = messages;
    render(
      <ToolRunGroup
        group={toolRunOf(messages)}
        threadId={chat.current_thread_id}
        renderMember={renderMember}
      />,
      { preloadedState: { chat } },
    );
    const expected = "Edited 1 file +1 −0 · Ran 1 command";
    const toggle = screen.getByRole("button", { name: expected });
    expect(toggle).toHaveAttribute("title", expected);
    expect(screen.getByTestId("tool-run-summary")).toHaveTextContent(expected);
  });

  it("pins calls needing the user while collapsed and never mounts them twice", async () => {
    const { user } = renderGroup("running");
    const toggle = screen.getByTestId("tool-run-toggle");
    expect(screen.getByTestId("tool-run-pinned")).toBeInTheDocument();
    expect(processCards("proc-dev")).toHaveLength(1);
    expect(processCards("proc-build")).toHaveLength(0);

    await user.click(toggle);
    expect(screen.queryByTestId("tool-run-pinned")).not.toBeInTheDocument();
    expect(processCards("proc-dev")).toHaveLength(1);

    await user.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(processCards("proc-dev")).toHaveLength(1);

    await waitFor(() =>
      expect(screen.queryByTestId("tool-run-members")).not.toBeInTheDocument(),
    );
    expect(screen.getByTestId("tool-run-pinned")).toBeInTheDocument();
    expect(processCards("proc-dev")).toHaveLength(1);
  });

  it("opens itself for a reveal request and scrolls once the panel settles", () => {
    vi.useFakeTimers();
    const scroll = vi
      .spyOn(Element.prototype, "scrollIntoView")
      .mockImplementation(() => undefined);
    renderGroup("exited");
    const toggle = screen.getByTestId("tool-run-toggle");
    expect(processCards("proc-build")).toHaveLength(0);

    let claimed = false;
    act(() => {
      claimed = revealProcessOutput("proc-build");
    });
    expect(claimed).toBe(true);
    expect(toggle).toHaveAttribute("aria-expanded", "true");

    act(() => {
      vi.advanceTimersByTime(50);
    });
    expect(processCards("proc-build")).toHaveLength(1);
    expect(scroll).not.toHaveBeenCalled();

    const panel = document.getElementById(
      toggle.getAttribute("aria-controls") ?? "",
    );
    if (!panel) throw new Error("expected the group panel");
    act(() => {
      fireEvent.transitionEnd(panel);
    });
    expect(scroll).toHaveBeenCalledTimes(1);
    expect(scroll.mock.contexts[0]).toBe(processCards("proc-build")[0]);
  });

  it("ignores reveal requests for processes it does not own", () => {
    renderGroup("exited");
    expect(revealProcessOutput("proc-other")).toBe(false);
    expect(screen.getByTestId("tool-run-toggle")).toHaveAttribute(
      "aria-expanded",
      "false",
    );
  });

  it("opens the owning collapsed group from the process toast View button", async () => {
    const messages = messagesWith("exited");
    const chat = createDefaultChatState();
    const threadId = chat.current_thread_id;
    chat.threads[threadId].thread.messages = messages;
    const { store, user } = render(
      <>
        <ToolRunGroup
          group={toolRunOf(messages)}
          threadId={threadId}
          renderMember={renderMember}
        />
        <ProcessCompletedToasts />
      </>,
      { preloadedState: { chat } },
    );
    act(() => {
      store.dispatch(
        processCompleted({
          chat_id: threadId,
          seq: "1",
          type: "process_completed",
          process_id: "proc-build",
          status: "failed",
          exit_code: 1,
          short_description: "Build",
          mode: "background",
        }),
      );
    });
    await user.click(await screen.findByRole("button", { name: "View" }));
    await waitFor(() =>
      expect(screen.getByTestId("tool-run-toggle")).toHaveAttribute(
        "aria-expanded",
        "true",
      ),
    );
    expect(processCards("proc-build")).toHaveLength(1);
  });

  it("pins a generic tool rendered as an exec card while its process is live", () => {
    const runner = call("c-runner", "custom_runner", { target: "dev" });
    const messages: ChatMessages = [
      { role: "user", content: "go", message_id: "u1" },
      {
        role: "assistant",
        content: "",
        message_id: "a1",
        tool_calls: [call("c-cat", "cat", { paths: "a.ts" }), runner],
      },
      tool("c-cat", "a.ts"),
      tool("c-runner", "started", {
        exec: {
          process_id: "proc-runner",
          status: "running",
          mode: "background",
        },
      }),
      { role: "assistant", content: "Done", message_id: "a2" },
    ];
    const chat = createDefaultChatState();
    chat.threads[chat.current_thread_id].thread.messages = messages;
    render(
      <ToolRunGroup
        group={toolRunOf(messages)}
        threadId={chat.current_thread_id}
        renderMember={renderMember}
      />,
      { preloadedState: { chat } },
    );
    const pinned = screen.getByTestId("tool-run-pinned");
    expect(pinned).toHaveTextContent("custom_runner");
    expect(pinned).not.toHaveTextContent("cat");
  });

  it("re-opens a closing group instead of scrolling to its fading card", () => {
    vi.useFakeTimers();
    const scroll = vi
      .spyOn(Element.prototype, "scrollIntoView")
      .mockImplementation(() => undefined);
    renderGroup("exited");
    const toggle = screen.getByTestId("tool-run-toggle");
    fireEvent.click(toggle);
    act(() => {
      vi.advanceTimersByTime(TRANSCRIPT_SETTLE_MS);
    });
    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(processCards("proc-build")).toHaveLength(1);

    let claimed = false;
    act(() => {
      claimed = revealProcessOutput("proc-build");
    });
    expect(claimed).toBe(true);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(scroll).not.toHaveBeenCalled();
  });

  it("drops a pending reveal when the group is closed before it settles", () => {
    vi.useFakeTimers();
    const scroll = vi
      .spyOn(Element.prototype, "scrollIntoView")
      .mockImplementation(() => undefined);
    renderGroup("exited");
    const toggle = screen.getByTestId("tool-run-toggle");
    act(() => {
      revealProcessOutput("proc-build");
    });
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    fireEvent.click(toggle);
    act(() => {
      vi.advanceTimersByTime(TRANSCRIPT_SETTLE_MS);
    });
    fireEvent.click(toggle);
    act(() => {
      vi.advanceTimersByTime(TRANSCRIPT_SETTLE_MS);
    });
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(scroll).not.toHaveBeenCalled();
  });

  it("summarizes folded notices and keeps their actions working when expanded", async () => {
    const messages: ChatMessages = [
      { role: "user", content: "go", message_id: "u1" },
      {
        role: "assistant",
        content: "",
        message_id: "a1",
        tool_calls: [call("c-cat", "cat", { paths: "a.ts" })],
      },
      tool("c-cat", "a.ts"),
      {
        role: "event",
        message_id: "ev-proc",
        subkind: "process_completed",
        source: "exec.registry",
        content: "Build finished",
        payload: { process_id: "proc-elsewhere", exit_code: 0 },
      },
      {
        role: "event",
        message_id: "ev-cron",
        subkind: "cron_fire",
        source: "scheduler.cron",
        content: "Nightly run fired",
        payload: { task_id: "task-1" },
      },
      { role: "assistant", content: "Done", message_id: "a2" },
    ];
    const chat = createDefaultChatState();
    chat.threads[chat.current_thread_id].thread.messages = messages;
    const onOpenOutput = vi.fn();
    const renderWithEvents = (item: DisplayItem) =>
      item.type === "event" ? (
        <EventRow
          event={item.event}
          run={item.run}
          onOpenProcessOutput={onOpenOutput}
        />
      ) : (
        renderMember(item)
      );
    const { store, user } = render(
      <ToolRunGroup
        group={toolRunOf(messages)}
        threadId={chat.current_thread_id}
        renderMember={renderWithEvents}
      />,
      { preloadedState: { chat } },
    );
    const expected = "Read 1 file · Received 2 notices";
    const toggle = screen.getByRole("button", { name: expected });
    expect(toggle).toHaveAttribute("title", expected);
    expect(screen.getByTestId("tool-run-summary")).toHaveTextContent(expected);
    expect(screen.queryByTestId("event-row")).not.toBeInTheDocument();

    await user.click(toggle);
    const rows = within(screen.getByTestId("tool-run-members")).getAllByTestId(
      "event-row",
    );
    expect(rows).toHaveLength(2);
    for (const row of rows) {
      await user.click(within(row).getByTestId("transcript-toggle"));
    }
    await user.click(screen.getByRole("button", { name: "Open output" }));
    expect(onOpenOutput).toHaveBeenCalledWith("proc-elsewhere");
    await user.click(screen.getByRole("button", { name: "Open scheduler" }));
    expect(store.getState().pages.at(-1)).toEqual({
      name: "scheduler",
      taskId: "task-1",
    });
  });

  it("does not re-derive call flags on unrelated store updates", () => {
    const parse = vi.spyOn(JSON, "parse");
    const callArgs = new Set(
      [DEV, BUILD].map((item) => item.function.arguments),
    );
    const argParses = () =>
      parse.mock.calls.filter(([text]) => callArgs.has(text)).length;
    const { store } = renderGroup("running");
    const before = argParses();
    expect(before).toBeGreaterThan(0);
    act(() => {
      store.dispatch(setToolUse("agent"));
      store.dispatch(setToolUse("explore"));
    });
    expect(argParses()).toBe(before);
  });
});
