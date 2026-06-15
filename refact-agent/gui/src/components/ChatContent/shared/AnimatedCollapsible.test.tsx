import type React from "react";
import { Theme } from "@radix-ui/themes";
import { fireEvent, render, screen } from "@testing-library/react";
import { act } from "react-dom/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";

import { COLLAPSE_ANIMATION_MS } from "../../shared/useDelayedUnmount";
import { AnimatedCollapsible } from "./AnimatedCollapsible";

const originalMatchMedia = window.matchMedia;

function mockReducedMotion(enabled: boolean) {
  const matchMedia = vi.fn((mediaQuery: string): MediaQueryList => {
    return {
      media: mediaQuery,
      matches: enabled && mediaQuery === "(prefers-reduced-motion: reduce)",
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    };
  });

  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: matchMedia,
  });
}

function renderCollapsible(ui: React.ReactElement) {
  return render(<Theme>{ui}</Theme>);
}

describe("AnimatedCollapsible", () => {
  afterEach(() => {
    vi.useRealTimers();
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: originalMatchMedia,
    });
  });

  it("toggles aria-expanded and data-open in uncontrolled mode", () => {
    const { container } = renderCollapsible(
      <AnimatedCollapsible title="Details" defaultOpen={false}>
        <div>Hidden body</div>
      </AnimatedCollapsible>,
    );
    const root = container.querySelector("section");
    const trigger = screen.getByRole("button", { name: /details/i });

    expect(root).toHaveAttribute("data-open", "false");
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("Hidden body")).toBeNull();

    fireEvent.click(trigger);

    expect(root).toHaveAttribute("data-open", "true");
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("Hidden body")).toBeInTheDocument();
  });

  it("keeps content mounted while closing and unmounts after the shared delay", () => {
    vi.useFakeTimers();
    renderCollapsible(
      <AnimatedCollapsible title="Details" defaultOpen>
        <div>Hidden body</div>
      </AnimatedCollapsible>,
    );
    const trigger = screen.getByRole("button", { name: /details/i });

    fireEvent.click(trigger);

    expect(trigger).toHaveAttribute("aria-expanded", "false");
    expect(screen.getByText("Hidden body")).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(COLLAPSE_ANIMATION_MS - 1);
    });

    expect(screen.getByText("Hidden body")).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(1);
    });

    expect(screen.queryByText("Hidden body")).toBeNull();
  });

  it("supports controlled open state and onOpenChange", () => {
    const onOpenChange = vi.fn();
    const { container } = renderCollapsible(
      <AnimatedCollapsible
        open={false}
        onOpenChange={onOpenChange}
        title="Details"
      >
        <div>Hidden body</div>
      </AnimatedCollapsible>,
    );
    const trigger = screen.getByRole("button", { name: /details/i });

    fireEvent.click(trigger);

    expect(onOpenChange).toHaveBeenCalledWith(true);
    expect(container.querySelector("section")).toHaveAttribute(
      "data-open",
      "false",
    );
  });

  it("renders a render-prop header with status", () => {
    renderCollapsible(
      <AnimatedCollapsible
        defaultOpen
        header={({ open, status }) =>
          `${open ? "Open" : "Closed"} ${status} details`
        }
        status="success"
      >
        <div>Hidden body</div>
      </AnimatedCollapsible>,
    );

    expect(
      screen.getByRole("button", { name: /open success details/i }),
    ).toHaveAttribute("aria-expanded", "true");
  });

  it("keeps animate=false closing path instant", () => {
    renderCollapsible(
      <AnimatedCollapsible animate={false} defaultOpen title="Details">
        <div>Hidden body</div>
      </AnimatedCollapsible>,
    );

    fireEvent.click(screen.getByRole("button", { name: /details/i }));

    expect(screen.queryByText("Hidden body")).toBeNull();
  });

  it("keeps reduced-motion closing path instant", () => {
    mockReducedMotion(true);
    renderCollapsible(
      <AnimatedCollapsible defaultOpen title="Details">
        <div>Hidden body</div>
      </AnimatedCollapsible>,
    );

    fireEvent.click(screen.getByRole("button", { name: /details/i }));

    expect(screen.queryByText("Hidden body")).toBeNull();
  });
});
