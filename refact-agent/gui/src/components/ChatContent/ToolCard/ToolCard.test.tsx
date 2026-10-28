import React from "react";
import { Theme } from "@radix-ui/themes";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Terminal } from "lucide-react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ToolCard as KitToolCard } from "../../ui/ToolCard";
import { ChatScrollAnchorContext } from "../useChatScrollAnchor";
import { CollapsibleStoreProvider, useStoredOpen } from "../useStoredOpen";
import { ToolCard, type ToolCardProps } from "./ToolCard";

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

function FocusableHarness({ density }: { density?: ToolCardProps["density"] }) {
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
          density={density}
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

function Harness({
  animate = true,
  density,
}: {
  animate?: boolean;
  density?: ToolCardProps["density"];
}) {
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
          density={density}
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

function StoredHarness({ storeKey }: { storeKey: string }) {
  const [isOpen, onToggle] = useStoredOpen(storeKey);
  return (
    <ToolCard
      density="compact"
      icon={<Terminal />}
      isOpen={isOpen}
      onToggle={onToggle}
      status="success"
      summary={storeKey}
    >
      <div>Stored details</div>
    </ToolCard>
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

  it("keeps default density and existing open-state defaults unchanged", () => {
    const { container } = render(
      <Theme>
        <KitToolCard title="Run command">Command output</KitToolCard>
        <Harness />
      </Theme>,
    );

    for (const card of container.querySelectorAll("section")) {
      expect(card).toHaveAttribute("data-density", "default");
    }
    expect(screen.getByRole("button", { name: "Run command" })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
    expect(screen.getByRole("button", { name: "Shell" })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
  });

  it("opts into compact density without changing the uncontrolled default", async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    const { container } = render(
      <Theme>
        <KitToolCard
          density="compact"
          onOpenChange={onOpenChange}
          title="Inspect work"
        >
          Work details
        </KitToolCard>
      </Theme>,
    );
    const card = container.querySelector("section");
    const toggle = screen.getByRole("button", { name: "Inspect work" });

    expect(card).toHaveAttribute("data-density", "compact");
    expect(toggle).toHaveAttribute("aria-expanded", "true");

    await user.click(toggle);

    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(onOpenChange).toHaveBeenCalledExactlyOnceWith(false);
  });

  it.each([
    ["running", "Waiting"],
    ["success", "Queued"],
    ["success", "Unverified"],
    ["error", "Approval needed"],
  ] as const)(
    "retains caller-supplied %s state text %s beside the compact summary",
    (status, stateLabel) => {
      const summary = "Inspect the active workspace and its outstanding work";
      const { container } = render(
        <Theme>
          <ToolCard
            density="compact"
            icon={<Terminal />}
            isOpen={false}
            meta={stateLabel}
            onToggle={vi.fn()}
            status={status}
            summary={summary}
          >
            Full result
          </ToolCard>
        </Theme>,
      );
      const toggle = screen.getByRole("button", {
        name: `${summary} ${stateLabel}`,
      });
      const state = screen.getByText(stateLabel);

      expect(container.querySelector("section")).toHaveAttribute(
        "data-density",
        "compact",
      );
      expect(toggle).toHaveAttribute("aria-expanded", "false");
      expect(state).toBeVisible();
      expect(state).toHaveAttribute("data-testid", "transcript-meta");
      expect(
        toggle.querySelector('[data-testid="transcript-title"]'),
      ).not.toContainElement(state);
      expect(screen.queryByText("Full result")).toBeNull();
    },
  );

  it("keeps compact toggles controlled and preserves scroll anchoring", async () => {
    const user = userEvent.setup();
    const onToggle = vi.fn();
    const preserveScrollAnchor = vi.fn((mutate: () => void) => mutate());
    const prepareScrollAnchor = vi.fn();
    const renderCard = (isOpen: boolean) => (
      <Theme>
        <ChatScrollAnchorContext.Provider
          value={{ preserveScrollAnchor, prepareScrollAnchor }}
        >
          <ToolCard
            density="compact"
            icon={<Terminal />}
            isOpen={isOpen}
            onToggle={onToggle}
            status="running"
            summary="Inspect work"
          >
            Full work details
          </ToolCard>
        </ChatScrollAnchorContext.Provider>
      </Theme>
    );
    const { rerender } = render(renderCard(false));
    const toggle = screen.getByRole("button", { name: "Inspect work" });
    const bodyId = toggle.getAttribute("aria-controls");

    await user.click(toggle);

    expect(prepareScrollAnchor).toHaveBeenCalled();
    expect(preserveScrollAnchor).toHaveBeenCalledOnce();
    expect(onToggle).toHaveBeenCalledOnce();
    expect(toggle).toHaveAttribute("aria-expanded", "false");

    rerender(renderCard(true));

    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(toggle).toHaveAttribute("aria-controls", bodyId);
    expect(screen.getByText("Full work details")).toBeInTheDocument();
  });

  it("keeps compact header actions outside the trigger and independently focusable", async () => {
    const user = userEvent.setup();
    const onToggle = vi.fn();
    const onAction = vi.fn();
    render(
      <Theme>
        <ToolCard
          actions={
            <button type="button" onClick={onAction}>
              Open trajectory
            </button>
          }
          density="compact"
          icon={<Terminal />}
          isOpen={false}
          meta="Queued"
          onToggle={onToggle}
          status="success"
          summary="Inspect work"
        />
      </Theme>,
    );
    const toggle = screen.getByRole("button", { name: "Inspect work Queued" });
    const action = screen.getByRole("button", { name: "Open trajectory" });

    expect(toggle).not.toContainElement(action);
    expect(screen.getByTestId("transcript-row")).toContainElement(action);

    await user.tab();
    expect(toggle).toHaveFocus();
    await user.tab();
    expect(action).toHaveFocus();
    await user.keyboard("{Enter}");

    expect(onAction).toHaveBeenCalledOnce();
    expect(onToggle).not.toHaveBeenCalled();
    expect(toggle).toHaveAttribute("aria-expanded", "false");
  });

  it.each([true, false])(
    "preserves stored compact preference %s across remounts and sibling toggles",
    async (storedOpen) => {
      const user = userEvent.setup();
      const preferences = new Map([["First work item", storedOpen]]);
      const store = {
        get: (key: string) => preferences.get(key),
        set: (key: string, open: boolean) => {
          preferences.set(key, open);
        },
      };
      const renderCards = () => (
        <Theme>
          <CollapsibleStoreProvider value={store}>
            <StoredHarness storeKey="First work item" />
            <StoredHarness storeKey="Second work item" />
          </CollapsibleStoreProvider>
        </Theme>
      );
      const { unmount } = render(renderCards());
      const first = screen.getByRole("button", { name: "First work item" });
      const second = screen.getByRole("button", { name: "Second work item" });

      expect(first).toHaveAttribute("aria-expanded", String(storedOpen));
      expect(second).toHaveAttribute("aria-expanded", "false");

      await user.click(second);
      expect(first).toHaveAttribute("aria-expanded", String(storedOpen));
      await user.click(first);
      expect(second).toHaveAttribute("aria-expanded", "true");
      unmount();
      render(renderCards());

      expect(
        screen.getByRole("button", { name: "First work item" }),
      ).toHaveAttribute("aria-expanded", String(!storedOpen));
      expect(
        screen.getByRole("button", { name: "Second work item" }),
      ).toHaveAttribute("aria-expanded", "true");
    },
  );

  it.each(["default", "compact"] as const)(
    "toggles the %s wrapper shell from the keyboard",
    async (density) => {
      const user = userEvent.setup();
      render(<Harness density={density} />);
      const toggle = screen.getByRole("button", { name: /^shell$/i });

      toggle.focus();
      await user.keyboard("{Enter}");

      expect(toggle).toHaveAttribute("aria-expanded", "true");

      await user.keyboard(" ");

      expect(toggle).toHaveAttribute("aria-expanded", "false");
    },
  );

  it.each(["default", "compact"] as const)(
    "keeps delayed %s collapsed content inert while it remains mounted",
    async (density) => {
      const user = userEvent.setup();
      render(<FocusableHarness density={density} />);
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

      await user.click(toggle);
      expect(body).not.toHaveAttribute("inert");
      await user.tab();
      expect(
        screen.getByRole("button", { name: /copy shell output/i }),
      ).toHaveFocus();
    },
  );

  it("mounts content immediately on open before delayed animation settles", () => {
    vi.useFakeTimers();
    render(<Harness />);

    fireEvent.click(screen.getByRole("button", { name: /^shell$/i }));

    expect(screen.getByText("Shell output")).toBeInTheDocument();
  });

  it.each(["default", "compact"] as const)(
    "restores %s open visual state when reopened before delayed unmount",
    (density) => {
      vi.useFakeTimers();
      const { container } = render(<Harness density={density} />);
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
    },
  );

  it.each(["default", "compact"] as const)(
    "keeps the %s no-animation path instant",
    async (density) => {
      const user = userEvent.setup();
      const { container } = render(
        <Harness animate={false} density={density} />,
      );

      await user.click(screen.getByRole("button", { name: /^shell$/i }));

      expect(container.querySelector("section")).toHaveAttribute(
        "data-open",
        "true",
      );
      expect(screen.getByText("Shell output")).toBeInTheDocument();
    },
  );

  it.each(["default", "compact"] as const)(
    "keeps the %s reduced-motion path instant",
    async (density) => {
      mockReducedMotion(true);
      const user = userEvent.setup();
      const { container } = render(<Harness density={density} />);

      await user.click(screen.getByRole("button", { name: /^shell$/i }));

      expect(container.querySelector("section")).toHaveAttribute(
        "data-open",
        "true",
      );
      expect(screen.getByText("Shell output")).toBeInTheDocument();
    },
  );
});
