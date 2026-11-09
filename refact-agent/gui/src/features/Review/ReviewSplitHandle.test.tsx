import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ReviewSplitHandle } from "./ReviewSplitHandle";

describe("ReviewSplitHandle", () => {
  it("supports keyboard bounds and exposes separator values", () => {
    const onChange = vi.fn();
    render(
      <ReviewSplitHandle
        value={300}
        min={272}
        max={500}
        onChange={onChange}
        controls="chat"
      />,
    );
    const handle = screen.getByRole("separator", { name: "Resize chat" });
    expect(handle).toHaveAttribute("aria-controls", "chat");
    expect(handle).toHaveAttribute("aria-valuenow", "300");
    fireEvent.keyDown(handle, { key: "ArrowLeft" });
    fireEvent.keyDown(handle, { key: "ArrowRight" });
    fireEvent.keyDown(handle, { key: "Home" });
    fireEvent.keyDown(handle, { key: "End" });
    expect(onChange.mock.calls).toEqual([[290], [310], [272], [500]]);
  });
  it("captures pointer movement, clamps, and stops on cancellation", () => {
    const onChange = vi.fn();
    render(
      <ReviewSplitHandle
        value={300}
        min={272}
        max={500}
        onChange={onChange}
        controls="chat"
      />,
    );
    const handle = screen.getByRole("separator");
    handle.setPointerCapture = vi.fn();
    fireEvent.pointerDown(handle, { button: 0, pointerId: 1, clientX: 100 });
    fireEvent.pointerMove(handle, { pointerId: 1, clientX: 1000 });
    expect(onChange).toHaveBeenLastCalledWith(500);
    fireEvent.pointerCancel(handle, { pointerId: 1 });
    fireEvent.pointerMove(handle, { pointerId: 1, clientX: 0 });
    expect(onChange).toHaveBeenCalledTimes(1);
  });
});
