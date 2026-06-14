import { describe, expect, it, vi } from "vitest";
import { screen, render, waitFor } from "../../utils/test-utils";
import { CronCreateForm } from "./CronCreateForm";

describe("CronCreateForm", () => {
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
