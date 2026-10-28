import { fireEvent, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import { render } from "../../../utils/test-utils";
import { VirtualizedChatList } from "../VirtualizedChatList";
import { TranscriptItem } from "./TranscriptItem";
import {
  RESTORE_FRAME_COUNT,
  RESTORE_MAX_MS,
  captureScrollAnchor,
  scheduleScrollAnchorRestore,
} from "../useChatScrollAnchor";

type VirtuosoCall = { totalListHeightChanged?: (height: number) => void };

function virtuosoCalls(): VirtuosoCall[] {
  return (
    ((globalThis as Record<string, unknown>).__VIRTUOSO_CALLS__ as
      | VirtuosoCall[]
      | undefined) ?? []
  );
}

function rect(top: number, height = 32): DOMRect {
  return {
    x: 0,
    y: top,
    width: 320,
    height,
    top,
    right: 320,
    bottom: top + height,
    left: 0,
    toJSON: () => ({}),
  };
}

function makeScroller(scrollTop: number) {
  const scroller = document.createElement("div");
  Object.defineProperties(scroller, {
    scrollTop: { configurable: true, value: scrollTop, writable: true },
    clientHeight: { configurable: true, value: 300 },
    scrollHeight: { configurable: true, value: 900 },
  });
  scroller.getBoundingClientRect = () => rect(0, 300);
  document.body.append(scroller);
  return scroller;
}

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

describe("transcript anchoring", () => {
  test("anchors to the trigger element even near the bottom", () => {
    const scroller = makeScroller(600);
    const item = document.createElement("div");
    item.dataset.chatScrollAnchorItem = "true";
    item.dataset.chatScrollAnchorKey = "k";
    const trigger = document.createElement("button");
    item.append(trigger);
    scroller.append(item);
    item.getBoundingClientRect = () => rect(200, 80);
    trigger.getBoundingClientRect = () => rect(240);

    expect(captureScrollAnchor(scroller)).toBeNull();
    const snapshot = captureScrollAnchor(scroller, trigger);
    expect(snapshot?.element).toBe(trigger);
    expect(snapshot?.topOffset).toBe(240);
    expect(snapshot?.fallback).toEqual({ key: "k", topOffset: 200 });
  });

  test("restores until transitionend after the frame window", () => {
    vi.useFakeTimers({ toFake: ["requestAnimationFrame", "setTimeout"] });
    const scroller = makeScroller(600);
    const trigger = document.createElement("button");
    const settle = document.createElement("div");
    scroller.append(trigger, settle);
    let top = 240;
    trigger.getBoundingClientRect = () => rect(top);
    const snapshot = captureScrollAnchor(scroller, trigger);
    if (!snapshot) throw new Error("expected snapshot");
    const onDone = vi.fn();
    scheduleScrollAnchorRestore(scroller, snapshot, undefined, {
      settleElement: () => settle,
      onDone,
    });

    for (let i = 0; i < RESTORE_FRAME_COUNT + 5; i += 1) {
      vi.advanceTimersToNextFrame();
    }
    expect(onDone).not.toHaveBeenCalled();
    top = 140;
    vi.advanceTimersToNextFrame();
    expect(scroller.scrollTop).toBe(500);
    top = 240;
    settle.dispatchEvent(new Event("transitionend", { bubbles: true }));
    expect(onDone).toHaveBeenCalledTimes(1);
  });

  test("ends the restore window by timeout when frames are throttled", () => {
    vi.useFakeTimers({ toFake: ["requestAnimationFrame", "setTimeout"] });
    const scroller = makeScroller(600);
    const trigger = document.createElement("button");
    scroller.append(trigger);
    trigger.getBoundingClientRect = () => rect(240);
    const snapshot = captureScrollAnchor(scroller, trigger);
    if (!snapshot) throw new Error("expected snapshot");
    const onDone = vi.fn();
    scheduleScrollAnchorRestore(scroller, snapshot, undefined, {
      settleElement: () => null,
      onDone,
    });
    vi.advanceTimersByTime(RESTORE_MAX_MS);
    expect(onDone).toHaveBeenCalledTimes(1);
  });

  test("suspends pin-to-bottom while a user toggle is in progress", () => {
    (globalThis as Record<string, unknown>).__VIRTUOSO_CALLS__ = [];
    vi.spyOn(window, "requestAnimationFrame").mockImplementation(() => 1);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(
      rect(0, 768),
    );
    render(
      <div>
        <VirtualizedChatList
          isStreaming
          items={[{ key: "a" }]}
          renderItem={() => <TranscriptItem output="out" verb="Read" />}
        />
      </div>,
    );
    const scroller = screen.getByTestId("chat-virtuoso-scroller");
    Object.defineProperty(scroller, "scrollHeight", {
      configurable: true,
      value: 2000,
    });
    Object.defineProperty(scroller, "scrollTop", {
      configurable: true,
      value: 500,
      writable: true,
    });
    fireEvent.click(screen.getByTestId("transcript-row"));
    virtuosoCalls().at(-1)?.totalListHeightChanged?.(2000);
    expect(scroller.scrollTop).not.toBe(2000);
  });
});
