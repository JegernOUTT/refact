import { describe, expect, test } from "vitest";
import {
  createDefaultChatState,
  render,
  screen,
} from "../../../utils/test-utils";
import { ChatThreadProvider, useThreadId } from "./ChatThreadContext";

function ThreadIdText() {
  const threadId = useThreadId();
  return <div data-testid="thread-id">{threadId}</div>;
}

describe("ChatThreadContext", () => {
  test("provider supplies explicit id to descendants", () => {
    render(
      <ChatThreadProvider chatId="thread-B">
        <ThreadIdText />
      </ChatThreadProvider>,
    );

    expect(screen.getByTestId("thread-id")).toHaveTextContent("thread-B");
  });

  test("useThreadId falls back to the current thread id", () => {
    const chat = createDefaultChatState();
    chat.current_thread_id = "thread-A";

    render(<ThreadIdText />, {
      preloadedState: { chat },
    });

    expect(screen.getByTestId("thread-id")).toHaveTextContent("thread-A");
  });
});
