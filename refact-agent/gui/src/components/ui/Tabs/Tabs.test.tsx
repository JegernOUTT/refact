import { readFile } from "node:fs/promises";
import path from "node:path";

import { screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

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

  it("falls back to count and index maths when the trigger cannot be measured", () => {
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

    expect(scrolled).toHaveLength(0);

    await user.click(screen.getByRole("tab", { name: "Two" }));

    await waitFor(() => {
      expect(scrolled.at(-1)).toBe(screen.getByRole("tab", { name: "Two" }));
    });

    scrollIntoView.mockRestore();
  });

  it("reveals an off-screen default trigger by scrolling only the list", () => {
    const scrolled: Element[] = [];
    const scrollIntoView = vi
      .spyOn(Element.prototype, "scrollIntoView")
      .mockImplementation(function (this: Element) {
        scrolled.push(this);
      });
    mockTriggerMetrics(
      new Map([
        ["One", { left: 0, width: 100 }],
        ["Two", { left: 140, width: 100 }],
      ]),
    );
    let scrollLeft = 0;
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(
      function (this: HTMLElement) {
        return this.getAttribute("role") === "tablist" ? 200 : 0;
      },
    );
    vi.spyOn(HTMLElement.prototype, "scrollLeft", "get").mockImplementation(
      function (this: HTMLElement) {
        return this.getAttribute("role") === "tablist" ? scrollLeft : 0;
      },
    );
    vi.spyOn(HTMLElement.prototype, "scrollLeft", "set").mockImplementation(
      function (this: HTMLElement, value: number) {
        if (this.getAttribute("role") === "tablist") scrollLeft = value;
      },
    );
    render(
      <Tabs defaultValue="two">
        <Tabs.List activeIndex={1} itemCount={2}>
          <Tabs.Trigger value="one">One</Tabs.Trigger>
          <Tabs.Trigger value="two">Two</Tabs.Trigger>
        </Tabs.List>
      </Tabs>,
    );

    const list = screen.getByRole("tablist");
    const active = screen.getByRole("tab", { name: "Two" });
    expect(list.scrollLeft).toBe(40);
    expect(active.offsetLeft).toBeGreaterThanOrEqual(list.scrollLeft);
    expect(active.offsetLeft + active.offsetWidth).toBeLessThanOrEqual(
      list.scrollLeft + list.clientWidth,
    );
    expect(scrolled).toHaveLength(0);

    scrollIntoView.mockRestore();
  });

  it("reveals app-activated triggers inside the list when focus is outside", async () => {
    const scrolled: Element[] = [];
    const scrollIntoView = vi
      .spyOn(Element.prototype, "scrollIntoView")
      .mockImplementation(function (this: Element) {
        scrolled.push(this);
      });
    mockTriggerMetrics(
      new Map([
        ["One", { left: 0, width: 100 }],
        ["Two", { left: 140, width: 100 }],
      ]),
    );
    let scrollLeft = 0;
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(
      function (this: HTMLElement) {
        return this.getAttribute("role") === "tablist" ? 200 : 0;
      },
    );
    vi.spyOn(HTMLElement.prototype, "scrollLeft", "get").mockImplementation(
      function (this: HTMLElement) {
        return this.getAttribute("role") === "tablist" ? scrollLeft : 0;
      },
    );
    vi.spyOn(HTMLElement.prototype, "scrollLeft", "set").mockImplementation(
      function (this: HTMLElement, value: number) {
        if (this.getAttribute("role") === "tablist") scrollLeft = value;
      },
    );

    const { rerender } = render(
      <Tabs value="one" onValueChange={vi.fn()}>
        <Tabs.List activeIndex={0} itemCount={2}>
          <Tabs.Trigger value="one">One</Tabs.Trigger>
          <Tabs.Trigger value="two">Two</Tabs.Trigger>
        </Tabs.List>
      </Tabs>,
    );

    rerender(
      <Tabs value="two" onValueChange={vi.fn()}>
        <Tabs.List activeIndex={1} itemCount={2}>
          <Tabs.Trigger value="one">One</Tabs.Trigger>
          <Tabs.Trigger value="two">Two</Tabs.Trigger>
        </Tabs.List>
      </Tabs>,
    );

    const list = screen.getByRole("tablist");
    const active = screen.getByRole("tab", { name: "Two" });
    await waitFor(() => expect(list.scrollLeft).toBe(40));
    expect(scrolled).toHaveLength(0);
    expect(active.offsetLeft).toBeGreaterThanOrEqual(list.scrollLeft);
    expect(active.offsetLeft + active.offsetWidth).toBeLessThanOrEqual(
      list.scrollLeft + list.clientWidth,
    );

    scrollIntoView.mockRestore();
  });

  it("scrolls app-activated triggers into view when focus is inside the list", async () => {
    const scrolled: Element[] = [];
    const scrollIntoView = vi
      .spyOn(Element.prototype, "scrollIntoView")
      .mockImplementation(function (this: Element) {
        scrolled.push(this);
      });

    const { rerender } = render(
      <Tabs value="one" onValueChange={vi.fn()}>
        <Tabs.List activeIndex={0} itemCount={2}>
          <Tabs.Trigger value="one">One</Tabs.Trigger>
          <Tabs.Trigger value="two">Two</Tabs.Trigger>
        </Tabs.List>
      </Tabs>,
    );
    const one = screen.getByRole("tab", { name: "One" });
    one.focus();

    rerender(
      <Tabs value="two" onValueChange={vi.fn()}>
        <Tabs.List activeIndex={1} itemCount={2}>
          <Tabs.Trigger value="one">One</Tabs.Trigger>
          <Tabs.Trigger value="two">Two</Tabs.Trigger>
        </Tabs.List>
      </Tabs>,
    );

    await waitFor(() => {
      expect(scrolled.at(-1)).toBe(screen.getByRole("tab", { name: "Two" }));
    });

    scrollIntoView.mockRestore();
  });

  it("keeps scrollLeft at zero when the default trigger fits in the list", () => {
    const scrolled: Element[] = [];
    const scrollIntoView = vi
      .spyOn(Element.prototype, "scrollIntoView")
      .mockImplementation(function (this: Element) {
        scrolled.push(this);
      });
    render(
      <Tabs defaultValue="one">
        <Tabs.List
          ref={(list) => {
            if (!list) return;
            Object.defineProperties(list, {
              clientWidth: { configurable: true, value: 200 },
              scrollWidth: { configurable: true, value: 200 },
              scrollLeft: { configurable: true, writable: true, value: 0 },
            });
          }}
          activeIndex={0}
          itemCount={2}
        >
          <Tabs.Trigger
            ref={(trigger) => {
              if (!trigger) return;
              Object.defineProperties(trigger, {
                offsetLeft: { configurable: true, value: 0 },
                offsetWidth: { configurable: true, value: 80 },
              });
            }}
            value="one"
          >
            One
          </Tabs.Trigger>
          <Tabs.Trigger value="two">Two</Tabs.Trigger>
        </Tabs.List>
      </Tabs>,
    );

    expect(screen.getByRole("tablist").scrollLeft).toBe(0);
    expect(scrolled).toHaveLength(0);

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
