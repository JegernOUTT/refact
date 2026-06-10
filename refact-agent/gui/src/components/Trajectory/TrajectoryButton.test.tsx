import { describe, it, expect, vi } from "vitest";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { render, screen, within } from "../../utils/test-utils";
import { TrajectoryButton } from "./TrajectoryButton";

vi.mock("../Portal/Portal", () => ({
  Portal: ({ children }: { children: JSX.Element }) => children,
}));

const HANDOFF_OPTIONS = [
  "Include last user message + responses",
  "Include all opened files",
  "Include all edited files",
  "Include research, subagent & planning results",
  "Generate summary",
  "Include all user messages + responses",
];

describe("TrajectoryButton", () => {
  it("renders the trajectory button", () => {
    render(<TrajectoryButton />);
    const button = screen.getByTestId("trajectory-button");
    expect(button).toBeInTheDocument();
  });

  it("has correct aria-label", () => {
    render(<TrajectoryButton />);
    const button = screen.getByLabelText("Compress or Handoff");
    expect(button).toBeInTheDocument();
  });

  it("sizes the handoff tab strip to its labels instead of stretching a blank tail", async () => {
    const css = await readFile(
      path.resolve(__dirname, "TrajectoryPopover.module.css"),
      "utf8",
    );
    const tabStrip = css.match(/\.tabStrip \{[^}]+\}/)?.[0] ?? "";

    expect(tabStrip).toContain("width: max-content;");
    expect(tabStrip).toContain("max-width: 100%;");
    expect(tabStrip).not.toMatch(/\n\s*width: 100%;/);
  });

  it("opens the full compress and handoff popover on click", async () => {
    const { user } = render(<TrajectoryButton />);

    await user.click(screen.getByTestId("trajectory-button"));

    const popover = screen.getByRole("dialog");
    expect(
      within(popover).getByRole("tab", { name: "Compress in-place" }),
    ).toBeInTheDocument();

    const handoffTab = within(popover).getByRole("tab", { name: "Handoff" });
    expect(handoffTab).toBeInTheDocument();
    expect(
      within(popover).getByRole("checkbox", { name: "Drop all context files" }),
    ).toBeInTheDocument();
    expect(
      within(popover).getByRole("button", { name: "Preview" }),
    ).toBeInTheDocument();
    expect(
      within(popover).getByRole("button", { name: "Apply" }),
    ).toBeInTheDocument();

    await user.click(handoffTab);

    for (const option of HANDOFF_OPTIONS) {
      expect(
        within(popover).getByRole("checkbox", { name: option }),
      ).toBeInTheDocument();
    }
    expect(
      within(popover).getByRole("button", { name: "Create" }),
    ).toBeInTheDocument();
  });
});
