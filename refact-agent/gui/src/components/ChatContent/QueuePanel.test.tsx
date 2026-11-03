vi.hoisted(() => vi.resetModules());
import { beforeEach, expect, it, vi } from "vitest";
import {
  render,
  screen,
  fireEvent,
  waitFor,
  createDefaultChatState,
} from "../../utils/test-utils";
import { queueStatusText } from "./queuePresentation";
import { QueuePanel } from "./QueuePanel";
import { ChatThreadProvider } from "../../features/Chat/Thread/ChatThreadContext";
import type { RecoveryRecord } from "../../services/refact/chatSubscription";
vi.mock("./QueueRow", () => ({
  QueueRow: ({
    position,
    onToggleExpanded,
  }: {
    position: number;
    onToggleExpanded: () => void;
  }) => (
    <button data-testid="row" onClick={onToggleExpanded}>
      {position}
    </button>
  ),
}));
const items = Array.from({ length: 6 }, (_, i) => ({
  client_request_id: String(i),
  command_type: "delivery",
  priority: false,
  preview: "text",
  enqueued_at_ms: i,
}));
beforeEach(() => {
  sessionStorage.clear();
});
it("keeps wire order in a four-row scroll container and status live only on text", () => {
  const { rerender } = render(<QueuePanel queuedItems={items} isBusy />);
  expect(screen.getByTestId("queue-list")).toHaveAttribute(
    "data-visible-rows",
    "4",
  );
  expect(screen.getAllByTestId("row").map((row) => row.textContent)).toEqual([
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
  ]);
  expect(screen.getByRole("status")).toHaveTextContent("6 after step");
  rerender(<QueuePanel queuedItems={items} isBusy={false} />);
  expect(screen.getByRole("status")).toHaveTextContent("6 ready to deliver");
});
it("remembers collapsed state independently when switching threads and remounting", async () => {
  const view = (id: string) => (
    <ChatThreadProvider chatId={id}>
      <QueuePanel queuedItems={items} isBusy />
    </ChatThreadProvider>
  );
  const { rerender, unmount } = render(view("thread-a"));
  const toggle = screen.getByTestId("transcript-toggle");
  expect(toggle).toHaveTextContent("Queued");
  expect(toggle).toHaveAttribute("aria-expanded", "true");
  fireEvent.click(toggle);
  expect(toggle).toHaveAttribute("aria-expanded", "false");
  await waitFor(() => expect(screen.queryByTestId("queue-list")).toBeNull());
  rerender(view("thread-b"));
  expect(screen.getByTestId("queue-list")).toBeVisible();
  rerender(view("thread-a"));
  expect(screen.queryByTestId("queue-list")).toBeNull();
  unmount();
  render(view("thread-a"));
  expect(screen.getByTestId("transcript-toggle")).toHaveAttribute(
    "aria-expanded",
    "false",
  );
});

it("raises the list cap only while a surviving row is expanded", () => {
  const { rerender } = render(<QueuePanel queuedItems={items} isBusy />);
  const list = screen.getByTestId("queue-list");
  expect(list).toHaveAttribute("data-expanded-row", "false");
  fireEvent.click(screen.getAllByTestId("row")[4]);
  expect(list).toHaveAttribute("data-expanded-row", "true");
  fireEvent.click(screen.getAllByTestId("row")[4]);
  expect(list).toHaveAttribute("data-expanded-row", "false");
  fireEvent.click(screen.getAllByTestId("row")[4]);
  rerender(<QueuePanel queuedItems={items.slice(0, 4)} isBusy />);
  expect(list).toHaveAttribute("data-expanded-row", "false");
});

it("announces append and when-idle deliveries during an interruptible wait without changing timing labels", () => {
  expect(
    queueStatusText(
      [
        { ...items[0], push: "append" },
        { ...items[1], push: "when_idle" },
      ],
      true,
      true,
    ),
  ).toBe("2 delivering now");
  expect(queueStatusText(items, false, true)).toBe("6 ready to deliver");
});

it("leaves recovery commands in the recovery panel while retaining unrelated queued messages", () => {
  const chat = createDefaultChatState();
  chat.threads[chat.current_thread_id].recovery = {
    state: "exhausted",
  } as RecoveryRecord;
  const recoveryItem = {
    ...items[0],
    command_type: "recovery_retry",
    client_request_id: "recovery-command",
  };
  const { rerender } = render(
    <QueuePanel queuedItems={[recoveryItem, items[1]]} isBusy={false} />,
    { preloadedState: { chat } },
  );
  expect(screen.getAllByTestId("row")).toHaveLength(1);
  expect(screen.getByRole("status")).toHaveTextContent("1 ready to deliver");
  rerender(<QueuePanel queuedItems={[recoveryItem]} isBusy={false} />);
  expect(screen.queryByText("Queued")).not.toBeInTheDocument();
});
