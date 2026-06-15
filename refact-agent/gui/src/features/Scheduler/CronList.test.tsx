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
  action_kind: "agent_turn",
  delivery_kind: "chat",
  delivery: { kind: "chat" },
  chat_id: "chat-1",
  target: "existing_chat",
  isolated: false,
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
    expect(screen.getByText("Agent")).toBeInTheDocument();
    expect(screen.getByText("Chat")).toBeInTheDocument();
    expect(screen.getByText("Durable")).toBeInTheDocument();
    expect(screen.getByText("Recurring")).toBeInTheDocument();
    expect(screen.getByText("Last fired")).toBeInTheDocument();
    expect(screen.getByText("3")).toBeInTheDocument();
  });

  it("renders command and isolated action badges", () => {
    render(
      <CronList
        tasks={[
          { ...task, id: "cron_command", action_kind: "command" },
          {
            ...task,
            id: "cron_isolated",
            action_kind: "agent_turn",
            target: "isolated",
            isolated: true,
          },
        ]}
        {...defaultProps}
      />,
    );

    expect(screen.getByText("Command")).toBeInTheDocument();
    expect(screen.getByText("Isolated")).toBeInTheDocument();
  });

  it("renders delivery badges", () => {
    render(
      <CronList
        tasks={[
          {
            ...task,
            id: "cron_webhook",
            delivery_kind: "webhook",
            delivery: { kind: "webhook", url: "https://example.com/hook" },
          },
          {
            ...task,
            id: "cron_notifier",
            delivery_kind: "notifier",
            delivery: { kind: "notifier", integration_id: "notifier_telegram" },
          },
        ]}
        {...defaultProps}
      />,
    );

    expect(screen.getByText("Webhook")).toBeInTheDocument();
    expect(screen.getByText("Notifier")).toBeInTheDocument();
  });

  it("renders expandable run history with status time and error", async () => {
    const { user } = render(
      <CronList
        tasks={[
          {
            ...task,
            recent_runs: [
              ...task.recent_runs,
              {
                at_ms: Date.UTC(2026, 0, 1, 8, 30),
                status: "failed",
                error: "network goblin",
              },
            ],
          },
        ]}
        {...defaultProps}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Run history (2)" }));

    expect(screen.getByText("failed")).toBeInTheDocument();
    expect(screen.getByText("network goblin")).toBeInTheDocument();
    expect(screen.getAllByText("fired").length).toBeGreaterThan(1);
  });

  it("renders webhook trigger management and copies hook path", async () => {
    const { user } = render(
      <CronList
        tasks={[
          {
            ...task,
            id: "cron_hook",
            human_schedule: "webhook",
            cron: "",
            trigger_kind: "webhook",
            next_fire_at_ms: 0,
            hook_id: "deploy",
          },
        ]}
        {...defaultProps}
      />,
    );

    expect(screen.getByText("hook_id: deploy")).toBeInTheDocument();
    expect(screen.getByText("/hooks/deploy")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Copy path" }));

    expect(screen.getByRole("button", { name: "Copied" })).toBeInTheDocument();
  });

  it("explains webhook path when hook id is absent", () => {
    render(
      <CronList
        tasks={[
          {
            ...task,
            id: "cron_hook_unknown",
            human_schedule: "webhook",
            cron: "",
            trigger_kind: "webhook",
            next_fire_at_ms: 0,
            hook_id: null,
          },
        ]}
        {...defaultProps}
      />,
    );

    expect(screen.getByText("hook_id unavailable")).toBeInTheDocument();
    expect(screen.getByText("/hooks/:name")).toBeInTheDocument();
  });

  it("edits webhook trigger descriptions without converting schedules", async () => {
    const onUpdate = vi.fn();
    const { user } = render(
      <CronList
        tasks={[
          {
            ...task,
            id: "cron_hook",
            human_schedule: "webhook",
            cron: "",
            trigger_kind: "webhook",
            next_fire_at_ms: 0,
            hook_id: "deploy",
          },
        ]}
        {...defaultProps}
        onUpdate={onUpdate}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Edit" }));
    await user.clear(screen.getByLabelText("Edit description"));
    await user.type(screen.getByLabelText("Edit description"), "Updated hook");
    expect(screen.getByLabelText("Edit hook ID")).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(onUpdate).toHaveBeenCalledWith("cron_hook", {
        description: "Updated hook",
      });
    });
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
