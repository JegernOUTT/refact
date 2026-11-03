import { describe, expect, it, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import { http, HttpResponse } from "msw";
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
import { useAttachedFiles, useCheckboxes } from "./useCheckBoxes";
import { ideAttachFileToChat } from "../../hooks/useEventBusForApp";
import { server } from "../../utils/mockServer";

function InputProbe({ label }: { label: string }) {
  const [value] = useInputValue(() => undefined);
  return <div data-testid={label}>{value}</div>;
}

function ContextProbe({ label }: { label: string }) {
  const [value, , immediate] = useInputValue(() => undefined);
  const files = useAttachedFiles();
  const { checkboxes } = useCheckboxes();
  return (
    <div data-testid={label} data-send={String(immediate)}>
      {JSON.stringify({
        value,
        files: files.files.map((file) => file.path),
        snippet: checkboxes.selected_lines.checked,
      })}
    </div>
  );
}

describe("useInputValue", () => {
  it("Buddy chats ignore IDE host input, attachments and selected context", async () => {
    const chat = createDefaultChatState();
    const currentId = chat.current_thread_id;
    const runtime = chat.threads[currentId];
    chat.threads.buddy = {
      ...runtime,
      thread: {
        ...runtime.thread,
        id: "buddy",
        buddy_meta: {
          is_buddy_chat: true,
          buddy_chat_kind: "companion",
          workflow_id: null,
        },
      },
    };
    const command = vi.fn();
    server.use(
      http.post("*/v1/chats/:id/commands", async ({ params, request }) => {
        command(String(params.id), await request.json());
        return HttpResponse.json({ status: "queued" });
      }),
    );
    const view = render(
      <>
        <ChatThreadProvider chatId={currentId}>
          <ContextProbe label="source" />
        </ChatThreadProvider>
        <ChatThreadProvider chatId="buddy">
          <ContextProbe label="buddy" />
        </ChatThreadProvider>
      </>,
      {
        preloadedState: {
          chat,
          config: { host: "vscode", lspPort: 8001, themeProps: {} },
          selected_snippet: {
            language: "text",
            code: "source context",
            path: "/source.txt",
            basename: "source.txt",
          },
          active_file: {
            name: "source.txt",
            path: "/source.txt",
            line1: 1,
            line2: 1,
            cursor: 0,
            can_paste: true,
          },
        },
      },
    );
    postMessage(
      setInputValue({
        messages: [{ role: "user", content: "IDE question" }],
        send_immediately: true,
      }),
    );
    await waitFor(() =>
      expect(command).toHaveBeenCalledWith(
        currentId,
        expect.objectContaining({ type: "user_message" }),
      ),
    );
    postMessage(setInputValue({ value: "IDE draft", send_immediately: true }));
    postMessage(ideAttachFileToChat("/intended-for-source.txt"));
    await waitFor(() =>
      expect(view.getByTestId("source")).toHaveTextContent("IDE draft"),
    );
    expect(view.getByTestId("buddy")).toHaveTextContent(
      JSON.stringify({ value: "", files: [], snippet: false }),
    );
    expect(view.getByTestId("buddy")).toHaveAttribute("data-send", "false");
    expect(command.mock.calls.every(([id]) => id === currentId)).toBe(true);
  });

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
