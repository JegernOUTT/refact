import { describe, it, expect } from "vitest";
import { render, screen } from "../../utils/test-utils";
import { TrajectoryButton } from "./TrajectoryButton";

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
});
