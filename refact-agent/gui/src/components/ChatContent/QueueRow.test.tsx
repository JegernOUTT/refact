vi.hoisted(() => vi.resetModules());
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "../../utils/test-utils";
import type { QueuedItem } from "../../features/Chat";
import { ChatThreadProvider } from "../../features/Chat/Thread/ChatThreadContext";
import { getDraftMessage, saveDraftMessage } from "../../utils/threadStorage";
import { QueueRow } from "./QueueRow";
const actions = vi.hoisted(() => ({
  cancelQueued: vi.fn(),
  setQueuedPriority: vi.fn(),
  updatePendingDelivery: vi.fn(),
}));
vi.mock("../../hooks/useChatActions", () => ({
  useChatActions: () => actions,
}));
vi.mock("./revealProcessOutput", () => ({ revealProcessOutput: vi.fn() }));
import { revealProcessOutput } from "./revealProcessOutput";
const item: QueuedItem = {
  client_request_id: "delivery-1",
  command_type: "delivery",
  priority: false,
  preview: "Job done",
  enqueued_at_ms: 1,
};
beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  actions.cancelQueued.mockResolvedValue(true);
  actions.setQueuedPriority.mockResolvedValue(true);
  actions.updatePendingDelivery.mockResolvedValue(undefined);
});
afterEach(() => {
  vi.restoreAllMocks();
});
function expand() {
  fireEvent.click(screen.getByRole("button", { name: /1 Agent continuation/ }));
}
describe("QueueRow", () => {
  it("changes timing with expanded shared controls using the wire id", async () => {
    render(<QueueRow queuedItem={item} position={1} />);
    expand();
    fireEvent.click(screen.getByRole("radio", { name: "When idle" }));
    await waitFor(() =>
      expect(actions.updatePendingDelivery).toHaveBeenCalledWith("delivery-1", {
        push: "when_idle",
      }),
    );
    expect(screen.getByTestId("queued-item")).toHaveAttribute(
      "data-push",
      "append",
    );
  });
  it("changes timing directly from the chip", async () => {
    const { user } = render(<QueueRow queuedItem={item} position={1} />);
    screen.getByRole("combobox").focus();
    await user.keyboard(" ");
    fireEvent.keyDown(screen.getByRole("option", { name: /^When idle/ }), {
      key: "Enter",
    });
    await waitFor(() =>
      expect(actions.updatePendingDelivery).toHaveBeenCalledWith("delivery-1", {
        push: "when_idle",
      }),
    );
  });
  it("cancels deliveries and reports failed actions inline", async () => {
    actions.updatePendingDelivery.mockRejectedValue(new Error("offline"));
    render(<QueueRow queuedItem={item} position={1} />);
    fireEvent.click(
      screen.getByRole("button", { name: /Cancel pending delivery/ }),
    );
    await screen.findByRole("alert");
    expect(actions.updatePendingDelivery).toHaveBeenCalledWith("delivery-1", {
      cancel: true,
    });
    expect(screen.getByTestId("queued-item")).toHaveAttribute(
      "aria-busy",
      "false",
    );
  });
  it("uses legacy priority and restores editable text after cancellation", async () => {
    const { store } = render(
      <QueueRow
        queuedItem={{
          ...item,
          command_type: "user_message",
          content: "edit me",
        }}
        position={1}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /1 Your message/ }));
    fireEvent.click(screen.getByRole("radio", { name: "Send next" }));
    await waitFor(() =>
      expect(actions.setQueuedPriority).toHaveBeenCalledWith(
        "delivery-1",
        true,
      ),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Edit queued message text" }),
      ).toBeEnabled(),
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Edit queued message text" }),
    );
    await waitFor(() =>
      expect(getDraftMessage(store.getState().chat.current_thread_id)).toBe(
        "edit me",
      ),
    );
    expect(actions.cancelQueued).toHaveBeenCalledWith("delivery-1");
  });
  it("cancels legacy items without calling delivery API", async () => {
    render(
      <QueueRow
        queuedItem={{ ...item, command_type: "user_message" }}
        position={1}
      />,
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Cancel queued message" }),
    );
    await waitFor(() =>
      expect(actions.cancelQueued).toHaveBeenCalledWith("delivery-1"),
    );
    expect(actions.updatePendingDelivery).not.toHaveBeenCalled();
  });
  it("marks preempt and idle rows and reveals process output", () => {
    const { rerender } = render(
      <QueueRow
        queuedItem={{
          ...item,
          push: "preempt",
          event: {
            subkind: "process_completed",
            source: "exec.process",
            payload: { process_id: "proc" },
          },
        }}
        position={1}
      />,
    );
    expect(screen.getByTestId("queued-item")).toHaveAttribute(
      "data-push",
      "preempt",
    );
    fireEvent.click(screen.getByRole("button", { name: /1 Process/ }));
    fireEvent.click(screen.getByRole("button", { name: "Open output" }));
    expect(revealProcessOutput).toHaveBeenCalledWith("proc");
    rerender(
      <QueueRow queuedItem={{ ...item, push: "when_idle" }} position={1} />,
    );
    expect(screen.getByTestId("queued-item")).toHaveAttribute(
      "data-push",
      "when_idle",
    );
  });
  it.each([
    "submit_question_response",
    "answer_questions",
    "regenerate",
    "tool_decision",
  ])(
    "routes %s cancel and priority through the queued command API",
    async (commandType) => {
      const post = vi.spyOn(window, "postMessage");
      render(
        <QueueRow
          queuedItem={{
            ...item,
            client_request_id: "cmd-1",
            command_type: commandType,
            content: "answer text",
          }}
          position={1}
        />,
      );
      fireEvent.click(screen.getByRole("button", { name: /^1 / }));
      expect(screen.queryByRole("radio", { name: "When idle" })).toBeNull();
      expect(
        screen.queryByRole("button", { name: "Edit queued message text" }),
      ).toBeNull();
      fireEvent.click(screen.getByRole("radio", { name: "Send next" }));
      await waitFor(() =>
        expect(actions.setQueuedPriority).toHaveBeenCalledWith("cmd-1", true),
      );
      await waitFor(() =>
        expect(screen.getByTestId("queued-item")).toHaveAttribute(
          "aria-busy",
          "false",
        ),
      );
      fireEvent.click(
        screen.getByRole("button", { name: /^Cancel queued command/ }),
      );
      await waitFor(() =>
        expect(actions.cancelQueued).toHaveBeenCalledWith("cmd-1"),
      );
      expect(actions.updatePendingDelivery).not.toHaveBeenCalled();
      expect(post).not.toHaveBeenCalled();
      post.mockRestore();
    },
  );
  it("keeps failed command cancel retryable without restoring composer text", async () => {
    const post = vi.spyOn(window, "postMessage");
    actions.cancelQueued.mockResolvedValueOnce(false);
    render(
      <QueueRow
        queuedItem={{
          ...item,
          client_request_id: "cmd-2",
          command_type: "answer_questions",
          content: "my answer",
        }}
        position={1}
      />,
    );
    const cancel = screen.getByRole("button", {
      name: /^Cancel queued command/,
    });
    fireEvent.click(cancel);
    await screen.findByRole("alert");
    await waitFor(() => expect(cancel).toBeEnabled());
    fireEvent.click(cancel);
    await waitFor(() => expect(actions.cancelQueued).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    expect(actions.updatePendingDelivery).not.toHaveBeenCalled();
    expect(post).not.toHaveBeenCalled();
    post.mockRestore();
  });
  it("keeps failed user-message edit in the queue without restoring text", async () => {
    const post = vi.spyOn(window, "postMessage");
    actions.cancelQueued.mockResolvedValueOnce(false);
    const { store } = render(
      <QueueRow
        queuedItem={{
          ...item,
          command_type: "user_message",
          content: "edit me",
        }}
        position={1}
      />,
    );
    const edit = screen.getByRole("button", {
      name: "Click to edit queued message",
    });
    fireEvent.click(edit);
    await screen.findByRole("alert");
    expect(post).not.toHaveBeenCalled();
    expect(getDraftMessage(store.getState().chat.current_thread_id)).toBe("");
    await waitFor(() => expect(edit).toBeEnabled());
    fireEvent.click(edit);
    await waitFor(() =>
      expect(getDraftMessage(store.getState().chat.current_thread_id)).toBe(
        "edit me",
      ),
    );
    expect(post).not.toHaveBeenCalled();
    post.mockRestore();
  });
  it("preserves the original thread draft when an edit completes after a thread switch", async () => {
    let resolveCancel!: (ok: boolean) => void;
    actions.cancelQueued.mockReturnValue(
      new Promise<boolean>((resolve) => {
        resolveCancel = resolve;
      }),
    );
    saveDraftMessage("original", "unfinished draft");
    saveDraftMessage("other", "other thread draft");
    const post = vi.spyOn(window, "postMessage");
    const { rerender } = render(
      <ChatThreadProvider chatId="original">
        <QueueRow
          queuedItem={{
            ...item,
            command_type: "user_message",
            priority: true,
            content: "queued text to edit",
          }}
          position={1}
        />
      </ChatThreadProvider>,
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Click to edit queued message" }),
    );
    expect(getDraftMessage("original")).toBe("unfinished draft");
    rerender(
      <ChatThreadProvider chatId="other">
        <div>Other chat</div>
      </ChatThreadProvider>,
    );
    resolveCancel(true);
    await waitFor(() =>
      expect(getDraftMessage("original")).toBe(
        "unfinished draft\n\nqueued text to edit",
      ),
    );
    expect(getDraftMessage("other")).toBe("other thread draft");
    expect(post).not.toHaveBeenCalled();
    expect(actions.cancelQueued).toHaveBeenCalledTimes(1);
  });
  it("does not resubmit a queued item after it is dequeued", async () => {
    const post = vi.spyOn(window, "postMessage");
    const { unmount } = render(
      <QueueRow
        queuedItem={{
          ...item,
          command_type: "regenerate",
          content: "again",
        }}
        position={1}
      />,
    );
    fireEvent.click(
      screen.getByRole("button", { name: /^Cancel queued command/ }),
    );
    await waitFor(() => expect(actions.cancelQueued).toHaveBeenCalledTimes(1));
    unmount();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(post).not.toHaveBeenCalled();
    expect(actions.setQueuedPriority).not.toHaveBeenCalled();
    expect(actions.updatePendingDelivery).not.toHaveBeenCalled();
    post.mockRestore();
  });
});
