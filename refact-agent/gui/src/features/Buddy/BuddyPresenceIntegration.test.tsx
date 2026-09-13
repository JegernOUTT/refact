import { useRef } from "react";
import { act, fireEvent, render, screen } from "../../utils/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BuddyCanvas } from "./BuddyCanvas";
import { BuddyDreamCanvas } from "./BuddyDreamCanvas";
import { BuddyPresencePreviewProvider } from "./BuddyPresencePreviewProvider";
import { BuddyWorld } from "./BuddyWorld";
import {
  BuddyPresencePreviewContext,
  buddyPresenceMotion,
  useBuddyPresencePreferences,
  type BuddyWorkState,
} from "./buddyPresenceMotion";
import { useBuddyPlaySession } from "./hooks/useBuddyPlaySession";
import { createBuddySpeechMemory } from "./buddySpeech";
import { createInitialSemanticState } from "./state";
import { PALETTES, STAGES } from "./constants";

function hidden(value: boolean) {
  Object.defineProperty(document, "hidden", { configurable: true, value });
  act(() => document.dispatchEvent(new Event("visibilitychange")));
}
function HookHarness() {
  const memory = useRef(createBuddySpeechMemory());
  const play = useBuddyPlaySession({
    name: "Pixel",
    pet: undefined,
    busy: false,
    offerBusy: true,
    buddyX: 50,
    directorIntentKind: null,
    directorIntentStartedAtMs: 0,
    speechMemory: memory.current,
  });
  return (
    <>
      <button onClick={play.startFetch}>Fetch</button>
      <button onClick={() => play.handleSceneClick(36, 78)}>Aim</button>
      <button onClick={play.resumePlay}>Resume</button>
      <output>
        {play.session?.phase ?? play.suspension?.reason ?? "none"}
      </output>
    </>
  );
}
const value = (enabled = true, workState: BuddyWorkState = "idle") => ({
  enabled,
  workState,
  lowPower: false,
  textOnly: false,
  reducedMotion: undefined,
});

function PresenceLabel() {
  const presence = useBuddyPresencePreferences();
  return <output>{buddyPresenceMotion(presence.workState).label}</output>;
}

describe("mounted presence integrations", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(100000);
    hidden(false);
  });
  afterEach(() => {
    hidden(false);
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });
  it("does not describe unavailable provider facts as idle", () => {
    render(
      <BuddyPresencePreviewProvider>
        <PresenceLabel />
      </BuddyPresencePreviewProvider>,
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "Current work presence unavailable; no execution is inferred.",
    );
    expect(screen.queryByText("Idle and available.")).not.toBeInTheDocument();
  });

  it("cancels dream frames on hide, surface collapse and unmount without restarting stale callbacks", () => {
    let id = 0;
    const frames = new Map<number, FrameRequestCallback>();
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      frames.set(++id, callback);
      return id;
    });
    vi.stubGlobal("cancelAnimationFrame", (handle: number) =>
      frames.delete(handle),
    );
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(
      {} as CanvasRenderingContext2D,
    );
    const view = render(<BuddyDreamCanvas kind="sky_flight" />);
    expect(frames.size).toBe(1);
    const staleFrame = [...frames.values()][0];
    hidden(true);
    expect(frames.size).toBe(0);
    staleFrame(100);
    expect(frames.size).toBe(0);
    hidden(false);
    expect(frames.size).toBe(1);
    view.rerender(<BuddyDreamCanvas kind="sky_flight" visible={false} />);
    expect(frames.size).toBe(0);
    view.rerender(<BuddyDreamCanvas kind="sky_flight" />);
    expect(frames.size).toBe(1);
    const unmountedFrame = [...frames.values()][0];
    view.unmount();
    expect(frames.size).toBe(0);
    unmountedFrame(200);
    expect(frames.size).toBe(0);
    hidden(true);
    const hiddenView = render(<BuddyDreamCanvas kind="sky_flight" />);
    expect(frames.size).toBe(0);
    hiddenView.unmount();
  });
  it("freezes the actual live play hook clock on hide and work interruption", () => {
    const view = render(
      <BuddyPresencePreviewContext.Provider value={value()}>
        <HookHarness />
      </BuddyPresencePreviewContext.Provider>,
    );
    fireEvent.click(screen.getByText("Fetch"));
    fireEvent.click(screen.getByText("Aim"));
    act(() => vi.advanceTimersByTime(200));
    hidden(true);
    expect(screen.getByRole("status")).toHaveTextContent("hidden");
    act(() => vi.advanceTimersByTime(60000));
    hidden(false);
    fireEvent.click(screen.getByText("Resume"));
    act(() => vi.advanceTimersByTime(749));
    expect(screen.getByRole("status")).toHaveTextContent("throwing");
    act(() => vi.advanceTimersByTime(1));
    expect(screen.getByRole("status")).toHaveTextContent("chasing");
    view.rerender(
      <BuddyPresencePreviewContext.Provider
        value={value(true, "awaiting_decision")}
      >
        <HookHarness />
      </BuddyPresencePreviewContext.Provider>,
    );
    expect(screen.getByRole("status")).toHaveTextContent("work_took_over");
    fireEvent.click(screen.getByText("Resume"));
    expect(screen.getByRole("status")).toHaveTextContent("work_took_over");
    view.unmount();
    expect(vi.getTimerCount()).toBe(0);
  });
  it("preserves default-off live play behavior", () => {
    render(<HookHarness />);
    fireEvent.click(screen.getByText("Fetch"));
    fireEvent.click(screen.getByText("Aim"));
    hidden(true);
    act(() => vi.advanceTimersByTime(950));
    expect(screen.getByRole("status")).toHaveTextContent("chasing");
  });
  it.each(["avatar", "world"])(
    "owns zero queued animation frames when %s is hidden or unmounted",
    (kind) => {
      let id = 0;
      const frames = new Map<number, FrameRequestCallback>();
      vi.stubGlobal(
        "requestAnimationFrame",
        (callback: FrameRequestCallback) => {
          frames.set(++id, callback);
          return id;
        },
      );
      vi.stubGlobal("cancelAnimationFrame", (handle: number) =>
        frames.delete(handle),
      );
      vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
      const state = createInitialSemanticState();
      const noop = () => undefined;
      const view = render(
        kind === "avatar" ? (
          <BuddyCanvas state={state} />
        ) : (
          <BuddyWorld
            state={state}
            stage={STAGES[0]}
            palette={PALETTES[0]}
            pulse={null}
            pet={undefined}
            nowPlaying={null}
            activeQuest={null}
            activeSpeech={null}
            setupNeeded={false}
            onCanvasEvent={noop}
            onCare={noop}
            onOpenPage={noop}
            onRunMode={noop}
            onDismissSetup={noop}
            onSpeechControl={noop}
          />
        ),
      );
      expect(frames.size).toBeGreaterThan(0);
      hidden(true);
      expect(frames.size).toBe(0);
      hidden(false);
      expect(frames.size).toBeGreaterThan(0);
      view.unmount();
      expect(frames.size).toBe(0);
    },
  );
});
