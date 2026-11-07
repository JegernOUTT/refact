import { describe, expect, it } from "vitest";
import type { LiveQueueDiagnostics as Diagnostics } from "../../services/refact/performance";
import { render, screen, within } from "../../utils/test-utils";
import { LiveQueueDiagnostics } from "./LiveQueueDiagnostics";

const pending = {
  pending_count: 2,
  oldest_pending_age_ms: 25_000,
  unknown_age_count: 1,
};
const diagnostics: Diagnostics = {
  sampled_at_ms: 1_700_000_000_000,
  coverage: {
    sessions_total: 9,
    inspected: 6,
    skipped_busy: 2,
    skipped_budget: 1,
    partial: true,
  },
  commands: pending,
  deliveries: {
    pending_count: 1,
    oldest_pending_age_ms: null,
    unknown_age_count: 1,
  },
  question_answers: {
    pending_count: 1,
    oldest_pending_age_ms: 500,
    unknown_age_count: 0,
  },
  blockers: [{ reason: "executing_tools", ...pending }],
  progress: {
    oldest_queue_progress_age_ms: null,
    oldest_chat_progress_age_ms: 4_000,
    unknown_queue_progress_sessions: 3,
    unknown_chat_progress_sessions: 0,
  },
  accepted_waiting_first_output: pending,
  phases: [{ phase: "stream_invocation", ...pending }],
};

describe("LiveQueueDiagnostics", () => {
  it("shows partial coverage, distinct counts, unknown ages and first-output phases", () => {
    render(<LiveQueueDiagnostics diagnostics={diagnostics} />);
    expect(screen.getByText(/Observed 6 of 9 sessions/)).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent(
      "2 sessions skipped because they were busy; 1 partially sampled or skipped by the sampling limit. Counts are lower bounds.",
    );
    expect(
      screen.getByText("Accepted messages awaiting first output"),
    ).toBeInTheDocument();
    const phase = screen.getByText("Stream invocation").parentElement;
    if (!phase) throw new Error("Phase aggregate missing");
    expect(within(phase).getByText("Oldest age: 25 s")).toBeInTheDocument();
    const deliveries = screen.getByText("Deliveries").parentElement;
    if (!deliveries) throw new Error("Delivery aggregate missing");
    expect(
      within(deliveries).getByText("Oldest age: Unknown"),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/Question answers are included in pending commands/),
    ).toBeInTheDocument();
    expect(screen.getByText("Executing tools")).toBeInTheDocument();
    expect(screen.getByText("Unknown sessions: 3")).toBeInTheDocument();
  });

  it("reports unknown registry size and unavailable legacy data without a healthy zero", () => {
    const view = render(
      <LiveQueueDiagnostics
        diagnostics={{
          ...diagnostics,
          coverage: {
            ...diagnostics.coverage,
            sessions_total: null,
            inspected: 0,
          },
        }}
      />,
    );
    expect(
      screen.getByText(/Observed 0 of unknown sessions/),
    ).toBeInTheDocument();
    view.rerender(<LiveQueueDiagnostics />);
    expect(
      screen.getByText("Live queue diagnostics unavailable."),
    ).toBeInTheDocument();
    expect(screen.queryByText("Commands")).not.toBeInTheDocument();
  });
});
