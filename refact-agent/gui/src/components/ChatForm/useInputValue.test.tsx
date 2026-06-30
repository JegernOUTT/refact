import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import {
  createDefaultChatState,
  postMessage,
  render,
} from "../../utils/test-utils";
import { ChatThreadProvider } from "../../features/Chat/Thread";
import { setInputValue } from "./actions";
import { useInputValue } from "./useInputValue";

function InputProbe({ label }: { label: string }) {
  const [value] = useInputValue(() => undefined);
  return <div data-testid={label}>{value}</div>;
}

describe("useInputValue", () => {
  it("ignores thread-targeted input events for other panes", async () => {
    const chat = createDefaultChatState();
    const currentId = chat.current_thread_id;
    const currentRuntime = chat.threads[currentId];
    chat.threads["thread-b"] = {
      ...currentRuntime,
      thread: { ...currentRuntime.thread, id: "thread-b" },
    };
    chat.open_thread_ids = [currentId, "thread-b"];

    render(
      <>
        <ChatThreadProvider chatId={currentId}>
          <InputProbe label="current" />
        </ChatThreadProvider>
        <ChatThreadProvider chatId="thread-b">
          <InputProbe label="other" />
        </ChatThreadProvider>
      </>,
      { preloadedState: { chat } },
    );

    postMessage(
      setInputValue({
        chatId: "thread-b",
        value: "thread-b draft",
        send_immediately: false,
      }),
    );

    await waitFor(() => {
      expect(document.querySelector('[data-testid="other"]')).toHaveTextContent(
        "thread-b draft",
      );
    });
    expect(document.querySelector('[data-testid="current"]')).toHaveTextContent(
      "",
    );
  });
});
