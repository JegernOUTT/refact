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

  it("submits valid form values", async () => {
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
