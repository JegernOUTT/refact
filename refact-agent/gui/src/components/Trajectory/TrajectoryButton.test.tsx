import { describe, it, expect, vi } from "vitest";
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
    const button = screen.getByLabelText("Compress or Handoff");
    expect(button).toBeInTheDocument();
  });

  it("opens the full compress and handoff popover on click", async () => {
    const { user } = render(<TrajectoryButton />);

    await user.click(screen.getByTestId("trajectory-button"));

    const popover = screen.getByRole("dialog");
    expect(
      within(popover).getByRole("radio", { name: "Compress in-place" }),
    ).toBeInTheDocument();
    expect(
      within(popover).getByRole("radio", { name: "Handoff" }),
    ).toBeInTheDocument();
    expect(
      within(popover).getByRole("checkbox", { name: "Drop all context files" }),
    ).toBeInTheDocument();
    expect(
      within(popover).getByRole("button", { name: "Preview" }),
    ).toBeInTheDocument();
    expect(
      within(popover).getByRole("button", { name: "Apply" }),
    ).toBeInTheDocument();
  });
});
