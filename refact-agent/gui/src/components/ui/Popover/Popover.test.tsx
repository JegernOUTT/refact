import React from "react";
import { describe, expect, it } from "vitest";

import { render, screen } from "../../../utils/test-utils";
import { Popover } from "./Popover";

const customStyle = {
  "--rf-popover-test-style": "preserved",
} as React.CSSProperties;

function PopoverBody() {
  return (
    <>
      <Popover.Trigger>Open</Popover.Trigger>
      <Popover.Content
        ref={contentRef}
        className="custom-popover-content"
        maxHeight="320px"
        maxWidth="360px"
        style={customStyle}
      >
        <span>Popover content</span>
      </Popover.Content>
    </>
  );
}

let contentRef: React.RefObject<HTMLDivElement>;

describe("Popover", () => {
  it("forwards className, ref, and style through the sheet fallback", () => {
    contentRef = React.createRef<HTMLDivElement>();

    render(
      <Popover open forceSheet>
        <PopoverBody />
      </Popover>,
    );

    const content = screen.getByRole("dialog");
    expect(content).toHaveClass("custom-popover-content");
    expect(content).toBe(contentRef.current);
    expect(content.style.getPropertyValue("--rf-popover-test-style")).toBe(
      "preserved",
    );
    expect(content.style.getPropertyValue("--rf-overlay-max-width")).toBe(
      "360px",
    );
    expect(content.style.getPropertyValue("--rf-overlay-max-height")).toBe(
      "320px",
    );
  });

  it("keeps forwarded refs on the anchored popover path", () => {
    contentRef = React.createRef<HTMLDivElement>();

    render(
      <Popover open>
        <PopoverBody />
      </Popover>,
    );

    expect(contentRef.current).toHaveClass("custom-popover-content");
    expect(contentRef.current).toContainElement(
      screen.getByText("Popover content"),
    );
  });
});
