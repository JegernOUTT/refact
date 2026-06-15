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

  it("forwards refs to the Radix theme root", () => {
    const store = setUpStore();
    const ref = { current: null as HTMLDivElement | null };

    render(
      <Provider store={store}>
        <Theme ref={ref} data-testid="theme-root">
          <div>theme child</div>
        </Theme>
      </Provider>,
    );

    expect(ref.current).toBe(screen.getByTestId("theme-root"));
  });
});
