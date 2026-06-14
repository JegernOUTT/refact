import { describe, expect, it, vi } from "vitest";
import { screen, render, waitFor } from "../../utils/test-utils";
import { CronList } from "./CronList";
import type { CronTask } from "../../services/refact/schedulerApi";

const task: CronTask = {
  id: "cron_1",
  cron: "7 * * * *",
  human_schedule: "hourly at :07",
  description: "Hourly frog check",
  prompt: "Check frogs",
  recurring: true,
  durable: true,
  next_fire_at_ms: Date.UTC(2026, 0, 1, 9, 7),
  fire_count: 3,
  created_at_ms: Date.UTC(2026, 0, 1, 8, 0),
  enabled: true,
  paused: false,
  trigger_kind: "cron",
  tz: "UTC",
  every_ms: null,
  at_ms: null,
  last_status: "fired",
  last_error: null,
  recent_runs: [
    {
      at_ms: Date.UTC(2026, 0, 1, 8, 7),
      status: "fired",
      error: null,
    },
  ],
};

const defaultProps = {
  onDelete: vi.fn(),
  onToggleEnabled: vi.fn(),
  onRunNow: vi.fn(),
  onUpdate: vi.fn(),
};

describe("CronList", () => {
  it("renders cron task fixture with status and last-fired fields", () => {
    render(<CronList tasks={[task]} {...defaultProps} />);

    expect(screen.getByText("hourly at :07")).toBeInTheDocument();
    expect(screen.getByText("7 * * * *")).toBeInTheDocument();
    expect(screen.getByText("Hourly frog check")).toBeInTheDocument();
    expect(screen.getByText("Enabled")).toBeInTheDocument();
    expect(screen.getByText("fired")).toBeInTheDocument();
    expect(screen.getByText("Cron")).toBeInTheDocument();
    expect(screen.getByText("Durable")).toBeInTheDocument();
    expect(screen.getByText("Recurring")).toBeInTheDocument();
    expect(screen.getByText("Last fired")).toBeInTheDocument();
    expect(screen.getByText("3")).toBeInTheDocument();
  });

  it("calls delete with the task id", async () => {
    const onDelete = vi.fn();
    const { user } = render(
      <CronList tasks={[task]} {...defaultProps} onDelete={onDelete} />,
    );

    await user.click(screen.getByRole("button", { name: "Delete" }));

    expect(onDelete).toHaveBeenCalledWith("cron_1");
  });

  it("calls pause and run-now actions", async () => {
    const onToggleEnabled = vi.fn();
    const onRunNow = vi.fn();
    const { user } = render(
      <CronList
        tasks={[task]}
        {...defaultProps}
        onToggleEnabled={onToggleEnabled}
        onRunNow={onRunNow}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Pause" }));
    await user.click(screen.getByRole("button", { name: "Run now" }));

    expect(onToggleEnabled).toHaveBeenCalledWith("cron_1", false);
    expect(onRunNow).toHaveBeenCalledWith("cron_1");
  });

  it("shows resume for paused tasks", async () => {
    const onToggleEnabled = vi.fn();
    const pausedTask = { ...task, enabled: false, paused: true };
    const { user } = render(
      <CronList
        tasks={[pausedTask]}
        {...defaultProps}
        onToggleEnabled={onToggleEnabled}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Resume" }));

    expect(screen.getByText("Paused")).toBeInTheDocument();
    expect(onToggleEnabled).toHaveBeenCalledWith("cron_1", true);
  });

  it("submits edited cron schedule fields", async () => {
    const onUpdate = vi.fn();
    const { user } = render(
      <CronList tasks={[task]} {...defaultProps} onUpdate={onUpdate} />,
    );

    await user.click(screen.getByRole("button", { name: "Edit" }));
    await user.clear(screen.getByLabelText("Edit description"));
    await user.type(screen.getByLabelText("Edit description"), "Updated frogs");
    await user.clear(screen.getByLabelText("Edit cron expression"));
    await user.type(
      screen.getByLabelText("Edit cron expression"),
      "*/15 * * * *",
    );
    await user.clear(screen.getByLabelText("Edit timezone"));
    await user.type(screen.getByLabelText("Edit timezone"), "Asia/Tokyo");
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(onUpdate).toHaveBeenCalledWith("cron_1", {
        description: "Updated frogs",
        cron: "*/15 * * * *",
        tz: "Asia/Tokyo",
      });
    });
  });
});
