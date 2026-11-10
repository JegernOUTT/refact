import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "../../../utils/test-utils";
import { setUpStore } from "../../../app/store";
import { restoreChat } from "../../../features/Chat/Thread";
import type {
  ChatMessages,
  ToolCall,
  EventContent,
} from "../../../services/refact/types";
import {
  buildDisplayItems,
  tryIncrementalDisplayItemsUpdate,
} from "../ChatContentDisplayItems";
import { ToolContent } from "../ToolsContent";

vi.mock("../../../services/refact/toolJobs", async (original) => ({
  ...(await original<typeof import("../../../services/refact/toolJobs")>()),
  useGetToolJobQuery: () => ({
    currentData: undefined,
    isError: false,
    refetch: vi.fn(),
  }),
  useCancelToolJobMutation: () => [vi.fn(), { isLoading: false }],
}));
afterEach(cleanup);

const call: ToolCall = {
  id: "call",
  index: 0,
  type: "function",
  function: { name: "glob", arguments: '{"pattern":"src/*.ts"}' },
};
const receipt = {
  job_id: "job",
  chat_id: "chat",
  tool_call_id: "call",
  tool_name: "glob",
  model: "model",
  turn_id: "turn",
  status: "accepted",
  result_url: "/result",
  cancel_url: "/cancel",
};
const initial: ChatMessages = [
  { role: "assistant", content: "", tool_calls: [call] },
  {
    role: "tool",
    tool_call_id: "call",
    content: "Accepted background tool",
    extra: { tool_job: receipt },
  },
  { role: "assistant", content: "Meanwhile" },
  { role: "user", content: "Continue" },
];
function completion(
  job = "job",
  content: EventContent = "## Native output",
  part = 0,
): ChatMessages[number] {
  return {
    role: "event",
    subkind: "system_notice",
    source: "tool_job.completed",
    message_id: `tool_job:${job}:2:${part}`,
    content,
    extra: {
      event: {
        source: "tool_job.completed",
        subkind: "system_notice",
        payload: { actor: "tool_completion", job_id: job },
      },
      tool_job_result: {
        actor: "tool_completion",
        job_id: job,
        revision: 2,
        tool_call_id: "call",
        tool_name: "glob",
        original_role: "tool",
      },
      delivery: { source: "tool_job.completed", id: `tool_job:${job}:2` },
    },
  } as ChatMessages[number];
}
function view(messages: ChatMessages, store = setUpStore()) {
  store.dispatch(
    restoreChat({
      id: "chat",
      model: "model",
      title: "Background test",
      createdAt: "2026-11-10T00:00:00Z",
      updatedAt: "2026-11-10T00:00:00Z",
      messages,
    }),
  );
  const item = buildDisplayItems(messages, false)[0];
  if (item.type !== "assistant") throw new Error("Missing initial tool");
  return {
    store,
    element: (
      <ToolContent
        threadId="chat"
        toolCalls={[call]}
        jobResultsByToolId={item.jobResultsByToolId}
      />
    ),
  };
}

describe("background results attached to original calls", () => {
  it("attaches late multipart output once and keeps incremental builds equivalent", () => {
    const previous = buildDisplayItems(initial, false);
    const messages = [
      ...initial,
      completion(),
      completion(
        "job",
        [
          { m_type: "text", m_content: "Image description" },
          { m_type: "image/png", m_content: "aGVsbG8=" },
        ],
        1,
      ),
    ];
    const full = buildDisplayItems(messages, false);
    expect(full.map((item) => item.type)).toEqual([
      "assistant",
      "assistant",
      "user",
    ]);
    expect(full[0]).toMatchObject({
      jobResultsByToolId: {
        call: [{ toolName: "glob" }, { toolName: "glob" }],
      },
    });
    expect(
      tryIncrementalDisplayItemsUpdate(initial, messages, previous, false),
    ).toEqual(full);
    const updated = [...messages];
    updated[4] = completion("job", "## Changed output");
    const incremental = tryIncrementalDisplayItemsUpdate(
      messages,
      updated,
      full,
      false,
    );
    expect(incremental ?? buildDisplayItems(updated, false)).toEqual(
      buildDisplayItems(updated, false),
    );
  });

  it("keeps missing or mismatched jobs as standalone output", () => {
    expect(buildDisplayItems([completion()], false)[0].type).toBe(
      "tool_job_result",
    );
    expect(
      buildDisplayItems([...initial, completion("other")], false).at(-1)?.type,
    ).toBe("tool_job_result");
  });

  it("keeps interleaved jobs and prototype-shaped call ids isolated", () => {
    const otherCall = { ...call, id: "__proto__" };
    const otherEvent = completion("other");
    otherEvent.extra = {
      ...otherEvent.extra,
      tool_job_result: {
        ...(otherEvent.extra?.tool_job_result as Record<string, unknown>),
        tool_call_id: "__proto__",
      },
    };
    const messages: ChatMessages = [
      ...initial,
      { role: "assistant", content: "", tool_calls: [otherCall] },
      {
        role: "tool",
        tool_call_id: "__proto__",
        content: "Accepted",
        extra: {
          tool_job: { ...receipt, tool_call_id: "__proto__", job_id: "other" },
        },
      },
      otherEvent,
      completion(),
    ];
    const items = buildDisplayItems(messages, false);
    expect(
      items.filter((item) => item.type === "tool_job_result"),
    ).toHaveLength(0);
    expect(items[0]).toMatchObject({
      jobResultsByToolId: { call: [{ jobId: "job" }] },
    });
    const other = items[3];
    if (other.type !== "assistant") throw new Error("Missing second call");
    expect(Object.keys(other.jobResultsByToolId)).toEqual(["__proto__"]);
    expect(other.jobResultsByToolId.__proto__?.[0].jobId).toBe("other");
    expect(
      tryIncrementalDisplayItemsUpdate(
        initial,
        messages,
        buildDisplayItems(initial, false),
        false,
      ),
    ).toEqual(items);
  });

  it("embeds sanitized App output without exposing original arguments or another tool row", () => {
    const appCall = {
      ...call,
      function: {
        name: "app",
        arguments: '{"action":"read_file","host_view_token":"ARG_SECRET"}',
      },
    };
    const event = completion(
      "job",
      '{"revision":"app-revision","host_view_token":"RESPONSE_SECRET"}',
    );
    event.extra = {
      ...event.extra,
      tool_job_result: {
        ...(event.extra?.tool_job_result as Record<string, unknown>),
        tool_name: "app",
      },
    };
    const messages: ChatMessages = [
      { role: "assistant", content: "", tool_calls: [appCall] },
      {
        role: "tool",
        tool_call_id: "call",
        content: "Accepted",
        extra: { tool_job: { ...receipt, tool_name: "app" } },
      },
      event,
    ];
    const current = view(messages);
    const item = buildDisplayItems(messages, false)[0];
    if (item.type !== "assistant") throw new Error("Missing App call");
    const { container } = render(
      <ToolContent
        threadId="chat"
        toolCalls={[appCall]}
        jobResultsByToolId={item.jobResultsByToolId}
      />,
      { store: current.store },
    );
    fireEvent.click(screen.getByTestId("transcript-toggle"));
    expect(screen.getByText("app-revision")).toBeInTheDocument();
    expect(screen.getAllByTestId("transcript-row")).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: "Raw JSON" }));
    expect(container.textContent).not.toContain("ARG_SECRET");
    expect(container.textContent).not.toContain("RESPONSE_SECRET");
  });

  it("preserves an earlier delivery if a newer receipt reuses the call id", () => {
    const messages: ChatMessages = [
      ...initial,
      completion(),
      {
        role: "tool",
        tool_call_id: "call",
        content: "Another job accepted",
        extra: { tool_job: { ...receipt, job_id: "replacement" } },
      },
    ];
    const current = view(messages);
    render(current.element, { store: current.store });
    expect(screen.getAllByTestId("transcript-toggle")).toHaveLength(2);
    fireEvent.click(screen.getByRole("button", { name: /Tool result.*glob/ }));
    expect(
      screen.getByRole("heading", { name: "Native output" }),
    ).toBeInTheDocument();
  });

  it("attaches context-file parts beneath the initial call without a separate result", async () => {
    const event = completion();
    const metadata = event.extra?.tool_job_result as Record<string, unknown>;
    const context = {
      ...event,
      role: "context_file",
      tool_call_id: "call",
      content: [
        {
          file_name: "src/native.ts",
          file_content: "native preview",
          line1: 1,
          line2: 1,
        },
      ],
      extra: {
        ...event.extra,
        tool_job_result: { ...metadata, original_role: "context_file" },
      },
    } as ChatMessages[number];
    const current = view([...initial, context]);
    render(current.element, { store: current.store });
    expect(screen.queryByTestId("context-file")).toBeNull();
    fireEvent.click(screen.getByTestId("transcript-toggle"));
    expect(screen.getByTitle("src/native.ts")).toBeInTheDocument();
    expect(await screen.findByText(/native preview/)).toBeInTheDocument();
    expect(screen.queryByText("Tool result")).toBeNull();
  });

  it("shows the Background badge and keeps output closed when delivery arrives", () => {
    const before = view(initial);
    const { rerender } = render(before.element, { store: before.store });
    expect(screen.getByText("Background")).toBeInTheDocument();
    const toggle = screen.getByTestId("transcript-toggle");
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    const after = view([...initial, completion()], before.store);
    rerender(after.element);
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("Native output")).toBeNull();
    expect(screen.getByText("Completed")).toBeInTheDocument();
    fireEvent.click(toggle);
    expect(
      screen.getByRole("heading", { name: "Native output" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("Tool result")).toBeNull();
    expect(screen.getAllByTestId("transcript-row")).toHaveLength(1);
    expect(screen.queryByRole("button", { name: /cancel/i })).toBeNull();
  });

  it("retains manual expansion while results arrive and preserves native images", () => {
    const before = view(initial);
    const { rerender } = render(before.element, { store: before.store });
    const toggle = screen.getByTestId("transcript-toggle");
    fireEvent.click(toggle);
    const after = view(
      [
        ...initial,
        completion("job", [
          { m_type: "text", m_content: "## Image result" },
          { m_type: "image/png", m_content: "aGVsbG8=" },
        ]),
      ],
      before.store,
    );
    rerender(after.element);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(
      screen.getByRole("heading", { name: "Image result" }),
    ).toBeInTheDocument();
    expect(document.querySelector("img")).toHaveAttribute(
      "src",
      "data:image/png;base64,aGVsbG8=",
    );
  });

  it("preserves failure status and privacy reveal instead of raw withheld output", () => {
    const event = {
      ...completion(),
      tool_failed: true,
    } as ChatMessages[number];
    event.extra = {
      ...event.extra,
      privacy_shell: { withheld: true, local_only_output: "PRIVATE_LOCAL" },
    };
    const native = buildDisplayItems([...initial, event], false)[0];
    if (native.type !== "assistant") throw new Error("Missing initial tool");
    const { store } = view(initial);
    render(
      <ToolContent
        threadId="chat"
        toolCalls={[call]}
        jobResultsByToolId={native.jobResultsByToolId}
      />,
      { store },
    );
    expect(screen.getByText("Failed")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("transcript-toggle"));
    expect(
      screen.getByTestId("privacy-withheld-output-card"),
    ).toBeInTheDocument();
    expect(screen.queryByText("PRIVATE_LOCAL")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Show me" }));
    expect(screen.getByTestId("privacy-local-output")).toHaveTextContent(
      "PRIVATE_LOCAL",
    );
  });
});
