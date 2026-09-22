import { describe, it, expect, vi } from "vitest";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { render, screen, within } from "../../utils/test-utils";
import { TrajectoryButton } from "./TrajectoryButton";

vi.mock("../Portal/Portal", () => ({
  Portal: ({ children }: { children: JSX.Element }) => children,
}));

describe("TrajectoryButton", () => {
  it("renders the trajectory button", () => {
    render(<TrajectoryButton />);
    const button = screen.getByTestId("trajectory-button");
    expect(button).toBeInTheDocument();
  });

  it("has correct aria-label", () => {
    render(<TrajectoryButton />);
    const button = screen.getByLabelText("Compress or rebuild context");
    expect(button).toBeInTheDocument();
  });

  it("uses the standard tab strip geometry for the handoff popover", async () => {
    const css = await readFile(
      path.resolve(__dirname, "TrajectoryPopover.module.css"),
      "utf8",
    );
    const tabStrip = css.match(/\.tabStrip \{[^}]+\}/)?.[0] ?? "";

    expect(tabStrip).toContain("max-width: 100%;");
    expect(tabStrip).not.toContain("width: max-content;");
    expect(tabStrip).not.toContain("grid-auto-columns: max-content;");
  });

  it("retains static and LLM compression tabs without Handoff with preview-gated actions", async () => {
    const { user } = render(<TrajectoryButton />);

    await user.click(screen.getByTestId("trajectory-button"));

    const popover = screen.getByRole("dialog");
    expect(
      within(popover).getByRole("tab", { name: "Compress in-place" }),
    ).toBeInTheDocument();
    expect(
      within(popover).getByRole("tab", { name: "LLM compression" }),
    ).toBeInTheDocument();

    expect(
      within(popover).queryByRole("tab", { name: "Handoff" }),
    ).not.toBeInTheDocument();
    expect(
      within(popover).getByRole("checkbox", { name: "Drop all context files" }),
    ).toBeInTheDocument();
    expect(
      within(popover).getByRole("checkbox", {
        name: "Remove usage and metering details",
      }),
    ).toBeInTheDocument();
    expect(
      within(popover).getByRole("button", { name: "Preview" }),
    ).toBeInTheDocument();
    expect(
      within(popover).getByRole("button", { name: "Apply" }),
    ).toBeInTheDocument();

    await user.click(
      within(popover).getByRole("tab", { name: "LLM compression" }),
    );
    expect(
      within(popover).getByText(/Rebuilds the active conversation context/i),
    ).toBeInTheDocument();
    expect(
      within(popover).getByRole("button", { name: "Rebuild context" }),
    ).toBeDisabled();
  });
});
