import { Theme } from "@radix-ui/themes";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Terminal } from "lucide-react";
import { afterEach, describe, expect, it, vi } from "vitest";

import styles from "./ToolCard.module.css";
import { ToolCard } from "./ToolCard";

const originalMatchMedia = window.matchMedia;

function renderToolCard() {
  const user = userEvent.setup();
  const result = render(
    <Theme>
      <ToolCard defaultOpen={false} icon={Terminal} title="Run command">
        <div>Command output</div>
        <button type="button">Copy command output</button>
      </ToolCard>
    </Theme>,
  );

  return { user, ...result };
}

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

describe("ToolCard", () => {
  afterEach(() => {
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: originalMatchMedia,
    });
  });

  it("updates open state and chevron synchronously on click", async () => {
    const { container, user } = renderToolCard();
    const card = container.querySelector("section");
    const toggle = screen.getByRole("button", { name: /run command/i });
    const chevron = container.querySelector("svg:last-child");

    expect(card).toHaveAttribute("data-open", "false");
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(chevron).toBeInTheDocument();

    await user.click(toggle);

    expect(card).toHaveAttribute("data-open", "true");
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(chevron).toHaveClass(styles.chevron);
  });

  it("toggles from the keyboard", async () => {
    const { user } = renderToolCard();
    const toggle = screen.getByRole("button", { name: /run command/i });

    toggle.focus();
    await user.keyboard("{Enter}");

    expect(toggle).toHaveAttribute("aria-expanded", "true");

    await user.keyboard(" ");

    expect(toggle).toHaveAttribute("aria-expanded", "false");
  });

  it("updates collapsed body state when opened", async () => {
    const { user } = renderToolCard();
    const toggle = screen.getByRole("button", { name: /run command/i });
    const bodyId = toggle.getAttribute("aria-controls");
    const body = bodyId ? document.getElementById(bodyId) : null;

    expect(body).toHaveAttribute("data-open", "false");
    expect(toggle).toHaveAttribute("aria-expanded", "false");

    await user.click(toggle);

    expect(body).toHaveAttribute("data-open", "true");
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("Command output")).toBeInTheDocument();
  });

  it("keeps collapsed body controls out of sequential keyboard focus", async () => {
    const { user } = renderToolCard();
    const toggle = screen.getByRole("button", { name: /run command/i });
    const bodyId = toggle.getAttribute("aria-controls");
    const body = bodyId ? document.getElementById(bodyId) : null;

    expect(body).toHaveAttribute("inert");

    await user.tab();
    expect(toggle).toHaveFocus();

    await user.tab();
    expect(
      screen.getByRole("button", { name: /copy command output/i }),
    ).not.toHaveFocus();

    await user.click(toggle);
    expect(body).not.toHaveAttribute("inert");

    await user.tab();
    expect(
      screen.getByRole("button", { name: /copy command output/i }),
    ).toHaveFocus();
  });

  it("keeps reduced-motion open feedback instant", async () => {
    mockReducedMotion(true);
    const { container, user } = renderToolCard();

    await user.click(screen.getByRole("button", { name: /run command/i }));

    expect(container.querySelector("section")).toHaveAttribute(
      "data-open",
      "true",
    );
    expect(screen.getByText("Command output")).toBeInTheDocument();
  });
});
