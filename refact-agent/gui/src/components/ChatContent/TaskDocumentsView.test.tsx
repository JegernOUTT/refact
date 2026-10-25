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

function documentToolState(messages: ToolMessage[] = []) {
  const chat = createDefaultChatState();
  const threadId = chat.current_thread_id;
  chat.threads[threadId].thread.messages = messages;
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
    expect(screen.getByText("3 documents")).toBeInTheDocument();
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
    expect(screen.getByText("1 documents")).toBeInTheDocument();
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

  it("renders raw doc_list markdown when no rows parse", () => {
    render(
      <TaskDocumentsContent toolType="doc_list" content="No documents found" />,
    );

    expect(screen.getByText("No documents found")).toBeInTheDocument();
    expect(
      screen.getByText("Parser produced no rows; raw output below"),
    ).toBeInTheDocument();
    expect(screen.queryByText("0 documents")).not.toBeInTheDocument();
  });

  it("does not render the raw fallback when doc_list rows parse", () => {
    render(<TaskDocumentsContent toolType="doc_list" content={DOC_LIST} />);

    expect(
      screen.queryByText("Parser produced no rows; raw output below"),
    ).not.toBeInTheDocument();
  });

  it("renders doc_get body markdown", () => {
    render(<TaskDocumentsContent toolType="doc_get" content={DOC_GET} />);

    expect(
      screen.getByRole("heading", { name: "Main Plan" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Ship document renderer")).toBeInTheDocument();
    expect(screen.getByText("main-plan")).toBeInTheDocument();
    expect(screen.getByText("v3")).toBeInTheDocument();
  });

  it("does not render raw scripts or inline image handlers from doc_get markdown", () => {
    const { container } = render(
      <TaskDocumentsContent toolType="doc_get" content={UNSAFE_DOC_GET} />,
    );

    expect(container.querySelector("script")).not.toBeInTheDocument();
    expect(container.querySelector("img[onerror]")).not.toBeInTheDocument();
  });

  it("renders pinned and non-pinned stars differently", () => {
    render(<TaskDocumentsContent toolType="doc_list" content={DOC_LIST} />);

    expect(screen.getByLabelText("Pinned main-plan")).toHaveTextContent("★");
    expect(screen.getByLabelText("Not pinned api-spec")).toHaveTextContent("☆");
  });
});

describe("TaskDocumentsView compact dedicated route", () => {
  it("starts doc_get closed with the parsed document name and full metadata behind a second disclosure", async () => {
    const { container, user } = renderDocumentTool("doc_get", DOC_GET);
    const toggle = screen.getByRole("button", {
      name: "Read Main Plan Response received",
    });

    expect(screen.getByTestId("task-documents-view")).toBeInTheDocument();
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(container.querySelector("section")).toHaveAttribute(
      "data-density",
      "compact",
    );
    expect(
      screen.queryByText("Ship document renderer"),
    ).not.toBeInTheDocument();
    expect(screen.queryByText("Task document")).not.toBeInTheDocument();

    toggle.focus();
    await user.keyboard("{Enter}");

    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(container.querySelectorAll("dt")).toHaveLength(3);
    expect(container.querySelectorAll("dd")).toHaveLength(3);
    expect(screen.getByText("Name")).toBeInTheDocument();
    expect(screen.getByText("Version")).toBeInTheDocument();
    expect(screen.getByText("3")).toBeInTheDocument();
    expect(screen.getByText("Kind")).toBeInTheDocument();
    const details = screen.getByRole("button", { name: "More details" });
    expect(details).toHaveAttribute("aria-expanded", "false");
    expect(
      screen.getByText("Ship document renderer").closest("[inert]"),
    ).not.toBeNull();

    details.focus();
    await user.keyboard("{Enter}");

    expect(
      screen.getByText("Ship document renderer").closest("[inert]"),
    ).toBeNull();
    expect(screen.getByRole("heading", { name: "Main Plan" })).toBeVisible();
    expect(screen.getByText("main-plan")).toBeVisible();
    expect(screen.getByText("v3")).toBeVisible();
    expect(screen.getByLabelText("Pinned")).toHaveTextContent("★");
  });

  it("starts doc_list closed with an observed count and explicit response coverage, retaining all rows on demand", async () => {
    const { container, user } = renderDocumentTool("doc_list", DOC_LIST);
    const toggle = screen.getByRole("button", {
      name: "3 documents shown Response received",
    });

    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(container.querySelector("section")).toHaveAttribute(
      "data-density",
      "compact",
    );
    expect(screen.queryByText("API Spec")).not.toBeInTheDocument();
    await user.click(toggle);

    expect(container.querySelectorAll("dt")).toHaveLength(2);
    expect(screen.getByText("Documents shown")).toBeInTheDocument();
    expect(screen.getByText("3")).toBeInTheDocument();
    expect(screen.getByText("Coverage")).toBeInTheDocument();
    expect(
      screen.getByText("Parsed rows in this response"),
    ).toBeInTheDocument();
    expect(screen.getByText("API Spec").closest("[inert]")).not.toBeNull();
    await user.click(screen.getByRole("button", { name: "More details" }));

    expect(container.querySelectorAll("article")).toHaveLength(3);
    expect(screen.getByText("Main Plan").closest("[inert]")).toBeNull();
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
    await user.click(
      screen.getByRole("button", {
        name: "1 document shown Response received",
      }),
    );
    expect(screen.getByText("1")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "More details" }));
    expect(screen.getByText("Aliased Plan")).toBeInTheDocument();
    expect(screen.queryByText("Missing Slug")).not.toBeInTheDocument();
  });

  it.each([
    ["slug: payload-slug", "Read payload-slug Response received"],
    ["name: \nslug: payload-slug", "Read payload-slug Response received"],
    ["version: 7", "Read requested-plan Response received"],
  ])(
    "uses parsed metadata without inventing a name: %s",
    async (metadata, header) => {
      const { user } = renderDocumentTool(
        "doc_get",
        `---\n${metadata}\n---\nBody`,
      );
      await user.click(screen.getByRole("button", { name: header }));
      expect(
        screen.getByRole("button", { name: "More details" }),
      ).toHaveAttribute("aria-expanded", "false");
      if (metadata === "version: 7")
        expect(screen.getByText("7")).toBeInTheDocument();
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
      const { container, user } = renderDocumentTool("doc_get", content);
      const toggle = screen.getByRole("button", {
        name: "Read requested-plan Outcome unknown",
      });
      expect(toggle).toHaveAttribute("aria-expanded", "false");
      await user.click(toggle);
      expect(container.querySelectorAll("dt")).toHaveLength(1);
      expect(screen.getByText("Requested document")).toBeInTheDocument();
      const details = screen.getByRole("button", { name: "More details" });
      expect(details).toHaveAttribute("aria-expanded", "false");
      await user.click(details);
      if (content === "") {
        expect(
          screen.getByText(
            "Parser produced no document details; raw output below",
          ),
        ).toBeInTheDocument();
      } else {
        expect(container.textContent).toContain(
          content.includes("Legacy body") ? "Legacy body" : content,
        );
      }
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
      const toggle = screen.getByRole("button", {
        name: "List task documents Outcome unknown",
      });
      expect(toggle).toHaveAttribute("aria-expanded", "false");
      expect(
        screen.queryByText(/\d+ documents? shown/),
      ).not.toBeInTheDocument();
      await user.click(toggle);
      expect(container.querySelector("dl")).toBeNull();
      await user.click(screen.getByRole("button", { name: "More details" }));
      expect(
        screen.getByText("Parser produced no rows; raw output below"),
      ).toBeInTheDocument();
      expect(container.querySelector("article")).toBeNull();
    },
  );

  it.each(["doc_get", "doc_list"] as const)(
    "shows failed %s while closed without confirming payload metadata",
    async (toolType) => {
      const content = toolType === "doc_get" ? DOC_GET : DOC_LIST;
      const { container, user } = renderDocumentTool(toolType, content, {
        failed: true,
      });
      const summary =
        toolType === "doc_get" ? "Read requested-plan" : "List task documents";
      const toggle = screen.getByRole("button", {
        name: `${summary} Request failed`,
      });
      expect(toggle).toHaveAttribute("aria-expanded", "false");
      expect(container.querySelector("section")).toHaveAttribute(
        "data-status",
        "error",
      );
      expect(screen.queryByText("Main Plan")).not.toBeInTheDocument();
      expect(
        screen.queryByText(/\d+ documents? shown/),
      ).not.toBeInTheDocument();
      await user.click(toggle);
      expect(screen.queryByText("Name")).not.toBeInTheDocument();
      expect(screen.queryByText("Documents shown")).not.toBeInTheDocument();
      expect(screen.queryByText("Version")).not.toBeInTheDocument();
      await user.click(screen.getByRole("button", { name: "More details" }));
      expect(
        screen.getByText("Request failed; raw output below"),
      ).toBeInTheDocument();
      expect(container.textContent).toContain("Main Plan");
    },
  );

  it.each([
    ["doc_get", false, false, "Read requested-plan Awaiting result"],
    ["doc_list", false, false, "List task documents Awaiting result"],
    ["doc_get", true, false, "Read requested-plan Running"],
    ["doc_list", false, true, "List task documents Running"],
  ] as const)(
    "keeps pending %s closed without a premature result disclosure",
    async (toolType, streaming, waiting, header) => {
      const { user } = renderDocumentTool(toolType, undefined, {
        streaming,
        waiting,
      });
      const toggle = screen.getByRole("button", { name: header });
      expect(toggle).toHaveAttribute("aria-expanded", "false");
      await user.click(toggle);
      expect(
        screen.queryByRole("button", { name: "More details" }),
      ).not.toBeInTheDocument();
      expect(screen.queryByText("Documents shown")).not.toBeInTheDocument();
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
      expect(
        screen.getByRole("button", { name: "Read document Outcome unknown" }),
      ).toHaveAttribute("aria-expanded", "false");
    },
  );

  it("keeps non-string results unknown instead of claiming a parsed document", async () => {
    const { user } = renderDocumentTool("doc_get", [
      { m_type: "text", m_content: "name: Fake title" },
    ]);
    await user.click(
      screen.getByRole("button", {
        name: "Read requested-plan Outcome unknown",
      }),
    );
    expect(screen.queryByText("Name")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "More details" }),
    ).not.toBeInTheDocument();
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
    const toggle = screen.getByRole("button");
    const summary = within(toggle).getByText(/^Read /);
    expect(Array.from(summary.textContent ?? "")).toHaveLength(
      COMPACT_HEADLINE_CHARS,
    );
    expect(summary.textContent).toContain("…");
    expect(container.querySelector("script, img[onerror]")).toBeNull();
    expect(
      screen.queryByRole("heading", { name: "End of full document" }),
    ).not.toBeInTheDocument();
    await user.click(toggle);
    expect(container.querySelectorAll("dt")).toHaveLength(3);
    expect(container.querySelector("dd")?.textContent).toContain(hostile);
    expect(
      screen
        .getByRole("heading", { name: "End of full document" })
        .closest("[inert]"),
    ).not.toBeNull();
    expect(
      screen.getByRole("link", { name: "Read reference" }).closest("[inert]"),
    ).not.toBeNull();
    await user.click(screen.getByRole("button", { name: "More details" }));

    expect(
      screen
        .getByRole("heading", { name: "End of full document" })
        .closest("[inert]"),
    ).toBeNull();
    expect(screen.getByText(longName)).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: "Read reference" }),
    ).toHaveAttribute("href", "https://example.com/reference");
    expect(
      screen.getByRole("link", { name: "Read reference" }),
    ).not.toHaveAttribute("tabindex", "-1");
    expect(screen.getByRole("link", { name: "Open task" })).toHaveAttribute(
      "href",
      "refact://task/task-123",
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
      const preferences = new Map([[`tc:${getCall.id ?? ""}`, storedOpen]]);
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
      const read = screen.getByRole("button", {
        name: "Read Main Plan Response received",
      });
      const list = screen.getByRole("button", {
        name: "3 documents shown Response received",
      });
      expect(read).toHaveAttribute("aria-expanded", String(storedOpen));
      expect(list).toHaveAttribute("aria-expanded", "false");
      await user.click(list);
      expect(read).toHaveAttribute("aria-expanded", String(storedOpen));
      await user.click(read);
      expect(list).toHaveAttribute("aria-expanded", "true");
      unmount();
      render(rows, { preloadedState: { chat } });
      expect(
        screen.getByRole("button", {
          name: "Read Main Plan Response received",
        }),
      ).toHaveAttribute("aria-expanded", String(!storedOpen));
      expect(
        screen.getByRole("button", {
          name: "3 documents shown Response received",
        }),
      ).toHaveAttribute("aria-expanded", "true");
    },
  );

  it("does not force expansion on result/status updates or reset a chosen secondary disclosure", async () => {
    const { container, store, threadId, toolCall, user } = renderDocumentTool(
      "doc_get",
      undefined,
      { streaming: true },
    );
    const toggle = screen.getByRole("button", {
      name: "Read requested-plan Running",
    });
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
    expect(toggle).toHaveAccessibleName("Read Main Plan Response received");
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    await user.click(toggle);
    const details = screen.getByRole("button", { name: "More details" });
    expect(details).toHaveAttribute("aria-expanded", "false");
    await user.click(details);
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
    expect(details).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("v4")).toBeInTheDocument();
    await user.click(details);
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
    expect(toggle).toHaveAccessibleName("Read requested-plan Request failed");
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(details).toHaveAttribute("aria-expanded", "false");
    expect(container.querySelector("dl")?.textContent).not.toContain(
      "Main Plan",
    );
  });
});
