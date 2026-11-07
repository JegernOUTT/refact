import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "../../utils/test-utils";
import { Markdown } from "./Markdown";
import {
  InternalLinkProvider,
  type InternalLinkOrigin,
} from "../../contexts/InternalLinkContext";
import { applyChatEvent } from "../../features/Chat";
import { ChatThreadProvider } from "../../features/Chat/Thread/ChatThreadContext";
import { http, HttpResponse } from "msw";
import { server } from "../../utils/mockServer";
import { useInternalNavigation } from "../../hooks/useInternalNavigation";
import { parseRefactLink } from "../../contexts/internalLinkUtils";
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
  it("renders markdown images through the image viewer", async () => {
    const { user } = render(
      <Markdown>
        ![Architecture](https://example.com/architecture.png)
      </Markdown>,
    );

    const trigger = screen.getByRole("button", {
      name: "Open image: Architecture",
    });
    expect(trigger).toBeInTheDocument();
    await user.click(trigger);
    const viewer = within(
      await screen.findByRole("dialog", { name: "Architecture" }),
    );
    expect(viewer.getByRole("img", { name: "Architecture" })).toHaveAttribute(
      "src",
      "https://example.com/architecture.png",
    );
    await user.click(viewer.getByRole("button", { name: "Zoom in" }));
    expect(viewer.getByLabelText("Image zoom")).toHaveTextContent("125%");
  });

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

    expect(onInternalLink).toHaveBeenCalledWith(
      "refact://chat/child-chat-1",
      undefined,
    );
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

  it("routes once from the explicit originating pane without inventing the global parent", async () => {
    let reads = 0;
    server.use(
      http.get("*/v1/trajectories/child-chat-1", ({ request }) => {
        expect(new URL(request.url).searchParams.get("metadata_only")).toBe(
          "true",
        );
        reads += 1;
        return HttpResponse.json({
          id: "child-chat-1",
          title: "Child",
          mode: "AGENT",
          model: "",
          parent_id: "origin-pane",
          link_type: "subagent",
          root_chat_id: "origin-root",
        });
      }),
    );
    const origins: (InternalLinkOrigin | undefined)[] = [];
    function LinkedPane() {
      const navigation = useInternalNavigation();
      return (
        <InternalLinkProvider
          onInternalLink={(url, origin) => {
            origins.push(origin);
            return navigation.navigate(url, origin);
          }}
        >
          <ChatThreadProvider chatId="origin-pane">
            <Markdown>[view](refact://chat/child-chat-1)</Markdown>
          </ChatThreadProvider>
        </InternalLinkProvider>
      );
    }
    const { user, store } = render(<LinkedPane />);
    const globalParent = store.getState().chat.current_thread_id;
    await user.click(screen.getByRole("link", { name: "view" }));
    await waitFor(() =>
      expect(store.getState().chat.current_thread_id).toBe("child-chat-1"),
    );
    expect(globalParent).not.toBe("origin-pane");
    expect(origins).toEqual([{ chatId: "origin-pane" }]);
    expect(store.getState().chat.threads["child-chat-1"]?.thread).toMatchObject(
      {
        parent_id: "origin-pane",
        link_type: "subagent",
        root_chat_id: "origin-root",
      },
    );
    expect(
      store.getState().chat.threads["child-chat-1"]?.thread.goal,
    ).toBeUndefined();
    expect(reads).toBe(1);
  });

  it("reports false async handling and keeps external HTTPS anchors ordinary", async () => {
    const handler = vi.fn(() => Promise.resolve(false));
    const { user } = render(
      <InternalLinkProvider onInternalLink={handler}>
        <Markdown>
          [blocked](refact://card/T-1) [external](https://example.com/docs)
        </Markdown>
      </InternalLinkProvider>,
    );
    await user.click(screen.getByRole("link", { name: "blocked" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "current view has been kept",
    );
    expect(handler).toHaveBeenCalledOnce();
    expect(screen.getByRole("link", { name: "external" })).toHaveAttribute(
      "href",
      "https://example.com/docs",
    );
    expect(screen.getByRole("link", { name: "external" })).toHaveAttribute(
      "target",
      "_blank",
    );
    expect(parseRefactLink("refact://task/task-1/card/T-1")).toEqual({
      type: "task",
      id: "task-1",
      cardId: "T-1",
    });
    expect(parseRefactLink("refact://card/T-1")).toBeNull();
    expect(parseRefactLink("refact://chat/../other")).toBeNull();
    expect(parseRefactLink("refact://chat/id%2Fother")).toBeNull();
  });
});
