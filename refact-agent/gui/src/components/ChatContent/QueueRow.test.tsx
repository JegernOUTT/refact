vi.hoisted(() => vi.resetModules());
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "../../utils/test-utils";
import type { QueuedItem } from "../../features/Chat";
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
  actions.cancelQueued.mockResolvedValue(true);
  actions.setQueuedPriority.mockResolvedValue(true);
  actions.updatePendingDelivery.mockResolvedValue(undefined);
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
    const post = vi.spyOn(window, "postMessage");
    render(
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
    await waitFor(() => expect(post).toHaveBeenCalled());
    expect(actions.cancelQueued).toHaveBeenCalledWith("delivery-1");
    post.mockRestore();
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
});
