import { describe, expect, it } from "vitest";
import { act } from "react-dom/test-utils";
import {
  createDefaultChatState,
  render,
  screen,
  within,
} from "../../utils/test-utils";
import { applyChatEvent } from "../../features/Chat/Thread";
import type { ToolCall, ToolMessage } from "../../services/refact/types";
import { COMPACT_HEADLINE_CHARS } from "../Tools/AgenticTool/compactPresentation";
import { CollapsibleStoreProvider } from "./useStoredOpen";
import { TaskDocumentsContent, TaskDocumentsView } from "./TaskDocumentsView";

const DOC_LIST = [
  "| slug | name | kind | pinned | version | updated_at |",
  "|---|---|---|---|---:|---|",
  "| main-plan | Main Plan | plan | true | 3 | 2026-05-22T10:00:00Z |",
  "| api-spec | API Spec | spec | false | 1 | 2026-05-22T11:00:00Z |",
  "| runbook | Deploy Runbook | runbook | true | 2 | 2026-05-22T12:00:00Z |",
].join("\n");

const DOC_GET = [
  "---",
  "name: Main Plan",
  "slug: main-plan",
  "kind: plan",
  "pinned: true",
  "version: 3",
  "---",
  "",
  "# Main Plan",
  "",
  "- Ship document renderer",
].join("\n");

const UNSAFE_DOC_GET = [
  "---",
  "name: Unsafe Document",
  "slug: unsafe-document",
  "kind: plan",
  "pinned: false",
  "version: 1",
  "---",
  "",
  "# Unsafe Document",
  "",
  "<script>window.x=1</script>",
  '<img onerror="alert(1)" src=x>',
].join("\n");

type ToolType = "doc_get" | "doc_list";

function makeToolCall(
  toolType: ToolType,
  args = '{"slug":"requested-plan"}',
): ToolCall {
  return {
    id: `call-${toolType}`,
    index: 0,
    function: { name: toolType, arguments: args },
  };
}

function documentToolState(messages: ToolMessage[] = [], ownerCall?: ToolCall) {
  const chat = createDefaultChatState();
  const threadId = chat.current_thread_id;
  chat.threads[threadId].thread.messages =
    messages.length === 0 && ownerCall
      ? [{ role: "assistant", content: "", tool_calls: [ownerCall] }]
      : messages;
  return { chat, threadId };
}

function renderDocumentTool(
  toolType: ToolType,
  content?: ToolMessage["content"],
  {
    failed = false,
    args,
    streaming = false,
    waiting = false,
  }: {
    failed?: boolean;
    args?: string;
    streaming?: boolean;
    waiting?: boolean;
  } = {},
) {
  const toolCall = makeToolCall(toolType, args);
  const { chat, threadId } = documentToolState(
    content === undefined
      ? []
      : [
          {
            role: "tool",
            tool_call_id: toolCall.id ?? "",
            content,
            tool_failed: failed,
          },
        ],
    toolCall,
  );
  chat.threads[threadId].streaming = streaming;
  chat.threads[threadId].waiting_for_response = waiting;
  return {
    ...render(<TaskDocumentsView toolType={toolType} toolCall={toolCall} />, {
      preloadedState: { chat },
    }),
    threadId,
    toolCall,
  };
}

describe("TaskDocumentsContent", () => {
  it("parses doc_list markdown into rows and renders them", () => {
    render(<TaskDocumentsContent toolType="doc_list" content={DOC_LIST} />);

    expect(screen.getByText("Main Plan")).toBeInTheDocument();
    expect(screen.getByText("API Spec")).toBeInTheDocument();
    expect(screen.getByText("Deploy Runbook")).toBeInTheDocument();
    expect(screen.queryByText("Task documents")).not.toBeInTheDocument();
    expect(screen.getAllByRole("article")).toHaveLength(3);
  });

  it("parses doc_list alias column names and skips rows without slugs", () => {
    render(
      <TaskDocumentsContent
        toolType="doc_list"
        content={[
          "| slug | title | kind | pinned | version | updated at |",
          "|---|---|---|---|---:|---|",
          "| aliased-plan | Aliased Plan | plan | yes | 4 | 2026-05-22T13:00:00Z |",
          "|  | Missing Slug | brief | true | 1 | 2026-05-22T14:00:00Z |",
        ].join("\n")}
      />,
    );

    expect(screen.getByText("Aliased Plan")).toBeInTheDocument();
    expect(screen.getByText("2026-05-22T13:00:00Z")).toBeInTheDocument();
    expect(screen.getAllByRole("article")).toHaveLength(1);
    expect(screen.queryByText("Missing Slug")).not.toBeInTheDocument();
  });

  it("parses updated alias in doc_list markdown", () => {
    render(
      <TaskDocumentsContent
        toolType="doc_list"
        content={[
          "| slug | name | kind | pinned | version | updated |",
          "|---|---|---|---|---:|---|",
          "| updated-plan | Updated Plan | plan | no | 1 | 2026-05-22T15:00:00Z |",
        ].join("\n")}
      />,
    );

    expect(screen.getByText("Updated Plan")).toBeInTheDocument();
    expect(screen.getByText("2026-05-22T15:00:00Z")).toBeInTheDocument();
  });

  it("renders raw doc_list markdown without a notice when no rows parse", () => {
    const { container } = render(
      <TaskDocumentsContent toolType="doc_list" content="No documents found" />,
    );

    expect(screen.getByText("No documents found")).toBeInTheDocument();
    expect(container.textContent).toBe("No documents found");
    expect(container.querySelector("article")).toBeNull();
  });

  it("renders only the doc_get body markdown", () => {
    const { container } = render(
      <TaskDocumentsContent toolType="doc_get" content={DOC_GET} />,
    );

    expect(
      screen.getByRole("heading", { name: "Main Plan" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Ship document renderer")).toBeInTheDocument();
    expect(screen.queryByText("main-plan")).not.toBeInTheDocument();
    expect(screen.queryByText("v3")).not.toBeInTheDocument();
    expect(container.textContent).not.toContain("slug:");
  });

  it("renders unparsed doc_get output as raw markdown without a notice", () => {
    const { container } = render(
      <TaskDocumentsContent toolType="doc_get" content="Legacy body" />,
    );

    expect(container.textContent).toBe("Legacy body");
  });

  it("does not render raw scripts or inline image handlers from doc_get markdown", () => {
    const { container } = render(
      <TaskDocumentsContent toolType="doc_get" content={UNSAFE_DOC_GET} />,
    );

    expect(container.querySelector("script")).not.toBeInTheDocument();
    expect(container.querySelector("img[onerror]")).not.toBeInTheDocument();
  });
});

function itemOf(toggle: HTMLElement): HTMLElement {
  const section = toggle.closest("section");
  if (!section) throw new Error("expected transcript section");
  return section;
}

function rowParts(toggle: HTMLElement) {
  return {
    verb: within(toggle).queryByTestId("transcript-verb")?.textContent ?? null,
    target:
      within(toggle).queryByTestId("transcript-target")?.textContent ?? null,
    meta: within(toggle).queryByTestId("transcript-meta")?.textContent ?? null,
  };
}

function factMap(): Record<string, string> {
  const facts = screen.queryByTestId("transcript-facts");
  if (!facts) return {};
  return Object.fromEntries(
    Array.from(facts.querySelectorAll("dt")).map((dt) => [
      dt.textContent ?? "",
      dt.nextElementSibling?.textContent ?? "",
    ]),
  );
}

function rawTexts(): string[] {
  return Array.from(
    screen.getByTestId("transcript-raw").querySelectorAll("pre"),
  ).map((pre) => pre.textContent ?? "");
}

describe("TaskDocumentsView transcript route", () => {
  it("starts doc_get closed with the document name and version, revealing facts and body when opened", async () => {
    const { user } = renderDocumentTool("doc_get", DOC_GET);
    const toggle = screen.getByTestId("transcript-toggle");

    expect(screen.getByTestId("task-documents-view")).toBe(itemOf(toggle));
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(rowParts(toggle)).toEqual({
      verb: "Document",
      target: "Main Plan",
      meta: "v3",
    });
    expect(
      screen.queryByTestId("transcript-status-pill"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText("Ship document renderer"),
    ).not.toBeInTheDocument();

    toggle.focus();
    await user.keyboard("{Enter}");

    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(factMap()).toEqual({
      Slug: "main-plan",
      Version: "3",
      Kind: "plan",
    });
    expect(screen.getByRole("heading", { name: "Main Plan" })).toBeVisible();
    expect(screen.getByText("Ship document renderer")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Raw" }));
    expect(rawTexts()).toEqual([
      JSON.stringify({ slug: "requested-plan" }, null, 2),
      DOC_GET,
    ]);
  });

  it("starts doc_list closed with an observed count, retaining all rows on demand", async () => {
    const { container, user } = renderDocumentTool("doc_list", DOC_LIST);
    const toggle = screen.getByTestId("transcript-toggle");

    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(rowParts(toggle)).toEqual({
      verb: "Documents",
      target: "3 documents",
      meta: null,
    });
    expect(screen.queryByText("API Spec")).not.toBeInTheDocument();
    await user.click(toggle);

    expect(screen.queryByTestId("transcript-facts")).not.toBeInTheDocument();
    expect(screen.queryByText("Task documents")).not.toBeInTheDocument();
    expect(container.querySelectorAll("article")).toHaveLength(3);
    expect(screen.getByText("API Spec")).toBeVisible();
    expect(screen.getByText("Deploy Runbook")).toBeVisible();
    expect(screen.getByLabelText("Pinned main-plan")).toHaveTextContent("★");
    expect(screen.getByLabelText("Not pinned api-spec")).toHaveTextContent("☆");
  });

  it("uses existing list alias and missing-slug guards for the observed count", async () => {
    const { user } = renderDocumentTool(
      "doc_list",
      [
        "| slug | title | kind | pinned | version | updated at |",
        "|---|---|---|---|---:|---|",
        "| aliased-plan | Aliased Plan | plan | yes | 4 | yesterday |",
        "| | Missing Slug | brief | true | 1 | today |",
      ].join("\n"),
    );
    const toggle = screen.getByTestId("transcript-toggle");
    expect(rowParts(toggle).target).toBe("1 document");
    await user.click(toggle);
    expect(screen.getByText("Aliased Plan")).toBeInTheDocument();
    expect(screen.queryByText("Missing Slug")).not.toBeInTheDocument();
  });

  it.each([
    ["slug: payload-slug", "payload-slug", null],
    ["name: \nslug: payload-slug", "payload-slug", null],
    ["version: 7", "requested-plan", "v7"],
  ])(
    "uses parsed metadata without inventing a name: %s",
    (metadata, target, meta) => {
      renderDocumentTool("doc_get", `---\n${metadata}\n---\nBody`);
      const toggle = screen.getByTestId("transcript-toggle");
      expect(rowParts(toggle)).toEqual({ verb: "Document", target, meta });
    },
  );

  it.each([
    "# A prose title is not parsed document metadata\n\nLegacy body",
    "---\nname: Unclosed metadata\nLegacy body",
    "This read succeeded and verified all documents",
    "",
  ])(
    "keeps unknown doc_get output neutral and recoverable: %s",
    async (content) => {
      const { user } = renderDocumentTool("doc_get", content);
      const toggle = screen.getByTestId("transcript-toggle");
      expect(toggle).toHaveAttribute("aria-expanded", "false");
      expect(rowParts(toggle)).toEqual({
        verb: "Document",
        target: "requested-plan",
        meta: null,
      });
      await user.click(toggle);
      expect(factMap()).toEqual({ Slug: "requested-plan" });
      const output = screen.getByRole("group", { name: "Output" });
      if (content !== "")
        expect(output.textContent).toContain(
          content.includes("Legacy body") ? "Legacy body" : content,
        );
      await user.click(screen.getByRole("button", { name: "Raw" }));
      expect(rawTexts()[1]).toBe(content);
    },
  );

  it.each([
    "No documents found",
    "3 documents completed successfully",
    "| name | kind |\n|---|---|\n| Main Plan | plan |",
    "| slug | name | kind |\n|---|---|---|",
    "",
  ])(
    "never turns malformed or uncounted list output into a confirmed count: %s",
    async (content) => {
      const { container, user } = renderDocumentTool("doc_list", content);
      const toggle = screen.getByTestId("transcript-toggle");
      expect(toggle).toHaveAttribute("aria-expanded", "false");
      expect(rowParts(toggle)).toEqual({
        verb: "Documents",
        target: null,
        meta: null,
      });
      await user.click(toggle);
      expect(screen.queryByTestId("transcript-facts")).not.toBeInTheDocument();
      expect(container.querySelector("article")).toBeNull();
    },
  );

  it.each(["doc_get", "doc_list"] as const)(
    "shows failed %s without confirming payload metadata",
    async (toolType) => {
      const content = toolType === "doc_get" ? DOC_GET : DOC_LIST;
      const { user } = renderDocumentTool(toolType, content, {
        failed: true,
      });
      const toggle = screen.getByTestId("transcript-toggle");
      expect(toggle).toHaveAttribute("aria-expanded", "false");
      expect(itemOf(toggle)).toHaveAttribute("data-status", "failed");
      expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
        "Failed",
      );
      expect(rowParts(toggle)).toEqual({
        verb: toolType === "doc_get" ? "Document" : "Documents",
        target: toolType === "doc_get" ? "requested-plan" : null,
        meta: null,
      });
      await user.click(toggle);
      expect(factMap()).toEqual(
        toolType === "doc_get" ? { Slug: "requested-plan" } : {},
      );
      expect(
        screen.queryByRole("group", { name: "Output" }),
      ).not.toBeInTheDocument();
      expect(screen.getByRole("alert")).toHaveTextContent("Main Plan");
    },
  );

  it.each([
    ["doc_get", false, false, "unknown", "No result"],
    ["doc_list", false, false, "unknown", "No result"],
    ["doc_get", true, false, "running", "Running"],
    ["doc_list", false, true, "running", "Running"],
  ] as const)(
    "keeps pending %s closed without a premature result",
    async (toolType, streaming, waiting, status, pillText) => {
      const { user } = renderDocumentTool(toolType, undefined, {
        streaming,
        waiting,
      });
      const toggle = screen.getByTestId("transcript-toggle");
      expect(toggle).toHaveAttribute("aria-expanded", "false");
      expect(itemOf(toggle)).toHaveAttribute("data-status", status);
      expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
        pillText,
      );
      await user.click(toggle);
      expect(
        screen.queryByRole("group", { name: "Output" }),
      ).not.toBeInTheDocument();
      await user.click(screen.getByRole("button", { name: "Raw" }));
      expect(rawTexts()).toHaveLength(1);
    },
  );

  it.each([
    "not-json",
    "null",
    "[]",
    '{"slug":{"name":"not a slug"}}',
    '{"slug":12}',
  ])(
    "handles invalid arguments without fabricating a document identity: %s",
    (args) => {
      renderDocumentTool("doc_get", "Legacy document body", { args });
      const toggle = screen.getByTestId("transcript-toggle");
      expect(toggle).toHaveAttribute("aria-expanded", "false");
      expect(rowParts(toggle)).toEqual({
        verb: "Document",
        target: null,
        meta: null,
      });
    },
  );

  it("keeps non-string results out of the parsed output", async () => {
    const { user } = renderDocumentTool("doc_get", [
      { m_type: "text", m_content: "name: Fake title" },
    ]);
    const toggle = screen.getByTestId("transcript-toggle");
    expect(rowParts(toggle).target).toBe("requested-plan");
    await user.click(toggle);
    expect(factMap()).toEqual({ Slug: "requested-plan" });
    expect(
      screen.queryByRole("group", { name: "Output" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByText(/Fake title/u)).not.toBeInTheDocument();
  });

  it("bounds and escapes hostile metadata while retaining the full large Markdown and safe navigation", async () => {
    const hostile = '<img src=x onerror="alert(1)"><script>bad()</script>';
    const longName = `${hostile}${"🧪".repeat(400)}`;
    const content = [
      "---",
      `name: ${longName}`,
      "slug: hostile-document",
      "kind: plan",
      "version: 9",
      "pinned: true",
      "---",
      UNSAFE_DOC_GET,
      "[Unsafe link](javascript:alert%281%29)",
      "[Read reference](https://example.com/reference)",
      "[Open task](refact://task/task-123)",
      "Large document paragraph. ".repeat(2_000),
      "\n## End of full document",
    ].join("\n\n");
    const { container, user } = renderDocumentTool("doc_get", content);
    const toggle = screen.getByTestId("transcript-toggle");
    const target = within(toggle).getByTestId("transcript-target");
    expect(Array.from(target.textContent ?? "")).toHaveLength(
      COMPACT_HEADLINE_CHARS,
    );
    expect(target.textContent).toContain("…");
    expect(target.textContent).toContain(hostile);
    expect(container.querySelector("script, img[onerror]")).toBeNull();
    expect(
      screen.queryByRole("heading", { name: "End of full document" }),
    ).not.toBeInTheDocument();
    await user.click(toggle);
    expect(factMap()).toEqual({
      Slug: "hostile-document",
      Version: "9",
      Kind: "plan",
    });
    expect(
      screen.getByRole("heading", { name: "End of full document" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: "Read reference" }),
    ).toHaveAttribute("href", "https://example.com/reference");
    expect(
      screen.getByRole("link", { name: "Read reference" }),
    ).not.toHaveAttribute("tabindex", "-1");
    expect(screen.getByRole("link", { name: "Open task" })).toHaveAttribute(
      "href",
      "/#/task/task-123",
    );
    expect(
      container.querySelector('script, img[onerror], a[href^="javascript:"]'),
    ).toBeNull();
  });

  it.each([true, false])(
    "preserves stored choice %s across independent document rows and remounts",
    async (storedOpen) => {
      const getCall = makeToolCall("doc_get");
      const listCall = makeToolCall("doc_list");
      const preferences = new Map([
        [`doc_get:${getCall.id ?? ""}`, storedOpen],
      ]);
      const store = {
        get: (key: string) => preferences.get(key),
        set: (key: string, open: boolean) => {
          preferences.set(key, open);
        },
      };
      const { chat } = documentToolState([
        { role: "tool", tool_call_id: getCall.id ?? "", content: DOC_GET },
        { role: "tool", tool_call_id: listCall.id ?? "", content: DOC_LIST },
      ]);
      const rows = (
        <CollapsibleStoreProvider value={store}>
          <TaskDocumentsView toolType="doc_get" toolCall={getCall} />
          <TaskDocumentsView toolType="doc_list" toolCall={listCall} />
        </CollapsibleStoreProvider>
      );
      const { unmount, user } = render(rows, { preloadedState: { chat } });
      const [read, list] = screen.getAllByTestId("transcript-toggle");
      expect(read).toHaveAttribute("aria-expanded", String(storedOpen));
      expect(list).toHaveAttribute("aria-expanded", "false");
      await user.click(list);
      expect(read).toHaveAttribute("aria-expanded", String(storedOpen));
      await user.click(read);
      expect(list).toHaveAttribute("aria-expanded", "true");
      unmount();
      render(rows, { preloadedState: { chat } });
      const [readAgain, listAgain] = screen.getAllByTestId("transcript-toggle");
      expect(readAgain).toHaveAttribute("aria-expanded", String(!storedOpen));
      expect(listAgain).toHaveAttribute("aria-expanded", "true");
      expect(preferences.get(`doc_list:${listCall.id ?? ""}`)).toBe(true);
    },
  );

  it("does not force expansion on result/status updates", async () => {
    const { store, threadId, toolCall, user } = renderDocumentTool(
      "doc_get",
      undefined,
      { streaming: true },
    );
    const toggle = screen.getByTestId("transcript-toggle");
    expect(itemOf(toggle)).toHaveAttribute("data-status", "running");
    const message: ToolMessage = {
      role: "tool",
      message_id: "document-result",
      tool_call_id: toolCall.id ?? "",
      content: DOC_GET,
      tool_failed: false,
    };
    act(() => {
      store.dispatch(
        applyChatEvent({
          type: "message_added",
          chat_id: threadId,
          seq: "1",
          index: 0,
          message,
        }),
      );
    });
    expect(rowParts(toggle)).toEqual({
      verb: "Document",
      target: "Main Plan",
      meta: "v3",
    });
    expect(itemOf(toggle)).toHaveAttribute("data-status", "done");
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    await user.click(toggle);
    act(() => {
      store.dispatch(
        applyChatEvent({
          type: "message_updated",
          chat_id: threadId,
          seq: "2",
          message_id: "document-result",
          message: {
            ...message,
            content: DOC_GET.replace("version: 3", "version: 4"),
          },
        }),
      );
    });
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(rowParts(toggle).meta).toBe("v4");
    act(() => {
      store.dispatch(
        applyChatEvent({
          type: "message_updated",
          chat_id: threadId,
          seq: "3",
          message_id: "document-result",
          message: { ...message, tool_failed: true },
        }),
      );
    });
    expect(itemOf(toggle)).toHaveAttribute("data-status", "failed");
    expect(rowParts(toggle).target).toBe("requested-plan");
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(factMap()).toEqual({ Slug: "requested-plan" });
    expect(screen.getByRole("alert")).toHaveTextContent("Main Plan");
  });
});
