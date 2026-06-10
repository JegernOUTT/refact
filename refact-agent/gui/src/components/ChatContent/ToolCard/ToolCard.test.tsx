import React from "react";
import { Theme } from "@radix-ui/themes";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Terminal } from "lucide-react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ChatScrollAnchorContext } from "../useChatScrollAnchor";
import { ToolCard } from "./ToolCard";

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

function FocusableHarness() {
  const [isOpen, setIsOpen] = React.useState(true);
  return (
    <Theme>
      <ChatScrollAnchorContext.Provider
        value={{
          preserveScrollAnchor: (mutate) => mutate(),
          prepareScrollAnchor: () => undefined,
        }}
      >
        <ToolCard
          icon={<Terminal />}
          isOpen={isOpen}
          onToggle={() => setIsOpen((value) => !value)}
          status="success"
          summary="Shell"
        >
          <button type="button">Copy shell output</button>
        </ToolCard>
      </ChatScrollAnchorContext.Provider>
    </Theme>
  );
}

function Harness({ animate = true }: { animate?: boolean }) {
  const [isOpen, setIsOpen] = React.useState(false);
  return (
    <Theme>
      <ChatScrollAnchorContext.Provider
        value={{
          preserveScrollAnchor: (mutate) => mutate(),
          prepareScrollAnchor: () => undefined,
        }}
      >
        <ToolCard
          animate={animate}
          icon={<Terminal />}
          isOpen={isOpen}
          onToggle={() => setIsOpen((value) => !value)}
          status="success"
          summary="Shell"
        >
          <div>Shell output</div>
        </ToolCard>
      </ChatScrollAnchorContext.Provider>
    </Theme>
  );
}

describe("ChatContent ToolCard", () => {
  afterEach(() => {
    vi.useRealTimers();
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: originalMatchMedia,
    });
  });

  it("uses the current open state for immediate shell feedback", async () => {
    const user = userEvent.setup();
    const { container } = render(<Harness />);
    const card = container.querySelector("section");
    const toggle = screen.getByRole("button", { name: /^shell$/i });

    expect(card).toHaveAttribute("data-open", "false");
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("Shell output")).toBeNull();

    await user.click(toggle);

    expect(card).toHaveAttribute("data-open", "true");
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("Shell output")).toBeInTheDocument();
  });

  it("toggles the wrapper shell from the keyboard", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    const toggle = screen.getByRole("button", { name: /^shell$/i });

    toggle.focus();
    await user.keyboard("{Enter}");

    expect(toggle).toHaveAttribute("aria-expanded", "true");

    await user.keyboard(" ");

    expect(toggle).toHaveAttribute("aria-expanded", "false");
  });

  it("keeps delayed collapsed content inert while it remains mounted", async () => {
    const user = userEvent.setup();
    render(<FocusableHarness />);
    const toggle = screen.getByRole("button", { name: /^shell$/i });
    const bodyId = toggle.getAttribute("aria-controls");
    const body = bodyId ? document.getElementById(bodyId) : null;

    expect(body).not.toHaveAttribute("inert");

    await user.tab();
    expect(toggle).toHaveFocus();

    await user.tab();
    expect(
      screen.getByRole("button", { name: /copy shell output/i }),
    ).toHaveFocus();

    await user.click(toggle);
    expect(body).toHaveAttribute("inert");

    toggle.focus();
    await user.tab();
    expect(
      screen.getByRole("button", { name: /copy shell output/i }),
    ).not.toHaveFocus();
  });

  it("mounts content immediately on open before delayed animation settles", () => {
    vi.useFakeTimers();
    render(<Harness />);

    fireEvent.click(screen.getByRole("button", { name: /^shell$/i }));

    expect(screen.getByText("Shell output")).toBeInTheDocument();
  });

  it("restores open visual state when reopened before delayed unmount", () => {
    vi.useFakeTimers();
    const { container } = render(<Harness />);
    const card = container.querySelector("section");
    const toggle = screen.getByRole("button", { name: /^shell$/i });

    expect(card).toHaveAttribute("data-open", "false");
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("Shell output")).toBeNull();

    fireEvent.click(toggle);

    expect(card).toHaveAttribute("data-open", "true");
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("Shell output")).toBeInTheDocument();

    fireEvent.click(toggle);

    expect(card).toHaveAttribute("data-open", "false");
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.getByText("Shell output")).toBeInTheDocument();

    fireEvent.click(toggle);

    expect(card).toHaveAttribute("data-open", "true");
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("Shell output")).toBeInTheDocument();
  });

  it("keeps reduced-motion and no-animation path instant", async () => {
    const user = userEvent.setup();
    const { container } = render(<Harness animate={false} />);

    await user.click(screen.getByRole("button", { name: /^shell$/i }));

    expect(container.querySelector("section")).toHaveAttribute(
      "data-open",
      "true",
    );
    expect(screen.getByText("Shell output")).toBeInTheDocument();
  });

  it("keeps reduced-motion path instant", async () => {
    mockReducedMotion(true);
    const user = userEvent.setup();
    const { container } = render(<Harness />);

    await user.click(screen.getByRole("button", { name: /^shell$/i }));

    expect(container.querySelector("section")).toHaveAttribute(
      "data-open",
      "true",
    );
    expect(screen.getByText("Shell output")).toBeInTheDocument();
  });
});
