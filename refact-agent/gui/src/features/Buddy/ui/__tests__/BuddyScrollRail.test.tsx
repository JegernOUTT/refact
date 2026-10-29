import { describe, expect, it } from "vitest";
import { screen } from "@testing-library/react";
import { render } from "../../../../utils/test-utils";
import { BuddyScrollRail } from "../BuddyScrollRail";

function setBox(element: HTMLElement, left: number, right: number) {
  element.getBoundingClientRect = () =>
    ({ left, right, width: right - left }) as DOMRect;
}

describe("BuddyScrollRail", () => {
  it("scrolls the checked option into view and marks the hidden edges", () => {
    const { rerender } = render(
      <BuddyScrollRail watch="a">
        <div>
          <label>
            <input type="radio" readOnly />A
          </label>
          <label data-testid="last">
            <input type="radio" readOnly checked />B
          </label>
        </div>
      </BuddyScrollRail>,
    );
    const rail = screen.getByTestId("last").closest("div")?.parentElement;
    if (!rail) throw new Error("Expected the rail");
    Object.defineProperty(rail, "clientWidth", { value: 100 });
    Object.defineProperty(rail, "scrollWidth", { value: 220 });
    setBox(rail, 0, 100);
    setBox(screen.getByTestId("last"), 150, 220);

    rerender(
      <BuddyScrollRail watch="b">
        <div>
          <label>
            <input type="radio" readOnly />A
          </label>
          <label data-testid="last">
            <input type="radio" readOnly checked />B
          </label>
        </div>
      </BuddyScrollRail>,
    );

    expect(rail.scrollLeft).toBe(120);
    expect(rail).toHaveAttribute("data-overflow-start", "true");
    expect(rail).not.toHaveAttribute("data-overflow-end");
  });

  it("shows no edge cue when everything fits", () => {
    render(
      <BuddyScrollRail>
        <span>Fits</span>
      </BuddyScrollRail>,
    );
    const rail = screen.getByText("Fits").parentElement;
    if (!rail) throw new Error("Expected the rail");
    expect(rail).not.toHaveAttribute("data-overflow-start");
    expect(rail).not.toHaveAttribute("data-overflow-end");
  });
});
