import { afterEach, describe, expect, it, vi } from "vitest";

import { setUpStore } from "../../app/store";
import {
  createDefaultChatState,
  fireEvent,
  render,
  screen,
  waitFor,
} from "../../utils/test-utils";
import * as clipboard from "../../utils/copyChatHistoryToClipboard";
import { ThreadHistory } from "./ThreadHistory";

const createStore = () => {
  const chat = createDefaultChatState();
  const runtime = chat.threads[chat.current_thread_id];
  runtime.thread.id = "thread-1";
  runtime.thread.title = "Thread One";
  runtime.thread.model = "gpt-4o";
  runtime.thread.messages = [{ role: "user", content: "hello" }];
  chat.current_thread_id = "thread-1";
  chat.open_thread_ids = ["thread-1"];
  chat.threads = { "thread-1": runtime };

  return setUpStore({
    chat,
    config: { host: "web", lspPort: 8001, apiKey: "", themeProps: {} },
  });
};

const renderThreadHistory = () =>
  render(
    <ThreadHistory
      onCloseThreadHistory={() => undefined}
      backFromThreadHistory={() => undefined}
      host="web"
      tabbed={false}
      chatId="thread-1"
    />,
    { store: createStore() },
  );

afterEach(() => {
  vi.restoreAllMocks();
});

describe("ThreadHistory", () => {
  it("surfaces clipboard copy failures", async () => {
    vi.spyOn(clipboard, "copyChatHistoryToClipboard").mockRejectedValue(
      new Error("clipboard failed"),
    );

    renderThreadHistory();
    fireEvent.click(screen.getByRole("button", { name: /copy to clipboard/i }));

    await waitFor(() => {
      expect(
        screen
          .getAllByText((_, element) =>
            Boolean(element?.textContent?.includes("Failed to copy chat history")),
          )
          .at(0),
      ).toBeInTheDocument();
    });
  });

  it("shows copy success information", async () => {
    vi.spyOn(clipboard, "copyChatHistoryToClipboard").mockResolvedValue();

    renderThreadHistory();
    fireEvent.click(screen.getByRole("button", { name: /copy to clipboard/i }));

    await waitFor(() => {
      expect(
        screen
          .getAllByText((_, element) =>
            Boolean(
              element?.textContent?.includes("Chat history copied to clipboard"),
            ),
          )
          .at(0),
      ).toBeInTheDocument();
    });
  });
});
