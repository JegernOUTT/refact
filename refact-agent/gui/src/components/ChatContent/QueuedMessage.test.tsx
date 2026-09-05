import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "../../utils/test-utils";
import { QueuedMessage } from "./QueuedMessage";
import {
  describeQueuedItem,
  queuedItemPushMode,
  pushModeTone,
} from "./queuePresentation";
import { updatePendingDelivery } from "../../services/refact/chatCommands";
import type { QueuedItem } from "../../services/refact/chatSubscription";

const fetchMock = vi.fn().mockResolvedValue(new Response("{}"));
function commandBodies() {
  return fetchMock.mock.calls.map(
    (call) =>
      JSON.parse(String((call[1] as RequestInit).body ?? "{}")) as Record<
        string,
        unknown
      >,
  );
}
const item: QueuedItem = {
  client_request_id: "delivery-1",
  command_type: "delivery",
  priority: false,
  preview: "Job done",
  enqueued_at_ms: 1,
};
beforeEach(() => vi.stubGlobal("fetch", fetchMock));
afterEach(() => {
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});

describe("pending delivery", () => {
  it("defaults to B and only changes after authoritative queue props arrive", async () => {
    const { rerender } = render(
      <QueuedMessage queuedItem={item} position={1} />,
    );
    expect(screen.getByTestId("queued-item")).toHaveAttribute(
      "data-push",
      "append",
    );
    fireEvent.click(screen.getByRole("radio", { name: "When idle" }));
    await waitFor(() =>
      expect(commandBodies()).toContainEqual(
        expect.objectContaining({
          delivery_id: "delivery-1",
          push: "when_idle",
        }),
      ),
    );
    expect(screen.getByTestId("queued-item")).toHaveAttribute(
      "data-push",
      "append",
    );
    rerender(
      <QueuedMessage
        queuedItem={{ ...item, push: "when_idle" }}
        position={1}
      />,
    );
    expect(screen.getByTestId("queued-item")).toHaveAttribute(
      "data-push",
      "when_idle",
    );
    fireEvent.click(
      screen.getByRole("button", { name: /Cancel pending delivery/ }),
    );
    await waitFor(() =>
      expect(commandBodies()).toContainEqual(
        expect.objectContaining({ delivery_id: "delivery-1", cancel: true }),
      ),
    );
    expect(screen.getByTestId("queued-item")).toBeInTheDocument();
  });
  it.each([
    { push: "preempt" as const },
    { push: "append" as const },
    { push: "when_idle" as const },
    { cancel: true },
  ])("serializes exact update command %j", async (patch) => {
    const fetch = vi.fn().mockResolvedValue(new Response("{}"));
    vi.stubGlobal("fetch", fetch);
    await updatePendingDelivery("chat-1", "delivery-1", patch, 8001);
    expect(fetch).toHaveBeenCalledTimes(1);
    const args = fetch.mock.calls[0] as [string, RequestInit];
    expect(args[0]).toContain("/v1/chats/chat-1/commands");
    expect(JSON.parse(String(args[1].body))).toEqual({
      type: "update_pending_delivery",
      delivery_id: "delivery-1",
      ...patch,
      client_request_id: expect.any(String) as unknown,
    });
  });
  it("retains legacy priority, edit and cancel controls", async () => {
    render(
      <QueuedMessage
        queuedItem={{
          ...item,
          command_type: "user_message",
          content: "Edit me",
        }}
        position={1}
      />,
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Change to send next" }),
    );
    await waitFor(() =>
      expect(commandBodies()).toContainEqual({ priority: true }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Cancel queued message" }),
      ).toBeEnabled(),
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Click to edit queued message" }),
    );
    await waitFor(() =>
      expect(
        fetchMock.mock.calls.some(
          (call) => (call[1] as RequestInit).method === "DELETE",
        ),
      ).toBe(true),
    );
    expect(
      commandBodies().some((body) => body.type === "update_pending_delivery"),
    ).toBe(false);
  });
  it("accepts omitted optional SSE fields and humanizes unknown kinds", () => {
    expect(queuedItemPushMode(item)).toBe("append");
    expect(pushModeTone("preempt")).toBe("danger");
    expect(
      describeQueuedItem({ ...item, command_type: "future_command" }).title,
    ).toBe("Future command");
    expect(describeQueuedItem({ ...item, preview: "" }).preview).toBe(
      "Agent continuation",
    );
  });
});
