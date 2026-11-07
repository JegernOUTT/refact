import { screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { render } from "../../../utils/test-utils";
import { SettingItem } from "./SettingItem";

describe("SettingItem", () => {
  it("renders copy, control, and save status", () => {
    render(
      <SettingItem
        title="Theme"
        description="Choose a display theme."
        saveStatus="saved"
        control={<button type="button">Change theme</button>}
      />,
    );

    expect(screen.getByRole("heading", { name: "Theme" })).toBeTruthy();
    expect(screen.getByText("Choose a display theme.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Change theme" })).toBeTruthy();
    expect(screen.getByRole("status")).toHaveTextContent("Saved");
  });
});
