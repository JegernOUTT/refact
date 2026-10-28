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

function renderTaskDone(
  content: string | null,
  {
    failed = false,
    streaming = false,
    latest,
  }: { failed?: boolean; streaming?: boolean; latest?: boolean } = {},
) {
  const chat = createDefaultChatState();
  const runtime = chat.threads[chat.current_thread_id];
  runtime.thread.messages =
    content === null
      ? [{ role: "assistant", content: "", tool_calls: [toolCall()] }]
      : [toolMessage(undefined, content, failed)];
  runtime.streaming = streaming;
  return render(<TaskDoneTool toolCall={toolCall()} latest={latest} />, {
    preloadedState: { chat },
  });
}

describe("ReportToolCard", () => {
  beforeEach(() => vi.clearAllMocks());

  test("renders a single collapsed transcript item with the report as markdown", async () => {
    const { container, user } = renderReportTool(toolMessage());
    expect(
      container.querySelectorAll('[data-transcript-item="true"]'),
    ).toHaveLength(1);
    expect(screen.getByTestId("transcript-verb")).toHaveTextContent("Report");
    expect(screen.getByTestId("transcript-target")).toHaveTextContent(
      "Run subagent",
    );
    const toggle = screen.getByTestId("transcript-toggle");
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    await user.click(toggle);
    expect(
      screen.getByRole("heading", { name: "Report", level: 1 }),
    ).toBeInTheDocument();
    expect(screen.getByText("Animation fixed.")).toBeInTheDocument();
    expect(container.querySelector("details")).toBeNull();
    expect(container.textContent).not.toContain("Technical details");
    expect(container.textContent).not.toContain("More details");
  });

  test("shows streaming progress as output while the report is pending", () => {
    const chat = createDefaultChatState();
    const runtime = chat.threads[chat.current_thread_id];
    runtime.streaming = true;
    runtime.thread.messages = [
      { role: "assistant", content: "", tool_calls: [toolCall()] },
    ];
    render(
      <ReportToolCard
        defaultOpen
        defaultSummary="Run subagent"
        icon={<FileText />}
        toolCall={toolCall()}
      />,
      { preloadedState: { chat } },
    );
    expect(screen.getByTestId("transcript-icon")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("transcript-toggle"));
    expect(screen.getByRole("group", { name: "Progress" })).toHaveTextContent(
      "collecting context",
    );
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "Running",
    );
  });

  test.each([
    [false, false, "Failure (self-reported)", "Reported failure"],
    [true, false, "Success (self-reported)", null],
    [true, true, "Success (self-reported)", "Request failed"],
  ] as const)(
    "keeps reported success=%s separate from transport failed=%s",
    async (success, failed, outcome, pill) => {
      const { user } = renderReportTool(
        toolMessage(
          undefined,
          JSON.stringify({ summary: "Build", success }),
          failed,
        ),
      );
      if (pill) {
        expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
          pill,
        );
      } else {
        expect(screen.queryByTestId("transcript-status-pill")).toBeNull();
      }
      await user.click(screen.getByTestId("transcript-toggle"));
      expect(screen.getByText(outcome)).toBeInTheDocument();
    },
  );

  test("exposes copy/save actions, file chips and knowledge after opening", async () => {
    const report = `# Report\n\n${"Line. ".repeat(200)}\n\nEND OF REPORT`;
    const { user } = renderReportTool(
      toolMessage(
        undefined,
        JSON.stringify({
          type: "task_done",
          summary: "Cache fix",
          report,
          files_changed: ["src/cache.ts"],
          knowledge_path: "knowledge/cache.md",
        }),
      ),
      {
        extractReport: (content) => {
          const data = JSON.parse(content) as {
            summary: string;
            report: string;
            files_changed: string[];
            knowledge_path: string;
          };
          return {
            summary: data.summary,
            markdown: data.report,
            filesChanged: data.files_changed,
            knowledgePath: data.knowledge_path,
          };
        },
      },
    );
    expect(screen.queryByRole("button", { name: "Copy report" })).toBeNull();
    await user.click(screen.getByTestId("transcript-toggle"));
    await user.click(screen.getByRole("button", { name: "Copy report" }));
    expect(copy).toHaveBeenCalledWith(report);
    await user.click(screen.getByRole("button", { name: "Save as file" }));
    expect(save).toHaveBeenCalledWith(report);
    await user.click(screen.getByRole("button", { name: "cache.ts" }));
    expect(openFile).toHaveBeenCalledWith({ path: "src/cache.ts" });
    expect(screen.getByText("Saved to knowledge")).toBeInTheDocument();
    expect(screen.getByTestId("transcript-clamp")).toBeInTheDocument();
    expect(screen.getByText(/END OF REPORT/)).toBeInTheDocument();
  });

  test("failed transport still preserves copyable evidence", async () => {
    const { user } = renderReportTool(
      toolMessage(undefined, "Exact failure payload", true),
    );
    await user.click(screen.getByTestId("transcript-toggle"));
    await user.click(screen.getByRole("button", { name: "Copy report" }));
    expect(copy).toHaveBeenCalledWith("Exact failure payload");
  });

  test("stored open choices override defaults and never reopen on progress", () => {
    const values = new Map<string, boolean>([["tc:report-card-1", false]]);
    const store = {
      get: (key: string) => values.get(key),
      set: (key: string, value: boolean) => values.set(key, value),
    };
    const chat = createDefaultChatState();
    render(
      <CollapsibleStoreProvider value={store}>
        <ReportToolCard
          defaultOpen
          defaultSummary="Collect build evidence"
          icon={<FileText />}
          toolCall={toolCall()}
        />
      </CollapsibleStoreProvider>,
      { preloadedState: { chat } },
    );
    expect(screen.getByTestId("transcript-toggle")).toHaveAttribute(
      "aria-expanded",
      "false",
    );
  });

  test("raw block keeps the request arguments and response", async () => {
    const content = "# Report\n\n**Original** markdown";
    const { user } = renderReportTool(toolMessage(undefined, content));
    await user.click(screen.getByTestId("transcript-toggle"));
    await user.click(screen.getByRole("button", { name: /^Raw/ }));
    const raw = screen.getByTestId("transcript-raw");
    expect(raw).toHaveTextContent("Investigate animation");
    expect(raw).toHaveTextContent("**Original** markdown");
  });
});

describe("TaskDoneTool", () => {
  beforeEach(() => vi.clearAllMocks());

  test("keeps the task-done test id on the transcript root", () => {
    renderTaskDone(null, { streaming: true });
    const root = screen.getByTestId("task-done-tool");
    expect(root).toHaveAttribute("data-transcript-item", "true");
    expect(screen.getByTestId("transcript-target")).toHaveTextContent(
      "Task report",
    );
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "Running",
    );
    expect(root).not.toHaveTextContent(/Task completed/);
  });

  test.each([
    {
      state: "transport failed despite reported success",
      content: JSON.stringify({ type: "task_done", success: true }),
      failed: true,
      status: "failed",
      pill: "Request failed",
    },
    {
      state: "reported false",
      content: JSON.stringify({ type: "task_done", success: false }),
      failed: false,
      status: "failed",
      pill: "Reported failure",
    },
    {
      state: "reported true",
      content: JSON.stringify({ type: "task_done", success: true }),
      failed: false,
      status: "done",
      pill: null,
    },
    {
      state: "missing outcome",
      content: '{"type":"task_done"}',
      failed: false,
      status: "done",
      pill: null,
    },
    {
      state: "legacy success prose",
      content: "# Success!\n\nAll checks passed!",
      failed: false,
      status: "done",
      pill: null,
    },
  ])("maps $state truthfully", ({ content, failed, status, pill }) => {
    renderTaskDone(content, { failed });
    expect(screen.getByTestId("task-done-tool")).toHaveAttribute(
      "data-status",
      status,
    );
    if (pill) {
      expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
        pill,
      );
    } else {
      expect(screen.queryByTestId("transcript-status-pill")).toBeNull();
    }
    expect(screen.getByTestId("transcript-meta")).toHaveTextContent(
      "No verification evidence",
    );
  });

  test("absent result without activity is unknown, never success", () => {
    renderTaskDone(null);
    expect(screen.getByTestId("task-done-tool")).toHaveAttribute(
      "data-status",
      "unknown",
    );
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "No result",
    );
  });

  test("the latest report starts open with the full markdown, earlier ones collapsed", async () => {
    const payload = {
      type: "task_done",
      summary: "Build still needs fixes",
      success: false,
      report: "# Build report\n\n- Full failure evidence\n\nEND OF REPORT",
      files_changed: ["src/cache.ts"],
    };
    const view = renderTaskDone(JSON.stringify(payload), { latest: true });
    expect(screen.getByTestId("transcript-toggle")).toHaveAttribute(
      "aria-expanded",
      "true",
    );
    expect(
      screen.getByRole("heading", { name: "Build report" }),
    ).toBeInTheDocument();
    expect(await screen.findByText(/END OF REPORT/)).toBeInTheDocument();
    expect(screen.getByText("cache.ts")).toBeInTheDocument();
    await view.user.click(screen.getByRole("button", { name: "Copy report" }));
    expect(copy).toHaveBeenCalledWith(payload.report);
    view.unmount();
    renderTaskDone(JSON.stringify(payload), { latest: false });
    expect(screen.getByTestId("transcript-toggle")).toHaveAttribute(
      "aria-expanded",
      "false",
    );
  });

  test.each([
    null,
    [],
    { type: "task_done", summary: 42, report: false, files_changed: [1] },
  ])("malformed legacy fields are safe: %j", (payload) => {
    renderTaskDone(JSON.stringify(payload));
    expect(screen.getByTestId("transcript-toggle")).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    fireEvent.click(screen.getByTestId("transcript-toggle"));
    expect(screen.getByTitle("Copy report")).toBeInTheDocument();
  });
});

describe("report without a recorded result", () => {
  function renderUnrecorded(options: { busy?: boolean; paused?: boolean }) {
    const chat = createDefaultChatState();
    const runtime = chat.threads[chat.current_thread_id];
    const call = toolCall("report-unrecorded");
    runtime.streaming = options.busy === true;
    runtime.thread.messages = [
      { role: "assistant", content: "", tool_calls: [call] },
    ];
    if (options.paused) {
      runtime.confirmation.pause = true;
      runtime.confirmation.pause_reasons = [
        {
          type: "confirmation",
          tool_name: "subagent",
          command: "subagent",
          rule: "ask",
          tool_call_id: "report-unrecorded",
          integr_config_path: null,
        },
      ];
    }
    return render(
      <ReportToolCard
        defaultSummary="Report"
        icon={<span />}
        toolCall={call}
      />,
      { preloadedState: { chat } },
    );
  }

  test.each([
    [{}, "unknown", "No result"],
    [{ busy: true }, "running", "Running"],
    [{ busy: true, paused: true }, "needs_you", "Needs approval"],
  ] as const)("%o shows %s", (options, status, pill) => {
    const { container } = renderUnrecorded(options);
    expect(container.querySelector("section")).toHaveAttribute(
      "data-status",
      status,
    );
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      pill,
    );
  });
});
