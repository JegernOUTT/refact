import { fireEvent, render, screen } from "../../utils/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  RecoveryRecord,
  QueuedItem,
} from "../../services/refact/chatSubscription";
import type { RootState } from "../../app/store";
import type { GoalSnapshot } from "../../services/refact/types";
import { RecoveryCard, RecoveryDetails } from "./RecoveryCard";

const mocks = vi.hoisted(() => {
  vi.resetModules();
  return {
    run: vi.fn(),
    abort: vi.fn(),
    controlGoal: vi.fn(),
    checkStatus: vi.fn(),
    cancelQueuedRequest: vi.fn(),
    cancelQueued: vi.fn(),
    queued: [] as QueuedItem[],
    phase: "idle",
    pending: false,
    queuedCommandId: null as string | null,
    goal: null as Partial<GoalSnapshot> | null,
    uncertain: false,
    blocked: false,
    status: null as string | null,
  };
});
vi.mock("../../hooks/useAppSelector", () => ({
  useAppSelector: (selector: (state: RootState) => unknown) =>
    selector({} as RootState),
}));
vi.mock("../../features/Chat/Thread/selectors", async (importOriginal) => ({
  ...(await importOriginal<
    typeof import("../../features/Chat/Thread/selectors")
  >()),
  selectQueuedItemsById: () => mocks.queued,
  selectThreadById: () => ({ goal: mocks.goal }),
  selectRecoveryById: () => recovery,
}));
vi.mock("../../hooks/useChatActions", () => ({
  useChatActions: () => ({
    abort: mocks.abort,
    controlGoal: mocks.controlGoal,
    cancelQueued: mocks.cancelQueued,
  }),
}));
vi.mock("../../hooks/useChatCommandRequest", () => ({
  useChatCommandRequest: () => ({
    run: mocks.run,
    blocked: mocks.blocked,
    error: null,
    status: mocks.status,
    uncertain: mocks.uncertain,
    checkStatus: mocks.checkStatus,
    cancelQueuedRequest: mocks.cancelQueuedRequest,
    phase: mocks.phase,
    pending: mocks.pending,
    queuedCommandId: mocks.queuedCommandId,
  }),
}));
vi.mock("../ChatForm/RecoveryPolicySettings", () => ({
  RecoveryPolicySettings: () => <div>Saved policy editor</div>,
}));
vi.mock("../Chat/ModelSelector", () => ({
  ModelSelector: ({
    value,
    onValueChange,
  }: {
    value: string;
    onValueChange: (value: string) => void;
  }) => (
    <select
      aria-label="Retry model"
      value={value}
      onChange={(event) => onValueChange(event.target.value)}
    >
      <option value="">Keep original model</option>
      <option value="other-model">Other model</option>
    </select>
  ),
}));
const recovery: RecoveryRecord = {
  operation_id: "op",
  run_id: "run",
  revision: 3,
  state: "needs_action",
  policy: {
    mode: "transient",
    max_retries: 2,
    delay_ms: 10000,
    total_timeout_ms: 600000,
  },
  policy_source: "project",
  manual: false,
  retries_used: 0,
  latest_error: {
    category: "NetworkFailure",
    title: "Disconnected",
    explanation: "Network error",
    suggested_action: "retry",
    is_retryable: true,
    raw_error: "offline",
  },
  attempts: [],
};
beforeEach(() => {
  vi.clearAllMocks();
  mocks.goal = null;
  mocks.uncertain = false;
  mocks.blocked = false;
  mocks.status = null;
  mocks.phase = "idle";
  mocks.pending = false;
  mocks.queuedCommandId = null;
  mocks.queued = [];
  mocks.cancelQueued.mockResolvedValue(true);
  mocks.cancelQueuedRequest.mockResolvedValue(undefined);
  mocks.abort.mockResolvedValue(undefined);
  mocks.controlGoal.mockResolvedValue(undefined);
});
describe("RecoveryCard", () => {
  it("restores queued cancellation after remount without a hook-owned request", () => {
    mocks.queued = [
      {
        client_request_id: "restored",
        command_type: "recovery_retry",
        preview: "Retry",
        priority: false,
      },
    ];
    render(<RecoveryCard chatId="chat" recovery={recovery} />);
    expect(
      screen.queryByRole("button", { name: "Retry" }),
    ).not.toBeInTheDocument();
    expect(screen.getByText(/Retry accepted and queued/)).toBeVisible();
    fireEvent.click(
      screen.getByRole("button", { name: "Cancel queued retry" }),
    );
    expect(mocks.cancelQueued).toHaveBeenCalledWith("restored");
  });
  it("shows only queued cancellation or pending confirmation instead of another Retry", () => {
    mocks.phase = "queued";
    mocks.pending = true;
    mocks.blocked = true;
    mocks.queuedCommandId = "queued-retry";
    const view = render(<RecoveryCard chatId="chat" recovery={recovery} />);
    expect(
      screen.queryByRole("button", { name: "Retry" }),
    ).not.toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", { name: "Cancel queued retry" }),
    );
    expect(mocks.cancelQueuedRequest).toHaveBeenCalledOnce();
    mocks.phase = "awaiting_ack";
    mocks.queuedCommandId = null;
    view.rerender(<RecoveryCard chatId="chat" recovery={recovery} />);
    expect(
      screen.getByRole("button", { name: "Awaiting confirmation…" }),
    ).toBeDisabled();
  });
  it("shows every distinct current, attempt, ordinary and archived diagnostic without slicing", () => {
    const long = `${"x".repeat(500)} final diagnostic`;
    const { container } = render(
      <RecoveryCard
        chatId="chat"
        recovery={{
          ...recovery,
          latest_error: { ...recovery.latest_error, raw_error: long },
          attempts: [
            {
              at_ms: 1,
              retry_number: 0,
              outcome: "network_failure",
              error: recovery.latest_error,
            },
          ],
        }}
        errors={[{ role: "error", content: "ordinary failure" }]}
        archived={[
          {
            ...recovery,
            run_id: "old",
            latest_error: {
              ...recovery.latest_error,
              raw_error: "archive failure",
            },
          },
        ]}
      />,
    );
    expect(container.textContent).toContain(long);
    expect(screen.getByText("offline")).toBeVisible();
    expect(screen.getByText("ordinary failure")).toBeVisible();
    expect(screen.getByText("archive failure")).toBeVisible();
    expect(screen.getAllByRole("button", { name: "Retry" })).toHaveLength(1);
  });
  it("never displays success cards or historical success outcomes", () => {
    const view = render(
      <RecoveryCard
        chatId="chat"
        recovery={{ ...recovery, state: "succeeded" }}
      />,
    );
    expect(
      screen.queryByRole("region", { name: "Recovery status" }),
    ).not.toBeInTheDocument();
    view.rerender(
      <RecoveryDetails
        recovery={{
          ...recovery,
          attempts: [{ at_ms: 1, retry_number: 1, outcome: "succeeded" }],
        }}
      />,
    );
    expect(screen.queryByText(/Succeeded|Recovered/)).not.toBeInTheDocument();
  });
  it("sends a revision guarded retry and explicit model/options payload", () => {
    render(<RecoveryCard chatId="chat" recovery={recovery} />);
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(mocks.run).toHaveBeenLastCalledWith(
      {
        type: "recovery_retry",
        run_id: "run",
        expected_revision: 3,
        options: {
          mode: "transient",
          max_retries: 1,
          delay_ms: 10000,
          total_timeout_ms: 600000,
        },
      },
      expect.any(Function),
    );
    fireEvent.click(screen.getByRole("button", { name: "Options" }));
    fireEvent.change(screen.getByRole("spinbutton", { name: "New requests" }), {
      target: { value: "3" },
    });
    fireEvent.change(screen.getByRole("combobox", { name: "Retry model" }), {
      target: { value: "other-model" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(mocks.run).toHaveBeenLastCalledWith(
      expect.objectContaining({
        model: "other-model",
        options: expect.objectContaining({ max_retries: 3 }) as unknown,
      }),
      expect.any(Function),
    );
  });
  it("uses mutually exclusive collapsible panels with inert safety and user-only collapse key", () => {
    const view = render(
      <RecoveryCard chatId="chat" recovery={recovery} submissionKey="user-1" />,
    );
    const details = screen.getByRole("button", { name: "Details" });
    const options = screen.getByRole("button", { name: "Options" });
    fireEvent.click(details);
    expect(details).toHaveAttribute("aria-expanded", "true");
    fireEvent.click(options);
    expect(details).toHaveAttribute("aria-expanded", "false");
    expect(
      document.getElementById(details.getAttribute("aria-controls") ?? ""),
    ).toHaveAttribute("inert");
    expect(options).toHaveAttribute("aria-expanded", "true");
    view.rerender(
      <RecoveryCard
        chatId="chat"
        recovery={{ ...recovery, revision: 4 }}
        submissionKey="user-1"
      />,
    );
    expect(options).toHaveAttribute("aria-expanded", "true");
    view.rerender(
      <RecoveryCard
        chatId="chat"
        recovery={recovery}
        submissionKey="queued-user-2"
      />,
    );
    expect(options).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(options);
    fireEvent.click(options);
    expect(options).toHaveAttribute("aria-expanded", "false");
  });
  it("scopes saved policy separately and rejects invalid next-retry values", () => {
    render(<RecoveryCard chatId="chat" recovery={recovery} />);
    fireEvent.click(screen.getByRole("button", { name: "Options" }));
    fireEvent.click(screen.getByRole("radio", { name: "Saved thread policy" }));
    expect(screen.getByText("Saved policy editor")).toBeVisible();
    fireEvent.click(screen.getByRole("radio", { name: "Next retry only" }));
    fireEvent.change(screen.getByRole("spinbutton", { name: "New requests" }), {
      target: { value: "1.5" },
    });
    expect(screen.getByRole("button", { name: "Retry" })).toBeDisabled();
  });
  it("accepts whole milliseconds, rejects unchanged witnesses and resets one-off options for the next run", () => {
    const view = render(<RecoveryCard chatId="chat" recovery={recovery} />);
    fireEvent.click(screen.getByRole("button", { name: "Options" }));
    fireEvent.change(
      screen.getByRole("spinbutton", {
        name: "Delay between retries (seconds)",
      }),
      { target: { value: "1.001" } },
    );
    fireEvent.change(screen.getByRole("combobox", { name: "Retry model" }), {
      target: { value: "other-model" },
    });
    const retry = screen.getByRole("button", {
      name: "Retry",
    });
    expect(retry).toBeEnabled();
    fireEvent.click(retry);
    const [command, applied] = mocks.run.mock.lastCall as [
      Record<string, unknown>,
      (state: RootState) => boolean,
    ];
    expect(command.options).toMatchObject({ delay_ms: 1001 });
    expect(applied({} as RootState)).toBe(false);
    view.rerender(
      <RecoveryCard
        chatId="chat"
        recovery={{ ...recovery, run_id: "new-run" }}
      />,
    );
    expect(screen.getByRole("button", { name: "Retry" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Options" })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
  });
  it("keeps Stop during active recovery and respects cooldown", () => {
    render(
      <RecoveryCard
        chatId="chat"
        recovery={{
          ...recovery,
          state: "waiting",
          retry_not_before_ms: Date.now() + 30000,
        }}
      />,
    );
    expect(screen.getByRole("button", { name: "Retry now" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(mocks.abort).toHaveBeenCalledOnce();
    expect(
      screen.getByRole("button", { name: "Stop requested…" }),
    ).toBeDisabled();
  });
  it("suppresses retry when uncertain or held and reports queued status", () => {
    mocks.uncertain = true;
    mocks.blocked = true;
    const view = render(<RecoveryCard chatId="chat" recovery={recovery} />);
    expect(
      screen.queryByRole("button", { name: "Retry" }),
    ).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Check status" }));
    expect(mocks.checkStatus).toHaveBeenCalledOnce();
    mocks.uncertain = false;
    mocks.goal = { status: "paused" };
    view.rerender(<RecoveryCard chatId="chat" recovery={recovery} />);
    expect(
      screen.queryByRole("button", { name: "Retry" }),
    ).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Resume goal" }));
    expect(mocks.controlGoal).toHaveBeenCalledWith("resume");
    mocks.goal = null;
    mocks.status = "Retry accepted and queued.";
    view.rerender(<RecoveryCard chatId="chat" recovery={recovery} />);
    expect(screen.getByText("Retry accepted and queued.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Retry" })).toBeDisabled();
  });
});
