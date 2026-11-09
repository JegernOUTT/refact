import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import {
  isTerminalToolJob,
  parseToolJobReceipt,
  parseToolJobSnapshot,
} from "../../../services/refact/toolJobs";

const mocks = vi.hoisted(() => ({
  status: "running",
  endpoint: "one",
  cancel: vi.fn(),
  refetch: vi.fn(),
  failed: false,
}));
vi.mock("../../../hooks/useAppSelector", () => ({
  useAppSelector: (selector: (state: unknown) => unknown) =>
    selector({ config: { lspUrl: `http://${mocks.endpoint}`, apiKey: "key" } }),
}));
vi.mock("../../../services/refact/toolJobs", async (importOriginal) => ({
  ...(await importOriginal<
    typeof import("../../../services/refact/toolJobs")
  >()),
  useGetToolJobQuery: () => ({
    currentData: { receipt, status: mocks.status, revision: 2 },
    isError: mocks.failed,
    refetch: mocks.refetch,
  }),
  useCancelToolJobMutation: () => [mocks.cancel, { isLoading: false }],
}));
vi.mock("./ToolCard", () => ({
  ToolCard: ({
    statusLabel,
    actions,
    children,
  }: {
    statusLabel: string;
    actions: React.ReactNode;
    children: React.ReactNode;
  }) => (
    <div>
      <span>{statusLabel}</span>
      {actions}
      {children}
    </div>
  ),
}));
const receipt = {
  job_id: "job",
  chat_id: "chat",
  tool_call_id: "call",
  tool_name: "code_health",
  model: "model",
  turn_id: "turn",
  status: "accepted" as const,
  result_url: "/result",
  cancel_url: "/cancel",
};
const toolCall = {
  index: 0,
  id: "call",
  type: "function" as const,
  function: { name: "code_health", arguments: "{}" },
};
const args = { chatId: "chat", jobId: "job", endpoint: "same-origin" };

describe("job transport", () => {
  it("withholds in-flight responses after the endpoint changes", async () => {
    vi.resetModules();
    vi.doUnmock("../../../services/refact/toolJobs");
    const { configureStore, createSlice } = await import("@reduxjs/toolkit");
    const { toolJobsApi } = await import("../../../services/refact/toolJobs");
    const config = createSlice({
      name: "config",
      initialState: { host: "ide" as const, lspPort: 8123, apiKey: "key" },
      reducers: {
        change: (state) => {
          state.lspPort = 8124;
        },
      },
    });
    const store = configureStore({
      reducer: {
        config: config.reducer,
        [toolJobsApi.reducerPath]: toolJobsApi.reducer,
      },
      middleware: (getDefault) => getDefault().concat(toolJobsApi.middleware),
    });
    let resolve!: (response: Response) => void;
    const fetchMock = vi.fn(
      () =>
        new Promise<Response>((done) => {
          resolve = done;
        }),
    );
    vi.stubGlobal("fetch", fetchMock);
    const query = {
      chatId: "chat",
      jobId: "job",
      endpoint: "http://127.0.0.1:8123",
    };
    const pending = store.dispatch(
      toolJobsApi.endpoints.getToolJob.initiate(query),
    );
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalled());
    store.dispatch(config.actions.change());
    resolve(
      new Response(
        JSON.stringify({ receipt, status: "succeeded", revision: 4 }),
        { headers: { "content-type": "application/json" } },
      ),
    );
    expect((await pending).data).toBeUndefined();
    pending.unsubscribe();
    vi.unstubAllGlobals();
    store.dispatch(toolJobsApi.util.resetApiState());
    vi.doMock("../../../services/refact/toolJobs", async (importOriginal) => ({
      ...(await importOriginal<
        typeof import("../../../services/refact/toolJobs")
      >()),
      useGetToolJobQuery: () => ({
        currentData: { receipt, status: mocks.status, revision: 2 },
        isError: mocks.failed,
        refetch: mocks.refetch,
      }),
      useCancelToolJobMutation: () => [mocks.cancel, { isLoading: false }],
    }));
  });
});

describe("background tool job wire validation", () => {
  it("accepts flattened receipts and validates status snapshots without exposing result", () => {
    expect(parseToolJobReceipt(receipt)).toEqual(receipt);
    expect(
      parseToolJobSnapshot(
        {
          receipt,
          status: "running",
          revision: 2,
          result: { secret: "not retained" },
        },
        args,
      ),
    ).toEqual({ receipt, status: "running", revision: 2 });
    expect(
      parseToolJobSnapshot({ receipt, status: "succeeded", revision: 3 }, args)
        ?.status,
    ).toBe("succeeded");
  });
  it("rejects malformed and mismatched identities or revisions", () => {
    expect(parseToolJobReceipt({ ...receipt, job_id: "" })).toBeNull();
    expect(
      parseToolJobSnapshot({ receipt, status: "running", revision: -1 }, args),
    ).toBeNull();
    expect(
      parseToolJobSnapshot({ receipt, status: "fake", revision: 1 }, args),
    ).toBeNull();
    expect(
      parseToolJobSnapshot(
        { receipt, status: "running", revision: 1 },
        { ...args, chatId: "other" },
      ),
    ).toBeNull();
  });
  it("treats failures, cancelled, and unknown as terminal without guessing success", () => {
    for (const state of [
      "failed",
      "cancelled",
      "unknown",
      "succeeded",
    ] as const)
      expect(isTerminalToolJob(state)).toBe(true);
    for (const state of ["accepted", "running", "cancel_requested"] as const)
      expect(isTerminalToolJob(state)).toBe(false);
  });
  it("updates status without stealing focus or losing a draft", async () => {
    vi.resetModules();
    const { ToolJobCard } = await import("./ToolJobCard");
    mocks.status = "running";
    mocks.endpoint = "one";
    mocks.failed = false;
    const { rerender } = render(
      <>
        <input aria-label="draft" defaultValue="my draft" />
        <ToolJobCard receipt={receipt} toolCall={toolCall} />
      </>,
    );
    const draft = screen.getByLabelText("draft");
    draft.focus();
    expect(screen.getByText("Running in background")).toBeInTheDocument();
    mocks.status = "succeeded";
    rerender(
      <>
        <input aria-label="draft" defaultValue="my draft" />
        <ToolJobCard receipt={receipt} toolCall={toolCall} />
      </>,
    );
    expect(draft).toHaveFocus();
    expect(draft).toHaveValue("my draft");
    expect(screen.getByText("Completed")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /cancel/i }),
    ).not.toBeInTheDocument();
  });
  it("requests cancellation once and displays the confirmed request", async () => {
    vi.resetModules();
    const { ToolJobCard } = await import("./ToolJobCard");
    mocks.status = "running";
    mocks.endpoint = "one";
    mocks.failed = false;
    mocks.cancel.mockReset();
    mocks.refetch.mockReset();
    mocks.cancel.mockReturnValue({
      unwrap: () =>
        Promise.resolve({ job_id: "job", status: "cancel_requested" }),
    });
    render(<ToolJobCard receipt={receipt} toolCall={toolCall} />);
    fireEvent.click(screen.getByRole("button", { name: /cancel/i }));
    expect(
      await screen.findByText("Cancellation requested"),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /cancel/i })).toBeDisabled();
    expect(mocks.cancel).toHaveBeenCalledTimes(1);
    expect(mocks.refetch).toHaveBeenCalledTimes(1);
  });
  it("renders terminal completion without querying or triggering cancellation", async () => {
    vi.resetModules();
    const { ToolJobCompletionCard } = await import("./ToolJobCard");
    mocks.cancel.mockClear();
    mocks.refetch.mockClear();
    render(
      <ToolJobCompletionCard receipt={{ ...receipt, status: "unknown" }} />,
    );
    expect(screen.getByText("Outcome unknown")).toBeInTheDocument();
    expect(mocks.cancel).not.toHaveBeenCalled();
    expect(mocks.refetch).not.toHaveBeenCalled();
  });
  it("does not allow cancellation after an endpoint change", async () => {
    vi.resetModules();
    const { ToolJobCard } = await import("./ToolJobCard");
    mocks.status = "running";
    mocks.endpoint = "one";
    mocks.failed = false;
    const { rerender } = render(
      <ToolJobCard receipt={receipt} toolCall={toolCall} />,
    );
    const button = screen.getByRole("button", { name: /cancel/i });
    expect(button).toBeEnabled();
    mocks.endpoint = "two";
    rerender(<ToolJobCard receipt={receipt} toolCall={toolCall} />);
    expect(button).toBeDisabled();
    fireEvent.click(button);
    expect(mocks.cancel).not.toHaveBeenCalled();
    expect(screen.getByText("Status unavailable")).toBeInTheDocument();
  });
});
