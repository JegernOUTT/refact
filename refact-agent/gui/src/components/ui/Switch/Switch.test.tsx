import { describe, expect, it } from "vitest";
import { screen } from "@testing-library/react";

import { render } from "../../../utils/test-utils";
import { Switch } from "./Switch";

describe("Switch", () => {
  it("renders a labelled compact switch", () => {
    render(<Switch label="Enable tools" defaultChecked />);

    const control = screen.getByRole("switch", { name: "Enable tools" });

    expect(control).toHaveAttribute("data-state", "checked");
    expect(control).toHaveAttribute("aria-checked", "true");
  });

  it("supports disabled state", () => {
    render(<Switch label="Feature flag" disabled />);

    const control = screen.getByRole("switch", { name: "Feature flag" });

    expect(control).toBeDisabled();
    expect(control).toHaveAttribute("data-disabled", "");
  });
});
