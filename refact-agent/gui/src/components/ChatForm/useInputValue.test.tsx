import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import { act } from "react-dom/test-utils";
import { handOffQueuedMessageEdit } from "../../utils/queuedMessageEdit";
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

  it("restores queued edits as drafts without auto-send", async () => {
    localStorage.clear();
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    let setSend: ((value: boolean) => void) | null = null;
    function SendProbe() {
      const [value, , isSendImmediately, setIsSendImmediately] = useInputValue(
        () => undefined,
      );
      setSend = setIsSendImmediately;
      return (
        <div data-testid="probe" data-send={String(isSendImmediately)}>
          {value}
        </div>
      );
    }
    render(
      <ChatThreadProvider chatId={chatId}>
        <SendProbe />
      </ChatThreadProvider>,
      { preloadedState: { chat } },
    );
    const probe = () => document.querySelector('[data-testid="probe"]');

    act(() => setSend?.(true));
    expect(probe()).toHaveAttribute("data-send", "true");
    act(() => handOffQueuedMessageEdit(chatId, "priority text"));

    await waitFor(() => {
      expect(probe()).toHaveTextContent("priority text");
    });
    expect(probe()).toHaveAttribute("data-send", "false");
  });
});
