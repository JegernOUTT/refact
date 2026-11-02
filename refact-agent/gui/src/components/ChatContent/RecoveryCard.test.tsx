import { fireEvent, render, screen } from "../../utils/test-utils";
import { describe, expect, it, vi } from "vitest";
import type { RecoveryRecord } from "../../services/refact/chatSubscription";
import type { GoalSnapshot } from "../../services/refact/types";
import { RecoveryCard } from "./RecoveryCard";

const mocks = vi.hoisted(() => {
  vi.resetModules();
  return {
    run: vi.fn(),
    abort: vi.fn(),
    controlGoal: vi.fn(),
    goal: null as Partial<GoalSnapshot> | null,
  };
});
vi.mock("../../hooks/useAppSelector", () => ({
  useAppSelector: () => mocks.goal,
}));
vi.mock("../../hooks/useChatActions", () => ({
  useChatActions: () => ({
    abort: mocks.abort,
    controlGoal: mocks.controlGoal,
  }),
}));
vi.mock("../../hooks/useChatCommandRequest", () => ({
  useChatCommandRequest: () => ({
    run: mocks.run,
    blocked: false,
    error: null,
    status: null,
    uncertain: false,
  }),
}));
vi.mock("../Chat/ModelSelector", () => ({
  ModelSelector: ({
    value,
    onValueChange,
    allowUnset,
    unsetLabel,
  }: {
    value: string;
    onValueChange: (value: string) => void;
    allowUnset?: boolean;
    unsetLabel?: string;
  }) => (
    <select
      aria-label="Retry model"
      value={value}
      onChange={(event) => onValueChange(event.target.value)}
    >
      {allowUnset && <option value="">{unsetLabel}</option>}
      <option value="other-model">Other model</option>
    </select>
  ),
}));
const recovery: RecoveryRecord = {
  operation_id: "op",
  run_id: "run",
  revision: 3,
  state: "waiting",
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
describe("RecoveryCard", () => {
  it("keeps the original model until an explicit model choice is made", () => {
    mocks.goal = null;
    mocks.run.mockReset();
    render(
      <RecoveryCard
        chatId="chat"
        recovery={{ ...recovery, state: "needs_action" }}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Retry options" }));
    expect(screen.getByRole("combobox", { name: "Retry model" })).toHaveValue(
      "",
    );
    expect(
      screen.getByRole("option", { name: "Keep original model" }),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(mocks.run.mock.lastCall?.[0] as unknown).not.toHaveProperty("model");
    fireEvent.change(screen.getByRole("combobox", { name: "Retry model" }), {
      target: { value: "other-model" },
    });
    fireEvent.click(
      screen.getByRole("button", { name: "Retry with other-model" }),
    );
    expect(mocks.run).toHaveBeenLastCalledWith(
      expect.objectContaining({ model: "other-model" }),
      expect.any(Function),
    );
  });

  it("retains diagnostic text and original-request history after success with correct plural labels", () => {
    mocks.goal = null;
    const raw = `https://provider.invalid/${"x".repeat(
      497,
    )}\nsecond diagnostic line`;
    const view = render(
      <RecoveryCard
        chatId="chat"
        recovery={{
          ...recovery,
          state: "succeeded",
          retries_used: 1,
          latest_error: { ...recovery.latest_error, raw_error: raw },
          attempts: [
            {
              at_ms: 1,
              retry_number: 0,
              outcome: "network_failure",
              error: recovery.latest_error,
            },
            { at_ms: 2, retry_number: 1, outcome: "succeeded" },
          ],
        }}
      />,
    );
    const summary = screen.getByText("Recovered after 1 new request");
    fireEvent.click(summary);
    expect(
      screen.getByText("Original request: Network failure"),
    ).toBeInTheDocument();
    expect(screen.getByText("Retry 1: Succeeded")).toBeInTheDocument();
    expect(
      screen.getByRole("region", { name: "Recovery diagnostics" }).textContent,
    ).toContain(raw);
    expect(screen.queryByText(/Retry 0/)).not.toBeInTheDocument();
    view.rerender(
      <RecoveryCard
        chatId="chat"
        recovery={{ ...recovery, state: "succeeded", retries_used: 2 }}
      />,
    );
    expect(
      screen.getByText("Recovered after 2 new requests"),
    ).toBeInTheDocument();
  });

  it("keeps Stop available during waiting and respects mandatory cooldown", () => {
    mocks.goal = null;
    mocks.abort.mockResolvedValue(undefined);
    render(
      <RecoveryCard
        chatId="chat"
        recovery={{ ...recovery, retry_not_before_ms: Date.now() + 30000 }}
      />,
    );
    expect(screen.getByRole("button", { name: "Retry now" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(mocks.abort).toHaveBeenCalledOnce();
    expect(screen.getByText("Stop requested…")).toBeInTheDocument();
  });
  it("sends an explicitly authorised single retry with revision", () => {
    mocks.goal = null;
    mocks.run.mockReset();
    render(
      <RecoveryCard
        chatId="chat"
        recovery={{ ...recovery, state: "needs_action" }}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(mocks.run).toHaveBeenCalledWith(
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
  });
  it("requires explicit resume for a paused goal without treating manual Stop as a goal hold", () => {
    mocks.goal = { status: "paused" };
    mocks.controlGoal.mockResolvedValue(undefined);
    const view = render(
      <RecoveryCard
        chatId="chat"
        recovery={{ ...recovery, state: "needs_action" }}
      />,
    );
    expect(screen.getByRole("button", { name: "Retry" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Resume goal" }));
    expect(mocks.controlGoal).toHaveBeenCalledWith("resume");
    mocks.goal = { status: "stopped", stop_reason: "manual_abort" };
    view.rerender(
      <RecoveryCard
        chatId="chat"
        recovery={{ ...recovery, state: "cancelled" }}
      />,
    );
    expect(screen.getByRole("button", { name: "Retry" })).toBeEnabled();
    expect(
      screen.queryByRole("button", { name: "Resume goal" }),
    ).not.toBeInTheDocument();
    mocks.goal = { status: "stopped" };
    view.rerender(
      <RecoveryCard
        chatId="chat"
        recovery={{
          ...recovery,
          state: "cancelled",
          terminal_reason: "cancelled",
        }}
      />,
    );
    expect(screen.getByRole("button", { name: "Retry" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Resume goal" })).toBeEnabled();
    mocks.goal = null;
  });
  it("keeps Stop pending across a waiting-to-attempting event and rejects fractional request counts", () => {
    mocks.goal = null;
    mocks.abort.mockResolvedValue(undefined);
    const view = render(<RecoveryCard chatId="chat" recovery={recovery} />);
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    view.rerender(
      <RecoveryCard
        chatId="chat"
        recovery={{ ...recovery, state: "attempting" }}
      />,
    );
    expect(
      screen.getByRole("button", { name: "Stop requested…" }),
    ).toBeDisabled();
    view.rerender(
      <RecoveryCard
        chatId="chat"
        recovery={{ ...recovery, state: "cancelled" }}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Retry options" }));
    fireEvent.change(screen.getByRole("spinbutton", { name: "New requests" }), {
      target: { value: "1.5" },
    });
    expect(screen.getByRole("button", { name: "Retry" })).toBeDisabled();
  });
});
