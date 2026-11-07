import { screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { render } from "../../utils/test-utils";
import { DialogImage } from "./DialogImage";

describe("DialogImage", () => {
  it("opens the shared viewer with zoom controls and resets on reopen", async () => {
    const { user } = render(
      <DialogImage src="data:image/png;base64,abc" alt="Architecture" />,
    );
    await user.click(
      screen.getByRole("button", { name: "Open image: Architecture" }),
    );
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(
      screen.getByRole("img", { name: "Architecture" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Zoom in" }));
    expect(screen.getByLabelText("Image zoom")).toHaveTextContent("125%");
    await user.click(screen.getByRole("button", { name: "Fit" }));
    expect(screen.getByLabelText("Image zoom")).toHaveTextContent("100%");
    await user.keyboard("{Escape}");
    await user.click(
      screen.getByRole("button", { name: "Open image: Architecture" }),
    );
    expect(screen.getByLabelText("Image zoom")).toHaveTextContent("100%");
  });
});
