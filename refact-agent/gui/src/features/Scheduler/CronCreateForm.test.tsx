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
