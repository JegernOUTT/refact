import { useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";
import {
  CompanionConversation,
  type ConversationMemory,
} from "./CompanionConversation";
import {
  createDefaultChatState,
  render,
  screen,
  waitFor,
} from "../../../utils/test-utils";
import {
  goodCaps,
  goodPing,
  goodUser,
  goodPrompts,
  noTools,
  noCommandPreview,
  noCompletions,
  server,
} from "../../../utils/mockServer";
import { selectVisibleChatMountIds } from "../../Connection";
import { addThreadImage, applyChatEvent } from "../../Chat/Thread";
import { act } from "react-dom/test-utils";

vi.mock("../../../components/ChatContent/VirtualizedChatList", () => ({
  VirtualizedChatList: ({
    items,
    renderItem,
  }: {
    items: unknown[];
    renderItem: (item: unknown) => React.ReactNode;
  }) => (
    <div>
      {items.map((item, index) => (
        <div key={index}>{renderItem(item)}</div>
      ))}
    </div>
  ),
}));

function Harness({
  chatId,
  ensure = () => Promise.resolve("created"),
  initialDraft = "",
}: {
  chatId?: string;
  ensure?: () => Promise<string>;
  initialDraft?: string;
}) {
  const [memory, setMemory] = useState<ConversationMemory>({
    chatId,
    draft: initialDraft,
  });
  return (
    <CompanionConversation
      sourceId="source"
      memory={memory}
      ensure={ensure}
      onChange={(patch) => setMemory((previous) => ({ ...previous, ...patch }))}
    />
  );
}

beforeEach(() => {
  server.use(
    goodCaps,
    goodPing,
    goodUser,
    goodPrompts,
    noTools,
    noCommandPreview,
    noCompletions,
    http.get("*/v1/chat-modes", () =>
      HttpResponse.json({ modes: [], errors: [] }),
    ),
    http.get("*/v1/voice/status", () =>
      HttpResponse.json({ available: false }),
    ),
    http.get("*/v1/worktrees", () => HttpResponse.json({ worktrees: [] })),
  );
});

describe("CompanionConversation shared chat", () => {
  it.each([false, true])(
    "cleans only submitted attachments after confirmation (remount=%s)",
    async (remount) => {
      const chat = createDefaultChatState();
      const id = chat.current_thread_id;
      chat.threads[id].thread.model = "openai/gpt-4o";
      const image = {
        name: "sent.png",
        type: "image/png",
        content: "data:image/png;base64,c2VudA==",
      };
      const newer = {
        ...image,
        name: "new.png",
        content: "data:image/png;base64,bmV3",
      };
      chat.threads[id].attached_images = [image];
      const command = vi.fn();
      server.use(
        http.post("*/v1/chats/:id/commands", async ({ request }) => {
          command(await request.json());
          return HttpResponse.json({ status: "queued" });
        }),
      );
      let memory: ConversationMemory = { chatId: id, draft: "" };
      const onChange = (patch: Partial<ConversationMemory>) => {
        memory = { ...memory, ...patch };
      };
      const element = () => (
        <CompanionConversation
          sourceId="source"
          memory={memory}
          ensure={() => Promise.resolve(id)}
          onChange={onChange}
        />
      );
      let view = render(element(), {
        preloadedState: {
          chat,
          config: { host: "vscode", lspPort: 8001, themeProps: {} },
        },
      });
      await waitFor(() =>
        expect(screen.getByLabelText("Send message")).toBeEnabled(),
      );
      await view.user.click(screen.getByLabelText("Send message"));
      await waitFor(() => expect(command).toHaveBeenCalledTimes(1));
      const sent = command.mock.calls[0][0] as {
        client_request_id: string;
        content: never;
      };
      act(() => {
        view.store.dispatch(addThreadImage({ id, image: newer }));
      });
      if (remount) {
        const store = view.store;
        view.unmount();
        view = render(element(), { store });
      }
      act(() => {
        view.store.dispatch(
          applyChatEvent({
            type: "message_added",
            chat_id: id,
            seq: "1",
            index: 0,
            message: { role: "user", content: sent.content },
          }),
        );
        view.store.dispatch(
          applyChatEvent({
            type: "ack",
            chat_id: id,
            seq: "2",
            client_request_id: sent.client_request_id,
            accepted: true,
            result: null,
          }),
        );
      });
      await waitFor(() =>
        expect(view.store.getState().chat.threads[id]?.attached_images).toEqual(
          [newer],
        ),
      );
      expect(command).toHaveBeenCalledTimes(1);
    },
  );

  it("recovers a rejected in-flight request after minimize without sending again", async () => {
    const chat = createDefaultChatState();
    const id = chat.current_thread_id;
    chat.threads[id].thread.model = "openai/gpt-4o";
    const command = vi.fn();
    server.use(
      http.post("*/v1/chats/:id/commands", async ({ request }) => {
        command(await request.json());
        return HttpResponse.json({ status: "queued" });
      }),
    );
    let memory: ConversationMemory = { chatId: id, draft: "original" };
    const onChange = (patch: Partial<ConversationMemory>) => {
      memory = { ...memory, ...patch };
    };
    const element = () => (
      <CompanionConversation
        sourceId="source"
        memory={memory}
        ensure={() => Promise.resolve(id)}
        onChange={onChange}
      />
    );
    const view = render(element(), {
      preloadedState: {
        chat,
        config: { host: "vscode", lspPort: 8001, themeProps: {} },
      },
    });
    await waitFor(() =>
      expect(screen.getByLabelText("Send message")).toBeEnabled(),
    );
    await view.user.click(screen.getByLabelText("Send message"));
    await waitFor(() => expect(command).toHaveBeenCalledTimes(1));
    const sent = command.mock.calls[0][0] as { client_request_id: string };
    view.unmount();
    memory = { ...memory, draft: "newer draft" };
    view.store.dispatch(
      applyChatEvent({
        type: "ack",
        chat_id: id,
        seq: "1",
        client_request_id: sent.client_request_id,
        accepted: false,
        result: { error: "Rejected safely" },
      }),
    );
    const reopened = render(element(), { store: view.store });
    await waitFor(() => expect(memory.delivery).toBeUndefined());
    reopened.rerender(element());
    expect(screen.getByTestId("chat-form-textarea")).toHaveValue("newer draft");
    expect(screen.getByText(/Rejected safely/)).toBeInTheDocument();
    expect(command).toHaveBeenCalledTimes(1);
  });

  it("uses the actual lazy composer without creating history on typing or duplicate submits", async () => {
    const ensure = vi.fn(() => new Promise<string>(() => undefined));
    const command = vi.fn();
    server.use(
      http.post("*/v1/chats/:id/commands", () => {
        command();
        return HttpResponse.json({ status: "queued" });
      }),
    );
    const { user } = render(<Harness ensure={ensure} />, {
      preloadedState: {
        config: { host: "vscode", lspPort: 8001, themeProps: {} },
      },
    });
    const textarea = screen.getByTestId("chat-form-textarea");
    await user.type(textarea, "Ask about this source");
    expect(ensure).not.toHaveBeenCalled();
    expect(command).not.toHaveBeenCalled();
    await waitFor(() =>
      expect(screen.getByLabelText("Send message")).toBeEnabled(),
    );
    await user.keyboard("{Enter}{Enter}");
    expect(ensure).toHaveBeenCalledTimes(1);
    expect(textarea).toHaveValue("Ask about this source");
    expect(command).not.toHaveBeenCalled();
  });

  it("renders shared transcript and question form while keeping source draft independent of answers", async () => {
    const chat = createDefaultChatState();
    const id = chat.current_thread_id;
    const runtime = chat.threads[id];
    runtime.thread.model = "openai/gpt-4o";
    runtime.thread.messages = [
      {
        role: "assistant",
        content: "Buddy shared response",
        tool_calls: [
          {
            id: "ask-buddy",
            index: 0,
            function: { name: "ask_questions", arguments: "{}" },
          },
        ],
      },
    ];
    runtime.thread.pending_questions = [
      {
        tool_call_id: "ask-buddy",
        asked_at_ms: 1,
        status: "pending",
        questions: [
          {
            id: "scope",
            type: "single_select",
            text: "Which source scope?",
            options: ["This chat", "Project"],
          },
        ],
      },
    ];
    const command = vi.fn();
    server.use(
      http.post("*/v1/chats/:id/commands", async ({ request }) => {
        command(await request.json());
        return HttpResponse.json({ status: "queued" });
      }),
    );
    const view = render(<Harness chatId={id} initialDraft="source draft" />, {
      preloadedState: {
        chat,
        config: {
          host: "web",
          tabbed: true,
          lspPort: 8001,
          themeProps: {},
          capabilities: { terminalPanel: true },
        },
      },
    });
    expect(
      await screen.findByText("Buddy shared response"),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("radiogroup", { name: "Which source scope?" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Submit Answers" }),
    ).toBeDisabled();
    const textarea = screen.getByTestId("chat-form-textarea");
    await view.user.type(textarea, " edited");
    await view.user.click(screen.getByRole("radio", { name: "This chat" }));
    expect(textarea).toHaveValue("source draft edited");
    expect(command).not.toHaveBeenCalled();
    expect(selectVisibleChatMountIds(view.store.getState())).toContain(id);
    expect(
      view.container.querySelector('[class*="terminalWorkbench"]'),
    ).toBeNull();
    view.unmount();
    render(<Harness chatId={id} initialDraft="source draft edited" />, {
      store: view.store,
    });
    expect(screen.getByTestId("chat-form-textarea")).toHaveValue(
      "source draft edited",
    );
  });

  it("sends image-only shared attachments without changing the selected mode", async () => {
    const chat = createDefaultChatState();
    const id = chat.current_thread_id;
    chat.threads[id].thread.model = "openai/gpt-4o";
    chat.threads[id].attached_images = [
      {
        name: "image.png",
        type: "image/png",
        content: "data:image/png;base64,aGVsbG8=",
      },
    ];
    const command = vi.fn();
    server.use(
      http.post("*/v1/chats/:id/commands", async ({ request }) => {
        command(await request.json());
        return HttpResponse.json({ status: "queued" });
      }),
    );
    const view = render(<Harness chatId={id} />, {
      preloadedState: {
        chat,
        config: { host: "vscode", lspPort: 8001, themeProps: {} },
      },
    });
    await waitFor(() =>
      expect(screen.getByLabelText("Send message")).toBeEnabled(),
    );
    await view.user.click(screen.getByLabelText("Send message"));
    await waitFor(() =>
      expect(command).toHaveBeenCalledWith(
        expect.objectContaining({
          type: "user_message",
          content: expect.arrayContaining([
            {
              type: "image_url",
              image_url: { url: "data:image/png;base64,aGVsbG8=" },
            },
          ]) as unknown,
        }),
      ),
    );
    expect(command).toHaveBeenCalledTimes(1);
  });

  it("keeps an unconfirmed delivery locked across a remount and never resends it", async () => {
    const chat = createDefaultChatState();
    const id = chat.current_thread_id;
    chat.threads[id].thread.model = "openai/gpt-4o";
    const command = vi.fn();
    server.use(
      http.post("*/v1/chats/:id/commands", () => {
        command();
        return HttpResponse.json({ status: "queued" });
      }),
    );
    const memory: ConversationMemory = {
      chatId: id,
      draft: "keep me",
      queuedDraft: "old send",
      delivery: { content: "old send", before: 0, draft: "old send" },
    };
    const view = render(
      <CompanionConversation
        sourceId="source"
        memory={memory}
        ensure={() => Promise.resolve(id)}
        onChange={vi.fn()}
      />,
      {
        preloadedState: {
          chat,
          config: { host: "vscode", lspPort: 8001, themeProps: {} },
        },
      },
    );
    expect(
      screen.getByText(/Message delivery is not confirmed/),
    ).toBeInTheDocument();
    const textarea = screen.getByTestId("chat-form-textarea");
    expect(textarea).toHaveValue("keep me");
    expect(textarea).not.toHaveAttribute("readonly");
    expect(screen.getByLabelText("Send message")).toBeDisabled();
    await view.user.click(
      screen.getByRole("button", { name: "Check message status" }),
    );
    expect(view.store.getState().chat.sse_refresh_requested).toBe(id);
    expect(command).not.toHaveBeenCalled();
  });

  it("preserves typing made while a queued lazy draft was opening", async () => {
    const chat = createDefaultChatState();
    const id = chat.current_thread_id;
    chat.threads[id].thread.model = "openai/gpt-4o";
    let memory: ConversationMemory = {
      chatId: id,
      draft: "new draft",
      queuedDraft: "first",
      queuedDraftValue: "first",
    };
    const command = vi.fn();
    server.use(
      http.post("*/v1/chats/:id/commands", async ({ request }) => {
        const body: unknown = await request.json();
        command(body);
        return HttpResponse.json({ status: "queued" });
      }),
    );
    const onChange = vi.fn((patch: Partial<ConversationMemory>) => {
      memory = { ...memory, ...patch };
    });
    const element = () => (
      <CompanionConversation
        sourceId="source"
        memory={memory}
        ensure={() => Promise.resolve(id)}
        onChange={onChange}
      />
    );
    const view = render(element(), {
      preloadedState: {
        chat,
        config: { host: "vscode", lspPort: 8001, themeProps: {} },
      },
    });
    await waitFor(() => expect(command).toHaveBeenCalledTimes(1));
    view.rerender(element());
    expect(screen.getByTestId("chat-form-textarea")).toHaveValue("new draft");
    const sent = command.mock.calls[0][0] as { client_request_id: string };
    act(() => {
      view.store.dispatch(
        applyChatEvent({
          type: "message_added",
          chat_id: id,
          seq: "1",
          index: 0,
          message: { role: "user", content: "first" },
        }),
      );
      view.store.dispatch(
        applyChatEvent({
          type: "ack",
          chat_id: id,
          seq: "2",
          client_request_id: sent.client_request_id,
          accepted: true,
          result: null,
        }),
      );
    });
    await waitFor(() =>
      expect(onChange).toHaveBeenCalledWith({
        error: undefined,
        queuedDraft: undefined,
      }),
    );
    expect(memory.draft).toBe("new draft");
    expect(command).toHaveBeenCalledTimes(1);
  });
});
