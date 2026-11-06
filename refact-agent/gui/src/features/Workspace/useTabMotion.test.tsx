import { render, renderHook } from "@testing-library/react";
import { act } from "react-dom/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useTabMotion } from "./useTabMotion";

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("tab motion intent and layout", () => {
  it("allows a later retry and replaces the same key's exit snapshot", async () => {
    const action = vi.fn();
    function Harness() {
      const motion = useTabMotion<string>(["a"]);
      return (
        <div ref={motion.list}>
          <button
            data-pane-tab-key="a"
            onClick={() => motion.close("a", action, "snapshot")}
          >
            Close
          </button>
          <span>{motion.ghosts.length}</span>
        </div>
      );
    }
    const { getByRole, container } = render(<Harness />);
    act(() => getByRole("button").click());
    await act(async () => {
      await Promise.resolve();
    });
    act(() => getByRole("button").click());
    expect(action).toHaveBeenCalledTimes(2);
    expect(container.querySelector("span")).toHaveTextContent("1");
  });

  it("closes exactly once before navigation removes keys and unmounts", () => {
    vi.useFakeTimers();
    const closeTask = vi.fn();
    const { result, rerender, unmount } = renderHook(
      ({ keys }) => useTabMotion(keys),
      { initialProps: { keys: ["task", "buddy"] } },
    );
    act(() => {
      result.current.close("task", closeTask);
      result.current.close("task", closeTask);
    });
    expect(closeTask).toHaveBeenCalledOnce();
    rerender({ keys: ["buddy"] });
    unmount();
    act(() => {
      vi.runAllTimers();
    });
    expect(closeTask).toHaveBeenCalledOnce();
  });

  it("does not read tab geometry on a nongeometry rerender", () => {
    const offset = vi.spyOn(HTMLElement.prototype, "offsetLeft", "get");
    function Harness({ token }: { token: string }) {
      const motion = useTabMotion(["a", "b"]);
      return (
        <div ref={motion.list}>
          <div data-pane-tab-key="a">{token}</div>
          <div data-pane-tab-key="b" />
        </div>
      );
    }
    const { rerender } = render(<Harness token="one" />);
    expect(offset).toHaveBeenCalled();
    offset.mockClear();
    rerender(<Harness token="two" />);
    expect(offset).not.toHaveBeenCalled();
  });
  it("never snapshots OS-reduced close and dispatches rapid close only once", () => {
    vi.spyOn(window, "matchMedia").mockReturnValue({
      matches: true,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    } as unknown as MediaQueryList);
    const action = vi.fn();
    function Harness() {
      const motion = useTabMotion<string>(["a"]);
      return (
        <div ref={motion.list}>
          <button
            data-pane-tab-key="a"
            onClick={() => motion.close("a", action, "snapshot")}
          >
            Close
          </button>
          <span>{motion.ghosts.length}</span>
        </div>
      );
    }
    const { getByRole, container } = render(<Harness />);
    act(() => {
      getByRole("button").click();
      getByRole("button").click();
    });
    expect(action).toHaveBeenCalledOnce();
    expect(container.querySelector("span")).toHaveTextContent("0");
  });

  it("animates reordered keys and cancels inline FLIP styles on a new order", () => {
    let reversed = false;
    vi.spyOn(HTMLElement.prototype, "offsetLeft", "get").mockImplementation(
      function (this: HTMLElement) {
        return (this.dataset.paneTabKey === "a") !== reversed ? 0 : 100;
      },
    );
    const cancel = vi.spyOn(window, "cancelAnimationFrame");
    function Harness({ keys }: { keys: string[] }) {
      const motion = useTabMotion(keys);
      return (
        <div ref={motion.list}>
          {keys.map((key) => (
            <div key={key} data-pane-tab-key={key} />
          ))}
        </div>
      );
    }
    const { rerender, container, unmount } = render(
      <Harness keys={["a", "b"]} />,
    );
    reversed = true;
    rerender(<Harness keys={["b", "a"]} />);
    const node = container.querySelector<HTMLElement>(
      '[data-pane-tab-key="a"]',
    );
    if (!node) throw new Error("Missing reordered tab");
    expect(node.style.transform).toBe("translateX(-100px)");
    rerender(<Harness keys={["a"]} />);
    expect(cancel).toHaveBeenCalled();
    expect(node.style.transform).toBe("");
    expect(node.style.transition).toBe("");
    unmount();
  });
});
