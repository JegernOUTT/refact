import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { act } from "react-dom/test-utils";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { BuddyDisclosure } from "../BuddyDisclosure";
import { BuddySection } from "../BuddySection";
import {
  resetBuddyPersistentState,
  useBuddyPersistentState,
} from "../useBuddyPersistentState";

afterEach(() => resetBuddyPersistentState());

describe("BuddyDisclosure", () => {
  it("wires ARIA between trigger and region", () => {
    render(
      <BuddyDisclosure label="Workers" count={3} summary="2 running">
        <p>Body</p>
      </BuddyDisclosure>,
    );
    const trigger = screen.getByRole("button", { name: /Workers/ });
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    expect(trigger).toHaveTextContent("3");
    expect(trigger).toHaveTextContent("2 running");
    const regionId = trigger.getAttribute("aria-controls");
    expect(regionId).toBeTruthy();
    const region = document.getElementById(regionId ?? "");
    expect(region).toHaveAttribute("role", "region");
    expect(region).toHaveAttribute("aria-labelledby", trigger.id);
    expect(region).toHaveAttribute("hidden");
    expect(document.querySelector("details")).toBeNull();
  });

  it("toggles with click, Enter and Space", async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    render(
      <BuddyDisclosure label="Evidence" onOpenChange={onOpenChange}>
        <p>Evidence body</p>
      </BuddyDisclosure>,
    );
    const trigger = screen.getByRole("button", { name: "Evidence" });
    await user.click(trigger);
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByRole("region", { name: "Evidence" })).toBeVisible();
    trigger.focus();
    await user.keyboard("{Enter}");
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    await user.keyboard(" ");
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    expect(onOpenChange.mock.calls).toEqual([[true], [false], [true]]);
  });

  it("supports controlled mode", () => {
    function Controlled() {
      const [open, setOpen] = useState(true);
      return (
        <>
          <BuddyDisclosure label="Ctl" open={open} onOpenChange={setOpen}>
            body
          </BuddyDisclosure>
          <span data-testid="state">{String(open)}</span>
        </>
      );
    }
    render(<Controlled />);
    const trigger = screen.getByRole("button", { name: "Ctl" });
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    fireEvent.click(trigger);
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    expect(screen.getByTestId("state")).toHaveTextContent("false");
  });

  it("ignores toggles when controlled open does not change", () => {
    const onOpenChange = vi.fn();
    render(
      <BuddyDisclosure label="Fixed" open={false} onOpenChange={onOpenChange}>
        body
      </BuddyDisclosure>,
    );
    const trigger = screen.getByRole("button", { name: "Fixed" });
    fireEvent.click(trigger);
    expect(onOpenChange).toHaveBeenCalledWith(true);
    expect(trigger).toHaveAttribute("aria-expanded", "false");
  });

  it("persists open state across unmount via persistKey", () => {
    const view = render(
      <BuddyDisclosure label="History" persistKey="test:history">
        body
      </BuddyDisclosure>,
    );
    fireEvent.click(screen.getByRole("button", { name: "History" }));
    view.unmount();
    render(
      <BuddyDisclosure label="History" persistKey="test:history">
        body
      </BuddyDisclosure>,
    );
    expect(screen.getByRole("button", { name: "History" })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
  });

  it("respects defaultOpen", () => {
    render(
      <BuddyDisclosure label="Write-up" defaultOpen>
        body
      </BuddyDisclosure>,
    );
    expect(screen.getByRole("button", { name: "Write-up" })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
  });

  it("mounts lazy content on first open and keeps it mounted", () => {
    const Child = vi.fn(() => <p>Lazy child</p>);
    render(
      <BuddyDisclosure label="Lazy" lazy>
        <Child />
      </BuddyDisclosure>,
    );
    expect(Child).not.toHaveBeenCalled();
    const trigger = screen.getByRole("button", { name: "Lazy" });
    fireEvent.click(trigger);
    expect(screen.getByText("Lazy child")).toBeInTheDocument();
    fireEvent.click(trigger);
    expect(screen.getByText("Lazy child")).not.toBeVisible();
  });

  it("does not toggle when disabled", () => {
    render(
      <BuddyDisclosure label="Off" disabled>
        body
      </BuddyDisclosure>,
    );
    const trigger = screen.getByRole("button", { name: "Off" });
    expect(trigger).toBeDisabled();
    fireEvent.click(trigger);
    expect(trigger).toHaveAttribute("aria-expanded", "false");
  });
});

describe("useBuddyPersistentState", () => {
  function Counter({ id }: { id: string }) {
    const [value, setValue] = useBuddyPersistentState("test:counter", 0);
    return (
      <button type="button" onClick={() => setValue((n) => n + 1)}>
        {id}:{value}
      </button>
    );
  }

  it("shares one value between subscribers and resets", () => {
    const view = render(
      <>
        <Counter id="a" />
        <Counter id="b" />
      </>,
    );
    fireEvent.click(screen.getByRole("button", { name: "a:0" }));
    expect(screen.getByRole("button", { name: "b:1" })).toBeInTheDocument();
    view.unmount();
    render(<Counter id="c" />);
    expect(screen.getByRole("button", { name: "c:1" })).toBeInTheDocument();
    act(() => {
      resetBuddyPersistentState("test:counter");
    });
    expect(screen.getByRole("button", { name: "c:0" })).toBeInTheDocument();
  });

  it("falls back to local state without a key", () => {
    function Local() {
      const [value, setValue] = useBuddyPersistentState(undefined, "x");
      return (
        <button type="button" onClick={() => setValue("y")}>
          {value}
        </button>
      );
    }
    const view = render(<Local />);
    fireEvent.click(screen.getByRole("button", { name: "x" }));
    expect(screen.getByRole("button", { name: "y" })).toBeInTheDocument();
    view.unmount();
    render(<Local />);
    expect(screen.getByRole("button", { name: "x" })).toBeInTheDocument();
  });
});

describe("BuddySection", () => {
  it("renders a labelled section with heading, count, one action and footer", () => {
    render(
      <BuddySection
        title="Reports"
        count={4}
        description="Latest delivered work"
        action={<button type="button">View all</button>}
        footer={<button type="button">Load more</button>}
        testId="section"
      >
        <p>Body</p>
      </BuddySection>,
    );
    const section = screen.getByRole("region", { name: "Reports" });
    expect(section).toHaveAttribute("data-testid", "section");
    expect(
      screen.getByRole("heading", { level: 2, name: "Reports" }),
    ).toBeInTheDocument();
    expect(section).toHaveTextContent("4");
    expect(section).toHaveAccessibleDescription("Latest delivered work");
    expect(screen.getByRole("button", { name: "View all" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Load more" })).toBeVisible();
  });

  it("supports a nested heading level", () => {
    render(<BuddySection title="Workers" headingLevel="h3" />);
    expect(
      screen.getByRole("heading", { level: 3, name: "Workers" }),
    ).toBeInTheDocument();
  });

  it("collapses through BuddyDisclosure with persistence", () => {
    const view = render(
      <BuddySection
        title="Background work"
        collapsible={{ persistKey: "test:section" }}
        action={<button type="button">Refresh</button>}
      >
        <p>Inner</p>
      </BuddySection>,
    );
    const trigger = screen.getByRole("button", { name: "Background work" });
    expect(trigger.closest("h2")).not.toBeNull();
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    expect(screen.getByRole("button", { name: "Refresh" })).toBeVisible();
    fireEvent.click(trigger);
    expect(screen.getByText("Inner")).toBeVisible();
    view.unmount();
    render(
      <BuddySection
        title="Background work"
        collapsible={{ persistKey: "test:section" }}
      >
        <p>Inner</p>
      </BuddySection>,
    );
    expect(screen.getByText("Inner")).toBeVisible();
  });
});
