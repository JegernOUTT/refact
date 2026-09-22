import { readFile } from "node:fs/promises";
import path from "node:path";

import { afterEach, describe, expect, it, vi } from "vitest";
import { screen, waitFor } from "@testing-library/react";

import { render } from "../../../utils/test-utils";
import { Tabs } from "./Tabs";

type TriggerMetrics = Map<string, { left: number; width: number }>;

let restoreMetrics: (() => void) | null = null;

function mockTriggerMetrics(metrics: TriggerMetrics) {
  const left = vi
    .spyOn(HTMLElement.prototype, "offsetLeft", "get")
    .mockImplementation(function (this: HTMLElement) {
      return metrics.get(this.textContent ?? "")?.left ?? 0;
    });
  const width = vi
    .spyOn(HTMLElement.prototype, "offsetWidth", "get")
    .mockImplementation(function (this: HTMLElement) {
      return metrics.get(this.textContent ?? "")?.width ?? 0;
    });

  restoreMetrics = () => {
    left.mockRestore();
    width.mockRestore();
  };
}

async function readTabsCss() {
  return readFile(path.resolve(__dirname, "Tabs.module.css"), "utf8");
}

describe("Tabs", () => {
  afterEach(() => {
    restoreMetrics?.();
    restoreMetrics = null;
  });

  it("preserves caller list styles while setting tab indicator variables", () => {
    render(
      <Tabs defaultValue="two">
        <Tabs.List activeIndex={1} style={{ marginTop: "12px" }}>
          <Tabs.Trigger value="one">One</Tabs.Trigger>
          <Tabs.Trigger value="two">Two</Tabs.Trigger>
        </Tabs.List>
      </Tabs>,
    );

    expect(screen.getByRole("tablist")).toHaveStyle({
      marginTop: "12px",
      "--rf-tabs-count": "2",
      "--rf-tabs-index": "1",
    });
  });

  it("does not reserve a global scrollbar gutter in tab strips", async () => {
    const css = await readTabsCss();
    const list = css.match(/\.list \{[^}]+\}/)?.[0] ?? "";
    const trigger = css.match(/\.trigger \{[^}]+\}/)?.[0] ?? "";

    expect(list).toContain("display: grid;");
    expect(list).toContain("overflow: auto hidden;");
    expect(trigger).toContain("overflow: hidden;");
  });

  it("keeps triggers at their intrinsic width so narrow strips scroll", async () => {
    const css = await readTabsCss();
    const list = css.match(/\.list \{[^}]+\}/)?.[0] ?? "";
    const trigger = css.match(/\.trigger \{[^}]+\}/)?.[0] ?? "";

    expect(list).not.toContain("minmax(0, 1fr)");
    expect(list).toContain("grid-auto-columns: minmax(max-content, 1fr);");
    expect(list).toContain("overflow: auto hidden;");
    expect(list).toContain("overscroll-behavior-x: contain;");
    expect(list).toContain("scroll-snap-type: x proximity;");
    expect(list).toContain("composes: scrollbarThin from");
    expect(trigger).toContain("scroll-snap-align: start;");
  });

  it("drives the indicator from the measured active trigger", () => {
    mockTriggerMetrics(
      new Map([
        ["One", { left: 4, width: 120 }],
        ["Two", { left: 124, width: 96 }],
      ]),
    );

    render(
      <Tabs defaultValue="two">
        <Tabs.List activeIndex={1} itemCount={2}>
          <Tabs.Trigger value="one">One</Tabs.Trigger>
          <Tabs.Trigger value="two">Two</Tabs.Trigger>
        </Tabs.List>
      </Tabs>,
    );

    const tablist = screen.getByRole("tablist");

    expect(tablist).toHaveAttribute("data-tabs-measured", "true");
    expect(tablist).toHaveStyle({
      "--rf-tabs-indicator-left": "124px",
      "--rf-tabs-indicator-width": "96px",
    });
  });

  it("falls back to count and index maths when the trigger cannot be measured", async () => {
    render(
      <Tabs defaultValue="two">
        <Tabs.List activeIndex={1} itemCount={2}>
          <Tabs.Trigger value="one">One</Tabs.Trigger>
          <Tabs.Trigger value="two">Two</Tabs.Trigger>
        </Tabs.List>
      </Tabs>,
    );

    const tablist = screen.getByRole("tablist");

    expect(tablist).not.toHaveAttribute("data-tabs-measured");
    expect(tablist).toHaveStyle({
      "--rf-tabs-count": "2",
      "--rf-tabs-index": "1",
    });

    const css = await readTabsCss();
    const measured =
      css.match(
        /\.list\[data-tabs-measured="true"\] \.indicator \{[^}]+\}/,
      )?.[0] ?? "";

    expect(measured).toContain("left: var(--rf-tabs-indicator-left);");
    expect(measured).toContain("width: var(--rf-tabs-indicator-width);");
  });

  it("scrolls the newly activated trigger into view", async () => {
    const scrolled: Element[] = [];
    const scrollIntoView = vi
      .spyOn(Element.prototype, "scrollIntoView")
      .mockImplementation(function (this: Element) {
        scrolled.push(this);
      });

    const { user } = render(
      <Tabs defaultValue="one">
        <Tabs.List activeIndex={0} itemCount={2}>
          <Tabs.Trigger value="one">One</Tabs.Trigger>
          <Tabs.Trigger value="two">Two</Tabs.Trigger>
        </Tabs.List>
      </Tabs>,
    );

    expect(scrolled.at(-1)).toBe(screen.getByRole("tab", { name: "One" }));

    await user.click(screen.getByRole("tab", { name: "Two" }));

    await waitFor(() => {
      expect(scrolled.at(-1)).toBe(screen.getByRole("tab", { name: "Two" }));
    });

    scrollIntoView.mockRestore();
  });

  it("collapses inactive (hidden) tab panels regardless of consumer display overrides", async () => {
    const css = await readTabsCss();
    const hiddenRule = css.match(/\.content\[hidden\]\s*\{[^}]*\}/)?.[0] ?? "";

    expect(hiddenRule).toContain("display: none;");
  });

  it("renders an empty tab list without an indicator", () => {
    const { container } = render(
      <Tabs defaultValue="missing">
        <Tabs.List aria-label="Empty tabs" />
      </Tabs>,
    );

    expect(screen.getByRole("tablist", { name: "Empty tabs" })).toHaveStyle({
      "--rf-tabs-count": "1",
      "--rf-tabs-index": "0",
    });
    expect(container.querySelector("span[aria-hidden='true']")).toBeNull();
  });
});
