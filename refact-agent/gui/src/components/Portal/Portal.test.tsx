import { describe, expect, test, afterEach } from "vitest";
import { cleanup, render } from "@testing-library/react";
import { Portal } from "./Portal";
import { ThemePropsContext } from "../Theme/ThemePropsContext";

/**
 * Portal theme propagation (audit L-22 / S3-23).
 *
 * Portal must render store-free (no Redux) and inherit the resolved theme
 * through ThemePropsContext so portaled overlays keep host + appearance.
 */

afterEach(cleanup);

describe("Portal", () => {
  test("inherits host and appearance from ThemePropsContext", () => {
    render(
      <ThemePropsContext.Provider
        value={{
          host: "jetbrains",
          themeProps: { accentColor: "indigo" },
          appearance: "light",
        }}
      >
        <Portal>
          <span data-testid="portaled">content</span>
        </Portal>
      </ThemePropsContext.Provider>,
    );

    const portaled = document.body.querySelector('[data-testid="portaled"]');
    expect(portaled).not.toBeNull();
    const themeRoot = portaled?.closest(".radix-themes");
    expect(themeRoot).not.toBeNull();
    expect(themeRoot?.getAttribute("data-appearance")).toBe("light");
    expect(themeRoot?.getAttribute("data-host")).toBe("jetbrains");
  });

  test("renders without any provider (store-free, S3-23 guard)", () => {
    render(
      <Portal>
        <span data-testid="portaled-bare">bare</span>
      </Portal>,
    );

    const portaled = document.body.querySelector(
      '[data-testid="portaled-bare"]',
    );
    expect(portaled).not.toBeNull();
    expect(portaled?.closest(".radix-themes")).not.toBeNull();
  });
});
