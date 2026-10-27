import { FileText } from "lucide-react";
import type { ComponentProps } from "react";
import { beforeEach, describe, expect, test, vi } from "vitest";

import {
  createDefaultChatState,
  fireEvent,
  render,
  screen,
} from "../../../utils/test-utils";
import type { ChatMessage, ToolCall } from "../../../services/refact/types";
import { ReportToolCard } from "./ReportToolCard";
import { TaskDoneTool } from "./TaskDoneTool";
import { CollapsibleStoreProvider } from "../useStoredOpen";

vi.hoisted(() => vi.resetModules());
const { copy, save, openFile } = vi.hoisted(() => ({
  copy: vi.fn(),
  save: vi.fn(),
  openFile: vi.fn(),
}));
vi.mock("../../../hooks/useCopyToClipboard", () => ({
  useCopyToClipboard: () => copy,
}));
vi.mock("../../../hooks/useEventBusForIDE", async (importOriginal) => ({
  ...(await importOriginal<
    typeof import("../../../hooks/useEventBusForIDE")
  >()),
  useEventsBusForIDE: () => ({ newFile: save }),
}));
vi.mock("../../../hooks/useOpenFileInApp", () => ({
  useOpenFileInApp: () => ({ canOpen: true, openFile }),
}));
vi.mock("../../../utils/isIdeHost", () => ({ isIdeHost: () => true }));

function toolCall(id = "report-card-1"): ToolCall {
  return {
    id,
    index: 0,
    function: {
      name: "subagent",
      arguments: JSON.stringify({ task: "Investigate animation" }),
    },
    subchat_log: ["1/2: collecting context"],
  };
}

function toolMessage(
  id = "report-card-1",
  content = "# Report\n\nAnimation fixed.",
  failed = false,
): ChatMessage {
  return {
    role: "tool",
    tool_call_id: id,
    content,
    tool_failed: failed,
  };
}

function renderReportTool(
  message?: ChatMessage,
  props: Partial<ComponentProps<typeof ReportToolCard>> = {},
) {
  const chat = createDefaultChatState();
  const runtime = chat.threads[chat.current_thread_id];
  runtime.thread.messages = message ? [message] : [];
  const id =
    message && "tool_call_id" in message
      ? message.tool_call_id
      : "report-card-1";

  return render(
    <ReportToolCard
      toolCall={toolCall(id)}
      icon={<FileText />}
      defaultSummary="Run subagent"
      {...props}
    />,
    { preloadedState: { chat } },
  );
}

describe("ReportToolCard", () => {
  beforeEach(() => vi.clearAllMocks());
  test("routes its body through the shared animated collapsible wrapper", async () => {
    const { container, user } = renderReportTool(toolMessage());
    const card = container.querySelector("section");
    const toggle = screen.getByRole("button", { name: /run subagent/i });

    expect(card).toHaveAttribute("data-open", "true");
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(container.querySelector(".rf-expand-grid")).toBeInTheDocument();
    expect(screen.getByText("Animation fixed.")).toBeInTheDocument();

    await user.click(toggle);

    expect(card).toHaveAttribute("data-open", "false");
    expect(toggle).toHaveAttribute("aria-expanded", "false");
  });

  test("applies the running text shimmer and spinner while report tools are pending", () => {
    const { container } = renderReportTool();
    const toggle = screen.getByRole("button", { name: /run subagent/i });

    expect(container.querySelector("section")).toHaveAttribute(
      "data-status",
      "running",
    );
    expect(toggle).not.toHaveClass("rf-active-pulse");
    expect(toggle.querySelector(".rf-text-shimmer")).toHaveTextContent(
      "Run subagent",
    );
    expect(toggle.querySelector(".rf-spin")).toBeInTheDocument();
    expect(screen.getByText(/collecting context/)).toBeInTheDocument();
  });

  test("compact opt-in starts closed and keeps progress behind both disclosures", async () => {
    const { container, user } = renderReportTool(undefined, {
      compact: true,
      meta: "model: private-model",
    });
    const toggle = screen.getByRole("button");
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(container.querySelector("section")).toHaveAttribute(
      "data-density",
      "compact",
    );
    expect(toggle).toHaveTextContent("Awaiting report");
    expect(toggle).not.toHaveTextContent(/completed|success/i);
    expect(screen.queryByText(/collecting context/)).not.toBeInTheDocument();
    expect(screen.queryByText(/private-model/)).not.toBeInTheDocument();
    toggle.focus();
    await user.keyboard("{Enter}");
    expect(container.querySelectorAll("dt")).toHaveLength(2);
    const progress = screen.getByText(/collecting context/);
    expect(progress.closest("[inert]")).not.toBeNull();
    const more = screen.getByRole("button", { name: "More details" });
    more.focus();
    await user.keyboard("{Enter}");
    expect(progress.closest("[inert]")).toBeNull();
    expect(screen.getByText(/private-model/)).toBeInTheDocument();
  });

  test.each([
    [false, false, "Reported failure"],
    [true, false, "Reported success"],
    [true, true, "Request failed"],
  ] as const)(
    "keeps reported success=%s separate from transport failed=%s",
    (success, failed, label) => {
      const content = JSON.stringify({ summary: "Attempted build", success });
      const { container } = renderReportTool(
        toolMessage(undefined, content, failed),
        { compact: true },
      );
      expect(screen.getByText(label)).toBeInTheDocument();
      expect(screen.getByRole("button")).not.toHaveTextContent(
        /completed|verified/i,
      );
      expect(container.querySelector("section")).toHaveAttribute(
        "data-status",
        failed || !success ? "error" : "idle",
      );
      fireEvent.click(screen.getByRole("button"));
      expect(screen.getByText("No verification evidence")).toBeInTheDocument();
    },
  );

  test("preserves full report copy/save, keyboard file links and bounded content after deeper open", async () => {
    const markdown = `# Full report\n\n${"Evidence preserved. ".repeat(
      80,
    )}\n\nEND OF REPORT`;
    const { container, user } = renderReportTool(
      toolMessage(undefined, "transport"),
      {
        compact: true,
        unboundedContent: true,
        extractReport: () => ({
          summary: "Review the evidence",
          markdown,
          filesChanged: ["src/nested/cache.ts"],
          knowledgePath: ".refact/knowledge/report.md",
        }),
      },
    );
    expect(screen.queryByTitle("Copy report")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button"));
    const copyButton = screen.getByTitle("Copy report");
    expect(copyButton.closest("[inert]")).not.toBeNull();
    const more = screen.getByRole("button", { name: "More details" });
    more.focus();
    await user.keyboard("{Enter}");
    expect(copyButton.closest("[inert]")).toBeNull();
    copyButton.focus();
    await user.keyboard("{Enter}");
    expect(copy).toHaveBeenCalledWith(markdown);
    await user.click(screen.getByTitle("Save as file"));
    expect(save).toHaveBeenCalledWith(markdown);
    const file = screen.getByRole("button", { name: "cache.ts" });
    file.focus();
    await user.keyboard("{Enter}");
    expect(openFile).toHaveBeenCalledWith({ path: "src/nested/cache.ts" });
    expect(screen.getByText("Saved to knowledge")).toBeInTheDocument();
    expect(screen.getByText(/END OF REPORT/)).toBeInTheDocument();
    expect(container.querySelector("[class*='contentUnbounded']")).toBeNull();
    expect(container.querySelector("button button")).toBeNull();
  });

  test("failed transport still preserves copyable evidence", async () => {
    const { user } = renderReportTool(
      toolMessage(undefined, "Failure evidence", true),
      { compact: true },
    );
    await user.click(screen.getByRole("button"));
    await user.click(screen.getByRole("button", { name: "More details" }));
    await user.click(screen.getByTitle("Copy report"));
    expect(copy).toHaveBeenCalledWith("Failure evidence");
  });

  test("stored open choices override defaults and progress updates never reopen", () => {
    const values = new Map([["tc:report-card-1", true]]);
    const store = {
      get: (key: string) => values.get(key),
      set: (key: string, open: boolean) => values.set(key, open),
    };
    const view = (log: string[]) => (
      <CollapsibleStoreProvider value={store}>
        <ReportToolCard
          compact
          toolCall={{ ...toolCall(), subchat_log: log }}
          icon={<FileText />}
          defaultSummary="Collect build evidence"
        />
      </CollapsibleStoreProvider>
    );
    const { rerender, unmount } = render(view(["first progress"]));
    const toggle = screen.getByRole("button", {
      name: /Collect build evidence/,
    });
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    fireEvent.click(toggle);
    rerender(view(["later progress"]));
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    unmount();
    render(view(["remounted progress"]));
    expect(
      screen.getByRole("button", { name: /Collect build evidence/ }),
    ).toHaveAttribute("aria-expanded", "false");
  });

  test("explicit defaultOpen remains supported in compact mode", () => {
    renderReportTool(toolMessage(), { compact: true, defaultOpen: true });
    expect(
      screen.getByRole("button", { name: /Run subagent/ }),
    ).toHaveAttribute("aria-expanded", "true");
  });

  test("legacy report prose cannot mint success or verification", () => {
    renderReportTool(toolMessage(undefined, "PASS! All tests verified!"), {
      compact: true,
    });
    expect(screen.getByText("Report unverified")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button"));
    expect(screen.getByText("No verification evidence")).toBeInTheDocument();
  });

  test("TaskDoneTool pending uses compact report language, never completed", () => {
    const chat = createDefaultChatState();
    chat.threads[chat.current_thread_id].streaming = true;
    render(<TaskDoneTool toolCall={toolCall()} compact />, {
      preloadedState: { chat },
    });
    const row = screen.getByRole("button", { expanded: false });
    expect(row).toHaveTextContent("Task report");
    expect(row).toHaveTextContent("Awaiting report");
    expect(screen.queryByText(/Task completed/)).not.toBeInTheDocument();
  });

  test.each([
    { state: "pending", content: null, failed: false, icon: "loader-circle" },
    {
      state: "transport failed despite reported success",
      content: JSON.stringify({ type: "task_done", success: true }),
      failed: true,
      icon: "circle-x",
    },
    {
      state: "reported false",
      content: JSON.stringify({ type: "task_done", success: false }),
      failed: false,
      icon: "circle-x",
    },
    {
      state: "reported true",
      content: JSON.stringify({ type: "task_done", success: true }),
      failed: false,
      icon: "circle-check",
    },
    {
      state: "missing outcome",
      content: '{"type":"task_done"}',
      failed: false,
      icon: "file-text",
    },
    {
      state: "mistyped outcome",
      content: '{"type":"task_done","success":"true"}',
      failed: false,
      icon: "file-text",
    },
    {
      state: "malformed",
      content: "{broken",
      failed: false,
      icon: "file-text",
    },
    {
      state: "legacy success prose",
      content: "# Success!\n\nAll checks passed!",
      failed: false,
      icon: "file-text",
    },
  ])("TaskDoneTool uses truthful $state icon", ({ content, failed, icon }) => {
    const chat = createDefaultChatState();
    chat.threads[chat.current_thread_id].thread.messages =
      content === null ? [] : [toolMessage(undefined, content, failed)];
    chat.threads[chat.current_thread_id].streaming = content === null;
    const { container } = render(
      <TaskDoneTool toolCall={toolCall()} compact />,
      {
        preloadedState: { chat },
      },
    );
    const toggle = screen.getByRole("button", { expanded: false });
    expect(toggle.querySelector(`.lucide-${icon}`)).toBeInTheDocument();
    expect(container.querySelector("section")).toHaveAttribute(
      "data-status",
      icon === "circle-x" ? "error" : content === null ? "running" : "idle",
    );
    expect(toggle).toHaveTextContent(
      failed
        ? "Request failed"
        : content === null
          ? "Awaiting report"
          : icon === "circle-check"
            ? "Reported success"
            : icon === "circle-x"
              ? "Reported failure"
              : "Report unverified",
    );
    if (icon !== "circle-check") {
      expect(toggle.querySelector(".lucide-circle-check")).toBeNull();
    }
  });

  test.each([
    { state: "pending", content: null, failed: false },
    {
      state: "failed",
      content: "# Failure\n\nExact failure payload",
      failed: true,
    },
    {
      state: "markdown",
      content: "# Report\n\n**Original** markdown",
      failed: false,
    },
    { state: "malformed", content: "```json\n{broken}\n```", failed: false },
  ])(
    "compact report preserves raw request and result in $state state",
    ({ content, failed }) => {
      const { container } = renderReportTool(
        content === null ? undefined : toolMessage(undefined, content, failed),
        { compact: true },
      );
      fireEvent.click(screen.getByRole("button"));
      fireEvent.click(screen.getByRole("button", { name: "More details" }));
      const technical = screen.getByRole("button", {
        name: "Technical details",
      });
      expect(technical).toHaveAttribute("aria-expanded", "false");
      fireEvent.click(technical);
      const codes = Array.from(container.querySelectorAll("code"));
      expect(
        codes.some(
          (node) => node.textContent === toolCall().function.arguments,
        ),
      ).toBe(true);
      if (content !== null) {
        expect(codes.some((node) => node.textContent === content)).toBe(true);
      }
    },
  );

  test("standalone TaskDoneTool stays expanded and unbounded with full content, files and reported failure", async () => {
    const payload = {
      type: "task_done",
      summary: "Build still needs fixes",
      success: false,
      report: `# Build report\n\nFull failure evidence\n\n${"Evidence preserved. ".repeat(
        80,
      )}\n\nEND OF REPORT`,
      files_changed: ["src/cache.ts"],
    };
    const chat = createDefaultChatState();
    chat.threads[chat.current_thread_id].thread.messages = [
      toolMessage(undefined, JSON.stringify(payload)),
    ];
    const { user, container } = render(<TaskDoneTool toolCall={toolCall()} />, {
      preloadedState: { chat },
    });
    const row = screen.getByRole("button", { expanded: true });
    expect(row).toHaveTextContent("Build still needs fixes");
    expect(screen.getByText("Reported failure")).toBeInTheDocument();
    expect(row).not.toHaveTextContent(/completed|verified/i);
    expect(row.querySelector(".lucide-circle-x")).toBeInTheDocument();
    expect(row.querySelector(".lucide-circle-check")).toBeNull();
    expect(container.querySelector("section")).toHaveAttribute(
      "data-status",
      "error",
    );
    expect(container.querySelector('[data-density="compact"]')).toBeNull();
    expect(screen.queryByRole("button", { name: "More details" })).toBeNull();
    expect(await screen.findByText(/END OF REPORT/)).toBeInTheDocument();
    expect(
      container.querySelector("[class*='contentUnbounded']"),
    ).not.toBeNull();
    await user.click(screen.getByRole("button", { name: "Copy report" }));
    expect(copy).toHaveBeenCalledWith(payload.report);
    await user.click(screen.getByRole("button", { name: "Save as file" }));
    expect(save).toHaveBeenCalledWith(payload.report);
    await user.click(screen.getByText("cache.ts"));
    expect(openFile).toHaveBeenCalledWith({ path: "src/cache.ts" });
    expect(container.querySelector("button button")).toBeNull();
  });

  test.each([
    null,
    [],
    { type: "task_done", summary: 42, report: false, files_changed: [1] },
  ])("TaskDoneTool malformed legacy fields are safe: %j", (payload) => {
    const chat = createDefaultChatState();
    chat.threads[chat.current_thread_id].thread.messages = [
      toolMessage(undefined, JSON.stringify(payload)),
    ];
    render(<TaskDoneTool toolCall={toolCall()} compact />, {
      preloadedState: { chat },
    });
    expect(screen.getByRole("button")).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(screen.getByText("Report unverified")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button"));
    fireEvent.click(screen.getByRole("button", { name: "More details" }));
    expect(screen.getByTitle("Copy report")).toBeInTheDocument();
  });
});

describe("explicit compact report transport", () => {
  test.each(["pending", "unknown", "running"] as const)(
    "uses %s transport without inventing a result",
    (compactTransportState) => {
      const { container } = renderReportTool(undefined, {
        compact: true,
        compactTransportState,
      });
      expect(container.querySelector("section")).toHaveAttribute(
        "data-status",
        compactTransportState === "running" ? "running" : "idle",
      );
      expect(screen.getByRole("button")).toHaveAttribute(
        "aria-expanded",
        "false",
      );
      expect(screen.getByRole("button")).toHaveTextContent("Awaiting report");
    },
  );

  test("explicit failed transport dominates typed reported success only in compact mode", () => {
    const message = toolMessage(
      undefined,
      JSON.stringify({ summary: "Attempted build", success: true }),
    );
    const first = renderReportTool(message, {
      compact: true,
      compactTransportState: "failed",
    });
    expect(screen.getByText("Request failed")).toBeInTheDocument();
    expect(first.container.querySelector("section")).toHaveAttribute(
      "data-status",
      "error",
    );
    first.unmount();
    const panel = renderReportTool(message, {
      compactTransportState: "failed",
    });
    expect(panel.container.querySelector("section")).toHaveAttribute(
      "data-status",
      "success",
    );
  });
});
