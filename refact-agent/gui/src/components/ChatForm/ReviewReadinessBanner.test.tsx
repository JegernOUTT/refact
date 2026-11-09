import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { RootState } from "../../app/store";
import type { ReviewReadiness } from "../../services/refact/chatSubscription";
let ReviewReadinessBanner: typeof import("./ReviewReadinessBanner").ReviewReadinessBanner;

const mocks = vi.hoisted(() => ({
  readiness: undefined as ReviewReadiness | undefined,
  endpoint: "engine-A",
  query: { currentData: undefined as unknown, isError: false },
  cancel: vi.fn(),
  getJob: vi.fn(),
}));
vi.mock("../../hooks/useAppSelector", () => ({
  useAppSelector: (selector: (state: RootState) => unknown) =>
    selector({
      config: { apiKey: "key" },
      chat: { threads: { chat: { review_readiness: mocks.readiness } } },
    } as unknown as RootState),
}));
vi.mock("../../services/refact/apiUrl", () => ({
  getEngineEndpointIdentity: () => mocks.endpoint,
}));
vi.mock("../../services/refact/toolJobs", () => ({
  isTerminalToolJob: (status: string) =>
    ["succeeded", "failed", "cancelled", "unknown"].includes(status),
  useGetToolJobQuery: (...args: unknown[]) => {
    mocks.getJob(...args);
    return mocks.query;
  },
  useCancelToolJobMutation: () => [mocks.cancel, { isLoading: false }],
}));

const receipt = {
  job_id: "job",
  chat_id: "chat",
  tool_call_id: "call",
  tool_name: "review_readiness",
  model: "",
  turn_id: "turn",
  status: "accepted" as const,
  result_url: "/ignored",
  cancel_url: "/ignored",
};
describe("Review readiness composer feedback", () => {
  beforeEach(async () => {
    vi.resetModules();
    ({ ReviewReadinessBanner } = await import("./ReviewReadinessBanner"));
    vi.clearAllMocks();
    mocks.endpoint = "engine-A";
    mocks.readiness = { status: "pending", turn_nonce: "A", receipt };
    mocks.query = {
      currentData: { receipt, status: "running", revision: 1 },
      isError: false,
    };
    mocks.cancel.mockReturnValue({
      unwrap: () =>
        Promise.resolve({ job_id: "job", status: "cancel_requested" }),
    });
  });
  afterEach(cleanup);
  it("preserves the same draft input and focus across pending/ready/unavailable updates", () => {
    const Composer = () => (
      <>
        <ReviewReadinessBanner chatId="chat" />
        <textarea aria-label="Draft" defaultValue="Unsent draft" />
      </>
    );
    const view = render(<Composer />);
    const input = screen.getByLabelText<HTMLTextAreaElement>("Draft");
    input.focus();
    expect(screen.getByRole("status").textContent).toContain(
      "Preparing review",
    );
    mocks.readiness = { status: "ready", turn_nonce: "A" };
    view.rerender(<Composer />);
    expect(screen.queryByTestId("review-readiness-banner")).toBeNull();
    mocks.readiness = { status: "unavailable", turn_nonce: "B" };
    view.rerender(<Composer />);
    expect(screen.getByLabelText("Draft")).toBe(input);
    expect(input.value).toBe("Unsent draft");
    expect(document.activeElement).toBe(input);
    expect(screen.queryByRole("button", { name: /retry/i })).toBeNull();
  });
  it("requests actual cancellation without asserting that it completed", async () => {
    render(<ReviewReadinessBanner chatId="chat" />);
    fireEvent.click(
      screen.getByRole("button", { name: "Cancel review preparation" }),
    );
    await screen.findByText("Review preparation cancellation requested");
    expect(mocks.cancel).toHaveBeenCalledWith({
      chatId: "chat",
      jobId: "job",
      endpoint: "engine-A",
    });
  });
  it("disables stale endpoint cancellation and ignores old endpoint status", () => {
    const view = render(<ReviewReadinessBanner chatId="chat" />);
    mocks.endpoint = "engine-B";
    mocks.query.currentData = { receipt, status: "cancelled", revision: 2 };
    view.rerender(<ReviewReadinessBanner chatId="chat" />);
    expect(
      screen.getByText("Review preparation status unavailable"),
    ).toBeTruthy();
    expect(screen.getByRole<HTMLButtonElement>("button").disabled).toBe(true);
    expect(mocks.getJob.mock.lastCall?.[1]).toMatchObject({ skip: true });
  });
  it("shows actual terminal failure/cancellation without claiming Review is ready", () => {
    mocks.query.currentData = { receipt, status: "cancelled", revision: 2 };
    render(<ReviewReadinessBanner chatId="chat" />);
    expect(screen.getByRole("status").textContent).toContain("cancelled");
    expect(screen.queryByRole("button")).toBeNull();
  });
});
