import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "../../utils/test-utils";
import { Markdown } from "./Markdown";
import { InternalLinkProvider } from "../../contexts/InternalLinkContext";
import {
  applyChatEvent,
  createChatWithId,
  switchToThread,
} from "../../features/Chat";
import type { ChatEventEnvelope } from "../../services/refact/chatSubscription";

function snapshotWithLink(chatId: string): ChatEventEnvelope {
  return {
    chat_id: chatId,
    seq: "0",
    type: "snapshot",
    thread: {
      id: chatId,
      title: "Parent chat",
      model: "",
      mode: "AGENT",
      tool_use: "agent",
      boost_reasoning: false,
      context_tokens_cap: null,
      include_project_info: true,
      checkpoints_enabled: false,
      is_title_generated: false,
    },
    runtime: {
      state: "idle",
      paused: false,
      error: null,
      queue_size: 0,
      pause_reasons: [],
      queued_items: [],
    },
    messages: [
      {
        role: "assistant",
        message_id: "assistant-1",
        content:
          "Open the child trajectory: [view](refact://chat/child-chat-1)",
      },
    ],
    background_agents: [],
  };
}

describe("Markdown internal links", () => {
  it("clicking a refact chat link calls the internal link handler", async () => {
    const onInternalLink = vi.fn(() => true);
    const { user } = render(
      <InternalLinkProvider onInternalLink={onInternalLink}>
        <Markdown>
          Open the child trajectory: [view](refact://chat/child-chat-1)
        </Markdown>
      </InternalLinkProvider>,
    );

    const link = screen.getByRole("link", { name: "view" });
    expect(link).toHaveAttribute("href", "refact://chat/child-chat-1");

    await user.click(link);

    expect(onInternalLink).toHaveBeenCalledWith("refact://chat/child-chat-1");
  });

  it("snapshot markdown renders the refact chat link as an internal anchor", () => {
    const chatId = "parent-chat";
    const { store } = render(
      <Markdown>
        Open the child trajectory: [view](refact://chat/child-chat-1)
      </Markdown>,
    );

    store.dispatch(applyChatEvent(snapshotWithLink(chatId)));

    const content =
      store.getState().chat.threads[chatId]?.thread.messages[0]?.content;
    expect(content).toBe(
      "Open the child trajectory: [view](refact://chat/child-chat-1)",
    );

    const link = screen.getByRole("link", { name: "view" });
    expect(link).toHaveAttribute("href", "refact://chat/child-chat-1");
    expect(link).toHaveStyle({ cursor: "pointer" });
    expect(link.outerHTML).toContain("refact://chat/child-chat-1");
  });

  it("refact chat link handler dispatches the chat switch chain", async () => {
    const childChatId = "child-chat-1";
    const { user, store } = render(
      <InternalLinkProvider
        onInternalLink={(url) => {
          if (url !== `refact://chat/${childChatId}`) return false;
          store.dispatch(
            createChatWithId({
              id: childChatId,
              parentId: store.getState().chat.current_thread_id,
              linkType: "subagent",
            }),
          );
          store.dispatch(switchToThread({ id: childChatId }));
          return true;
        }}
      >
        <Markdown>
          Open the child trajectory: [view](refact://chat/child-chat-1)
        </Markdown>
      </InternalLinkProvider>,
    );

    const parentId = store.getState().chat.current_thread_id;
    await user.click(screen.getByRole("link", { name: "view" }));

    await waitFor(() => {
      expect(store.getState().chat.current_thread_id).toBe(childChatId);
    });
    expect(store.getState().chat.threads[childChatId]?.thread.parent_id).toBe(
      parentId,
    );
    expect(store.getState().chat.threads[childChatId]?.thread.link_type).toBe(
      "subagent",
    );
    expect(store.getState().chat.open_thread_ids).toContain(childChatId);
  });
});
