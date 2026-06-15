import { describe, expect, it, vi } from "vitest";
import { screen, render, waitFor } from "../../utils/test-utils";
import { CronCreateForm } from "./CronCreateForm";

describe("CronCreateForm", () => {
  it("presents trigger action and delivery builder sections", () => {
    render(<CronCreateForm onSubmit={vi.fn()} taskCount={0} />);

    expect(screen.getByRole("group", { name: "Trigger" })).toBeInTheDocument();
    expect(screen.getByRole("group", { name: "Action" })).toBeInTheDocument();
    expect(screen.getByRole("group", { name: "Delivery" })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Webhook" })).toBeInTheDocument();
  });

  it("validates required description", async () => {
    const onSubmit = vi.fn();
    const { user } = render(
      <CronCreateForm onSubmit={onSubmit} taskCount={0} />,
    );

    await user.type(screen.getByLabelText("Prompt"), "Check frogs");
    await user.click(screen.getByRole("button", { name: "Create" }));

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Description is required.",
    );
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("submits valid cron preset values with the existing default shape", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const { user } = render(
      <CronCreateForm onSubmit={onSubmit} taskCount={0} />,
    );

    await user.type(screen.getByLabelText("Description"), "Hourly frog check");
    await user.type(screen.getByLabelText("Prompt"), "Check frogs");
    await user.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => {
      expect(onSubmit).toHaveBeenCalledWith({
        cron: "7 * * * *",
        prompt: "Check frogs",
        recurring: true,
        durable: false,
        description: "Hourly frog check",
      });
    });
  });

  it("submits timezone with cron schedules", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const { user } = render(
      <CronCreateForm onSubmit={onSubmit} taskCount={0} />,
    );

    await user.type(screen.getByLabelText("Timezone"), "UTC");
    await user.type(screen.getByLabelText("Description"), "Hourly frog check");
    await user.type(screen.getByLabelText("Prompt"), "Check frogs");
    await user.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => {
      expect(onSubmit).toHaveBeenCalledWith({
        cron: "7 * * * *",
        tz: "UTC",
        prompt: "Check frogs",
        recurring: true,
        durable: false,
        description: "Hourly frog check",
      });
    });
  });

  it("switches to interval schedules", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const { user } = render(
      <CronCreateForm onSubmit={onSubmit} taskCount={0} />,
    );

    await user.click(screen.getByRole("radio", { name: "Interval" }));
    await user.clear(screen.getByRole("textbox", { name: "Interval" }));
    await user.type(screen.getByRole("textbox", { name: "Interval" }), "45m");
    await user.type(
      screen.getByLabelText("Description"),
      "Interval frog check",
    );
    await user.type(screen.getByLabelText("Prompt"), "Check frogs");
    await user.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => {
      expect(onSubmit).toHaveBeenCalledWith({
        every: "45m",
        prompt: "Check frogs",
        recurring: true,
        durable: false,
        description: "Interval frog check",
      });
    });
  });

  it("switches to one-shot schedules", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const { user } = render(
      <CronCreateForm onSubmit={onSubmit} taskCount={0} />,
    );

    await user.click(screen.getByRole("radio", { name: "One-shot" }));
    await user.clear(screen.getByRole("textbox", { name: "One-shot time" }));
    await user.type(
      screen.getByRole("textbox", { name: "One-shot time" }),
      "in 2h",
    );
    await user.type(screen.getByLabelText("Description"), "One frog check");
    await user.type(screen.getByLabelText("Prompt"), "Check frogs once");
    await user.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => {
      expect(onSubmit).toHaveBeenCalledWith({
        at: "in 2h",
        prompt: "Check frogs once",
        recurring: false,
        durable: false,
        description: "One frog check",
      });
    });
  });

  it("submits webhook trigger jobs with hook id", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const { user } = render(
      <CronCreateForm onSubmit={onSubmit} taskCount={0} />,
    );

    await user.click(screen.getByRole("radio", { name: "Webhook" }));
    await user.type(screen.getByLabelText("Hook ID"), "deploy");
    await user.type(screen.getByLabelText("Description"), "Deploy hook");
    await user.click(screen.getByRole("radio", { name: "Command action" }));
    await user.type(
      screen.getByRole("textbox", { name: "Command" }),
      "echo hi",
    );
    await user.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => {
      expect(onSubmit).toHaveBeenCalledWith({
        trigger: { kind: "webhook", hook_id: "deploy" },
        command: "echo hi",
        durable: false,
        description: "Deploy hook",
      });
    });
  });

  it("blocks webhook trigger jobs without a hook id", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const { user } = render(
      <CronCreateForm onSubmit={onSubmit} taskCount={0} />,
    );

    await user.click(screen.getByRole("radio", { name: "Webhook" }));
    await user.type(screen.getByLabelText("Description"), "Deploy hook");
    await user.type(screen.getByLabelText("Prompt"), "Run deploy");
    await user.click(screen.getByRole("button", { name: "Create" }));

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Webhook hook ID is required.",
    );
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("toggles action fields when switching action kind", async () => {
    const { user } = render(
      <CronCreateForm onSubmit={vi.fn()} taskCount={0} />,
    );

    expect(screen.getByLabelText("Prompt")).toBeInTheDocument();
    expect(
      screen.queryByRole("textbox", { name: "Command" }),
    ).not.toBeInTheDocument();

    await user.click(screen.getByRole("radio", { name: "Command action" }));

    expect(screen.queryByLabelText("Prompt")).not.toBeInTheDocument();
    expect(
      screen.getByRole("textbox", { name: "Command" }),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Working directory")).toBeInTheDocument();
    expect(screen.getByLabelText("Timeout")).toBeInTheDocument();
  });

  it("toggles delivery fields when switching delivery kind", async () => {
    const { user } = render(
      <CronCreateForm onSubmit={vi.fn()} taskCount={0} />,
    );

    expect(screen.getByRole("radio", { name: "Chat delivery" })).toBeChecked();
    expect(screen.queryByLabelText("Webhook URL")).not.toBeInTheDocument();
    expect(
      screen.queryByLabelText("Notifier integration ID"),
    ).not.toBeInTheDocument();

    await user.click(screen.getByRole("radio", { name: "Webhook delivery" }));

    expect(screen.getByLabelText("Webhook URL")).toBeInTheDocument();
    expect(screen.getByLabelText("Webhook token")).toBeInTheDocument();
    expect(
      screen.queryByLabelText("Notifier integration ID"),
    ).not.toBeInTheDocument();

    await user.click(screen.getByRole("radio", { name: "Notifier delivery" }));

    expect(screen.queryByLabelText("Webhook URL")).not.toBeInTheDocument();
    expect(
      screen.getByLabelText("Notifier integration ID"),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Notifier target")).toBeInTheDocument();
  });

  it("blocks command actions without command text", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const { user } = render(
      <CronCreateForm onSubmit={onSubmit} taskCount={0} />,
    );

    await user.type(screen.getByLabelText("Description"), "Command frog check");
    await user.click(screen.getByRole("radio", { name: "Command action" }));
    await user.click(screen.getByRole("button", { name: "Create" }));

    expect(screen.getByRole("alert")).toHaveTextContent("Command is required.");
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("submits command action fields without prompt", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const { user } = render(
      <CronCreateForm onSubmit={onSubmit} taskCount={0} />,
    );

    await user.type(screen.getByLabelText("Description"), "Command frog check");
    await user.click(screen.getByRole("radio", { name: "Command action" }));
    await user.type(
      screen.getByRole("textbox", { name: "Command" }),
      "npm test",
    );
    await user.type(
      screen.getByLabelText("Working directory"),
      "refact-agent/gui",
    );
    await user.type(screen.getByLabelText("Timeout"), "600");
    await user.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => {
      expect(onSubmit).toHaveBeenCalledWith({
        cron: "7 * * * *",
        command: "npm test",
        cwd: "refact-agent/gui",
        timeout_secs: 600,
        recurring: true,
        durable: false,
        description: "Command frog check",
      });
    });
  });

  it("blocks webhook delivery without a URL", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const { user } = render(
      <CronCreateForm onSubmit={onSubmit} taskCount={0} />,
    );

    await user.type(screen.getByLabelText("Description"), "Webhook frog check");
    await user.click(screen.getByRole("radio", { name: "Command action" }));
    await user.type(
      screen.getByRole("textbox", { name: "Command" }),
      "echo hi",
    );
    await user.click(screen.getByRole("radio", { name: "Webhook delivery" }));
    await user.click(screen.getByRole("button", { name: "Create" }));

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Webhook URL is required.",
    );
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("submits notifier delivery with integration id", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const { user } = render(
      <CronCreateForm onSubmit={onSubmit} taskCount={0} />,
    );

    await user.type(
      screen.getByLabelText("Description"),
      "Notifier frog check",
    );
    await user.click(screen.getByRole("radio", { name: "Command action" }));
    await user.type(
      screen.getByRole("textbox", { name: "Command" }),
      "echo hi",
    );
    await user.click(screen.getByRole("radio", { name: "Notifier delivery" }));
    await user.type(
      screen.getByLabelText("Notifier integration ID"),
      "notifier_telegram",
    );
    await user.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => {
      expect(onSubmit).toHaveBeenCalledWith({
        cron: "7 * * * *",
        command: "echo hi",
        delivery: {
          kind: "notifier",
          integration_id: "notifier_telegram",
        },
        recurring: true,
        durable: false,
        description: "Notifier frog check",
      });
    });
  });

  it("blocks non-chat delivery for agent actions", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const { user } = render(
      <CronCreateForm onSubmit={onSubmit} taskCount={0} />,
    );

    await user.type(screen.getByLabelText("Description"), "Webhook frog check");
    await user.type(screen.getByLabelText("Prompt"), "Check frogs");
    await user.click(screen.getByRole("radio", { name: "Webhook delivery" }));
    await user.type(
      screen.getByLabelText("Webhook URL"),
      "https://example.com/hook",
    );
    await user.click(screen.getByRole("button", { name: "Create" }));

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Webhook and notifier delivery require a command action.",
    );
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("emits isolated toggle for agent actions", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const { user } = render(
      <CronCreateForm onSubmit={onSubmit} taskCount={0} />,
    );

    await user.type(
      screen.getByLabelText("Description"),
      "Isolated frog check",
    );
    await user.type(screen.getByLabelText("Prompt"), "Check frogs alone");
    await user.click(screen.getByRole("switch", { name: "Isolated session" }));
    await user.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => {
      expect(onSubmit).toHaveBeenCalledWith({
        cron: "7 * * * *",
        prompt: "Check frogs alone",
        isolated: true,
        recurring: true,
        durable: false,
        description: "Isolated frog check",
      });
    });
  });

  it("does not submit edited values on blur before submit", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const { user } = render(
      <CronCreateForm onSubmit={onSubmit} taskCount={0} />,
    );

    await user.type(screen.getByLabelText("Description"), "Hourly frog check");
    await user.tab();
    await user.type(screen.getByLabelText("Prompt"), "Check frogs");
    await user.tab();

    expect(onSubmit).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => {
      expect(onSubmit).toHaveBeenCalledWith({
        cron: "7 * * * *",
        prompt: "Check frogs",
        recurring: true,
        durable: false,
        description: "Hourly frog check",
      });
    });
  });

  it("surfaces backend validation errors", () => {
    render(
      <CronCreateForm
        onSubmit={vi.fn()}
        taskCount={0}
        error={{ data: { detail: "Invalid cron expression: bad goblin" } }}
      />,
    );

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Invalid cron expression: bad goblin",
    );
  });
});
