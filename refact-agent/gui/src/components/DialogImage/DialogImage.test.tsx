import { fireEvent, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { render } from "../../utils/test-utils";
import { DialogImage } from "./DialogImage";

describe("DialogImage", () => {
  it("opens the full-image viewer and supports zoom, pan, and reset", async () => {
    const { user } = render(
      <DialogImage src="data:image/png;base64,abc" alt="Architecture" />,
    );

    await user.click(
      screen.getByRole("button", { name: "Open image: Architecture" }),
    );
    const image = screen.getByRole("img", { name: "Architecture" });
    expect(image).toHaveStyle({ transform: "translate(0px, 0px) scale(1)" });

    await user.click(screen.getByRole("button", { name: "Zoom in" }));
    expect(image).toHaveStyle({ transform: "translate(0px, 0px) scale(1.25)" });

    const viewport = screen.getByRole("application", {
      name: "Image pan and zoom",
    });
    fireEvent.mouseDown(viewport, { button: 0, clientX: 10, clientY: 20 });
    fireEvent.mouseMove(window, { clientX: 35, clientY: 50 });
    fireEvent.mouseUp(window);
    expect(image).toHaveStyle({ transform: "translate(25px, 30px) scale(1.25)" });

    await user.click(screen.getByRole("button", { name: "Reset image view" }));
    expect(image).toHaveStyle({ transform: "translate(0px, 0px) scale(1)" });
  });
});
