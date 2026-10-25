import { Theme } from "@radix-ui/themes";
import React from "react";
import { render, screen, waitFor } from "@testing-library/react";
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

  it("restores exact tabindex values only after every collapsed owner opens", () => {
    const values = [null, "", "0", "3", "-1"];
    const view = (outerOpen: boolean, innerOpen: boolean) => (
      <React.StrictMode>
        <ToolCard title="Outer" open={outerOpen}>
          <ToolCard title="Inner" open={innerOpen}>
            {values.map((value, index) => (
              <button key={index} data-testid={`control-${index}`}>
                Control {value ?? "absent"}
              </button>
            ))}
          </ToolCard>
        </ToolCard>
        <button tabIndex={7}>Outside</button>
      </React.StrictMode>
    );
    const { rerender } = render(view(true, true));
    const controls = values.map((value, index) => {
      const control = screen.getByTestId(`control-${index}`);
      if (value !== null) control.setAttribute("tabindex", value);
      return control;
    });
    const states = [
      [false, false],
      [true, false],
      [false, false],
      [false, true],
      [true, true],
      [true, false],
      [false, false],
      [true, true],
    ];
    for (const [outerOpen, innerOpen] of states) {
      rerender(view(outerOpen, innerOpen));
      controls.forEach((control, index) => {
        expect(control.getAttribute("tabindex")).toBe(
          outerOpen && innerOpen ? values[index] : "-1",
        );
      });
      const innerToggle = screen.getByRole("button", { name: "Inner" });
      expect(innerToggle.getAttribute("tabindex")).toBe(
        outerOpen ? null : "-1",
      );
      const innerBody = document.getElementById(
        innerToggle.getAttribute("aria-controls") ?? "",
      );
      expect(innerBody?.hasAttribute("inert")).toBe(!innerOpen);
      expect(screen.getByRole("button", { name: "Outside" })).toHaveAttribute(
        "tabindex",
        "7",
      );
    }
  });

  it("keeps independent inner state and mounted input across rapid outer toggles", async () => {
    const user = userEvent.setup();
    render(
      <Theme>
        <ToolCard title="Outer" defaultOpen={false}>
          <ToolCard title="Inner" defaultOpen={false}>
            <input aria-label="Retained note" defaultValue="Original" />
          </ToolCard>
        </ToolCard>
        <button>After</button>
      </Theme>,
    );
    const outer = screen.getByRole("button", { name: "Outer" });
    const inner = screen.getByRole("button", { name: "Inner" });
    const input = screen.getByLabelText("Retained note");
    await user.click(outer);
    expect(input).toHaveAttribute("tabindex", "-1");
    inner.focus();
    await user.tab();
    expect(screen.getByRole("button", { name: "After" })).toHaveFocus();
    await user.tab({ shift: true });
    expect(inner).toHaveFocus();
    await user.keyboard("{Enter}");
    await user.type(input, " updated");
    for (let index = 0; index < 3; index++) {
      await user.click(outer);
      expect(input).toHaveAttribute("tabindex", "-1");
      await user.click(outer);
      expect(inner).toHaveAttribute("aria-expanded", "true");
      expect(input).not.toHaveAttribute("tabindex");
    }
    expect(input).toHaveValue("Original updated");
    await user.click(inner);
    await user.click(outer);
    await user.click(outer);
    expect(inner).toHaveAttribute("aria-expanded", "false");
    expect(input).toHaveAttribute("tabindex", "-1");
  });

  it("guards descendant-only dynamic children and releases removed controls", async () => {
    const user = userEvent.setup();
    let setVisible: (visible: boolean) => void = () => undefined;
    function DynamicControls() {
      const [visible, updateVisible] = React.useState(false);
      setVisible = updateVisible;
      return visible ? <button tabIndex={2}>Dynamic control</button> : null;
    }
    render(
      <>
        <button onClick={() => setVisible(true)}>Show dynamic control</button>
        <button onClick={() => setVisible(false)}>
          Remove dynamic control
        </button>
        <ToolCard title="Outer" defaultOpen={false}>
          <ToolCard title="Inner" defaultOpen={false}>
            <DynamicControls />
          </ToolCard>
        </ToolCard>
      </>,
    );
    await user.click(
      screen.getByRole("button", { name: "Show dynamic control" }),
    );
    const control = screen.getByRole("button", { name: "Dynamic control" });
    await waitFor(() => expect(control).toHaveAttribute("tabindex", "-1"));
    await user.click(
      screen.getByRole("button", { name: "Remove dynamic control" }),
    );
    await waitFor(() => expect(control).toHaveAttribute("tabindex", "2"));
    expect(control).not.toBeInTheDocument();
  });

  it("releases all nested guards on unmount without leaving modified controls", () => {
    const { unmount } = render(
      <React.StrictMode>
        <ToolCard title="Outer" defaultOpen={false}>
          <ToolCard title="Inner" defaultOpen={false}>
            <button>Implicit tabindex</button>
            <button tabIndex={0}>Explicit tabindex</button>
          </ToolCard>
        </ToolCard>
      </React.StrictMode>,
    );
    const implicit = screen.getByRole("button", { name: "Implicit tabindex" });
    const explicit = screen.getByRole("button", { name: "Explicit tabindex" });
    const bodies = document.querySelectorAll("[inert]");
    expect(implicit).toHaveAttribute("tabindex", "-1");
    expect(explicit).toHaveAttribute("tabindex", "-1");
    unmount();
    expect(implicit).not.toHaveAttribute("tabindex");
    expect(explicit).toHaveAttribute("tabindex", "0");
    for (const body of bodies) expect(body).not.toHaveAttribute("inert");
  });

  it("observes only collapsed bodies and disconnects each guard on open or unmount", () => {
    const observe = vi.spyOn(MutationObserver.prototype, "observe");
    const disconnect = vi.spyOn(MutationObserver.prototype, "disconnect");
    const view = (outerOpen: boolean, innerOpen: boolean) => (
      <ToolCard title="Outer" open={outerOpen}>
        <ToolCard title="Inner" open={innerOpen}>
          <button>Action</button>
        </ToolCard>
      </ToolCard>
    );
    try {
      const { rerender, unmount } = render(view(false, false));
      const bodies = Array.from(document.querySelectorAll("[inert]"));
      expect(observe).toHaveBeenCalledTimes(2);
      for (const body of bodies) {
        expect(observe).toHaveBeenCalledWith(body, {
          childList: true,
          subtree: true,
        });
      }
      rerender(view(true, false));
      expect(disconnect).toHaveBeenCalledTimes(1);
      expect(observe).toHaveBeenCalledTimes(2);
      rerender(view(true, true));
      expect(disconnect).toHaveBeenCalledTimes(2);
      rerender(view(false, false));
      expect(observe).toHaveBeenCalledTimes(4);
      unmount();
      expect(disconnect).toHaveBeenCalledTimes(4);
    } finally {
      observe.mockRestore();
      disconnect.mockRestore();
    }
  });
});
