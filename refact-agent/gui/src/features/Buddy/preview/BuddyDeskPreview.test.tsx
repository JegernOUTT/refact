import { describe, expect, it } from "vitest";
import { render, screen } from "../../../utils/test-utils";
import { BuddyDeskPreview } from "./BuddyDeskPreview";

describe("BuddyDeskPreview", () => {
  it("renders the real Buddy canvas in its compact avatar", () => {
    render(<BuddyDeskPreview />);
    expect(
      screen.getByLabelText(/Pixel Buddy avatar/i).querySelector("canvas"),
    ).toBeInTheDocument();
  });

  it("opens the verified report without opening chat", async () => {
    const { user } = render(<BuddyDeskPreview />);
    await user.click(
      screen.getByRole("button", { name: /keyboard navigation report/i }),
    );
    expect(screen.getByLabelText("Readable report artifact")).toHaveTextContent(
      "Tab reaches each work item",
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("supports keyboard detail navigation and pending simulated action feedback", async () => {
    const { user } = render(<BuddyDeskPreview />);
    const check = screen.getByRole("button", {
      name: /check the compact workspace/i,
    });
    check.focus();
    await user.keyboard("{Enter}");
    const action = screen.getByRole("button", {
      name: "Simulate layout check",
    });
    await user.click(action);
    expect(screen.getByText("Simulated action pending…")).toBeInTheDocument();
    expect(action).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Back to list" }));
    expect(check).toHaveFocus();
  });

  it("runs local play without changing work or permissions", async () => {
    const { user } = render(<BuddyDeskPreview />);
    await user.click(
      screen.getByRole("button", { name: "Toss ball (simulated)" }),
    );
    expect(screen.getByText(/tossed the virtual ball/i)).toBeInTheDocument();
    expect(screen.getByText(/not changing your project/i)).toBeInTheDocument();
  });
});
