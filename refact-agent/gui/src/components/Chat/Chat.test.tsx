import { useRef } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "../../utils/test-utils";
import { useBottomDockClearance } from "./useBottomDockClearance";

type ResizeObserverMock = {
  callback: ResizeObserverCallback;
  disconnect: ReturnType<typeof vi.fn>;
  observe: ReturnType<typeof vi.fn>;
  unobserve: ReturnType<typeof vi.fn>;
};

function BottomDockHarness() {
  const dockRef = useRef<HTMLDivElement>(null);
  useBottomDockClearance(dockRef);
  return (
    <div data-testid="chat-root">
      <div ref={dockRef} data-testid="bottom-dock" />
    </div>
  );
}

describe("Chat bottom dock clearance", () => {
  let dockHeight = 0;
  let offsetHeightSpy: ReturnType<typeof vi.spyOn>;
  let resizeObservers: ResizeObserverMock[];

  beforeEach(() => {
    resizeObservers = [];
    offsetHeightSpy = vi
      .spyOn(HTMLElement.prototype, "offsetHeight", "get")
      .mockImplementation(function measuredOffsetHeight(this: HTMLElement) {
        return this.dataset.testid === "bottom-dock" ? dockHeight : 0;
      });
    vi.stubGlobal(
      "ResizeObserver",
      vi.fn((callback: ResizeObserverCallback) => {
        const observer: ResizeObserverMock = {
          callback,
          disconnect: vi.fn(),
          observe: vi.fn(),
          unobserve: vi.fn(),
        };
        resizeObservers.push(observer);
        return observer;
      }),
    );
  });

  afterEach(() => {
    offsetHeightSpy.mockRestore();
    vi.unstubAllGlobals();
  });

  it("measures composer-only clearance before paint", () => {
    dockHeight = 84;
    render(<BottomDockHarness />);

    expect(
      screen
        .getByTestId("chat-root")
        .style.getPropertyValue("--rf-composer-clearance"),
    ).toBe("84px");
    expect(resizeObservers).toHaveLength(1);
    expect(resizeObservers[0]?.observe).toHaveBeenCalledWith(
      screen.getByTestId("bottom-dock"),
    );
  });

  it("tracks collapsed and expanded terminal plus composer heights", () => {
    dockHeight = 108;
    const view = render(<BottomDockHarness />);
    const root = screen.getByTestId("chat-root");

    expect(root.style.getPropertyValue("--rf-composer-clearance")).toBe(
      "108px",
    );

    dockHeight = 336;
    resizeObservers[0]?.callback([], {} as ResizeObserver);
    expect(root.style.getPropertyValue("--rf-composer-clearance")).toBe(
      "336px",
    );

    dockHeight = 108;
    resizeObservers[0]?.callback([], {} as ResizeObserver);
    expect(root.style.getPropertyValue("--rf-composer-clearance")).toBe(
      "108px",
    );

    view.unmount();
    expect(resizeObservers[0]?.disconnect).toHaveBeenCalledOnce();
  });
});
