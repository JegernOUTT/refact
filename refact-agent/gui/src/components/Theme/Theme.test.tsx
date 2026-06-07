import { describe, expect, it } from "vitest";
import { Provider } from "react-redux";
import { render, screen } from "@testing-library/react";

import { setUpStore } from "../../app/store";
import { updateConfig } from "../../features/Config/configSlice";
import { Theme } from "./Theme";

describe("Theme", () => {
  it("sets host and appearance data attributes on the Radix root", () => {
    const store = setUpStore();

    store.dispatch(
      updateConfig({
        host: "jetbrains",
        themeProps: { appearance: "light" },
      }),
    );

    render(
      <Provider store={store}>
        <Theme>
          <div>theme child</div>
        </Theme>
      </Provider>,
    );

    const themeRoot = screen.getByText("theme child").closest(".radix-themes");

    expect(themeRoot?.getAttribute("data-host")).toBe("jetbrains");
    expect(themeRoot?.getAttribute("data-appearance")).toBe("light");
  });
});
