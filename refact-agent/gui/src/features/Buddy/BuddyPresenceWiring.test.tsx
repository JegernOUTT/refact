import { http, HttpResponse } from "msw";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { BuddyCanvas } from "./BuddyCanvas";
import { BuddyWorld } from "./BuddyWorld";
import { BuddyPresenceProvider } from "./BuddyPresenceProvider";
import { useBuddyPresencePreferences } from "./buddyPresenceMotion";
import { createInitialSemanticState } from "./state";
import { setBuddyTextPresence } from "./useBuddyTextPresence";
import { PALETTES, STAGES } from "./constants";

const PROJECT_ROOT = "/fixture";

function installPresenceHandlers() {
  server.use(
    http.get("*/v1/buddy/native-work", () =>
      HttpResponse.json({
        schema_version: 1,
        project_root: PROJECT_ROOT,
        coverage: "loaded_native_sessions",
        next_cursor: null,
        sessions: [],
      }),
    ),
    http.get("*/v1/buddy/decisions/current", () =>
      HttpResponse.json({
        schema_version: 1,
        project_root: PROJECT_ROOT,
        coverage: "loaded_native_sessions",
        truncated: false,
        questions: [],
      }),
    ),
  );
}

function setHidden(value: boolean) {
  Object.defineProperty(document, "hidden", { configurable: true, value });
  act(() => document.dispatchEvent(new Event("visibilitychange")));
}

function stubReducedMotion(reduced: boolean) {
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: reduced && query.includes("prefers-reduced-motion"),
    media: query,
    onchange: null,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
    addListener: () => undefined,
    removeListener: () => undefined,
    dispatchEvent: () => false,
  }));
}

function captureFrames() {
  let id = 0;
  const frames = new Map<number, FrameRequestCallback>();
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    frames.set(++id, callback);
    return id;
  });
  vi.stubGlobal("cancelAnimationFrame", (handle: number) =>
    frames.delete(handle),
  );
  return frames;
}

function stepFrames(frames: Map<number, FrameRequestCallback>, at: number) {
  for (const [handle, callback] of [...frames.entries()]) {
    frames.delete(handle);
    act(() => callback(at));
  }
}

function PresenceProbe() {
  const presence = useBuddyPresencePreferences();
  return (
    <output
      data-testid="presence-probe"
      data-enabled={String(presence.enabled)}
      data-reduced-motion={String(presence.reducedMotion)}
    >
      {presence.workState}
    </output>
  );
}

const sceneProps = {
  state: createInitialSemanticState(),
  stage: STAGES[0],
  palette: PALETTES[0],
  pulse: null,
  pet: undefined,
  nowPlaying: null,
  activeQuest: null,
  activeSpeech: null,
  setupNeeded: false,
  onCanvasEvent: () => undefined,
  onCare: () => undefined,
  onOpenPage: () => undefined,
  onRunMode: () => undefined,
  onDismissSetup: () => undefined,
  onSpeechControl: () => undefined,
};

describe("BuddyPresenceProvider wiring", () => {
  beforeEach(() => {
    installPresenceHandlers();
    setHidden(false);
  });
  afterEach(() => {
    setHidden(false);
    setBuddyTextPresence(false);
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("enables presence and derives a live work state from native facts", async () => {
    render(
      <BuddyPresenceProvider>
        <PresenceProbe />
      </BuddyPresenceProvider>,
    );

    const probe = screen.getByTestId("presence-probe");
    expect(probe).toHaveAttribute("data-enabled", "true");
    expect(probe).toHaveAttribute("data-reduced-motion", "undefined");
    await waitFor(() => expect(probe).toHaveTextContent("idle"));
  });

  it.each(["avatar", "scene"] as const)(
    "keeps no queued animation frame while the %s is hidden",
    (surface) => {
      const frames = captureFrames();
      vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);

      const view = render(
        <BuddyPresenceProvider>
          <PresenceProbe />
          {surface === "avatar" ? (
            <BuddyCanvas state={createInitialSemanticState()} />
          ) : (
            <BuddyWorld {...sceneProps} />
          )}
        </BuddyPresenceProvider>,
      );

      expect(screen.getByTestId("presence-probe")).toHaveAttribute(
        "data-enabled",
        "true",
      );
      expect(frames.size).toBeGreaterThan(0);
      setHidden(true);
      expect(frames.size).toBe(0);
      setHidden(false);
      expect(frames.size).toBeGreaterThan(0);
      view.unmount();
      expect(frames.size).toBe(0);
    },
  );

  it("throttles drawn frames under prefers-reduced-motion", () => {
    stubReducedMotion(true);
    const frames = captureFrames();
    const getContext = vi
      .spyOn(HTMLCanvasElement.prototype, "getContext")
      .mockReturnValue(null);

    render(
      <BuddyPresenceProvider>
        <BuddyCanvas state={createInitialSemanticState()} />
      </BuddyPresenceProvider>,
    );

    for (const at of [0, 100, 200, 300]) stepFrames(frames, at);
    expect(getContext).toHaveBeenCalledTimes(2);
  });

  it("draws every frame when reduced motion is not requested", () => {
    stubReducedMotion(false);
    const frames = captureFrames();
    const getContext = vi
      .spyOn(HTMLCanvasElement.prototype, "getContext")
      .mockReturnValue(null);

    render(
      <BuddyPresenceProvider>
        <BuddyCanvas state={createInitialSemanticState()} />
      </BuddyPresenceProvider>,
    );

    for (const at of [0, 100, 200, 300]) stepFrames(frames, at);
    expect(getContext).toHaveBeenCalledTimes(4);
  });

  it("offers pause and stop controls for a started play session", () => {
    render(
      <BuddyPresenceProvider>
        <BuddyWorld {...sceneProps} />
      </BuddyPresenceProvider>,
    );

    fireEvent.click(screen.getByRole("button", { name: /^Play fetch with/ }));

    expect(
      screen.getByRole("button", { name: "Pause play" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Stop play" }),
    ).toBeInTheDocument();
  });

  it("suspends a running play session when the document is hidden", () => {
    render(
      <BuddyPresenceProvider>
        <BuddyWorld {...sceneProps} />
      </BuddyPresenceProvider>,
    );

    fireEvent.click(screen.getByRole("button", { name: /^Play fetch with/ }));
    expect(screen.getByTestId("buddy-world")).toHaveAttribute(
      "data-play-suspended",
      "false",
    );

    setHidden(true);

    expect(screen.getByTestId("buddy-world")).toHaveAttribute(
      "data-play-suspended",
      "true",
    );
    expect(
      screen.getByRole("button", { name: "Resume play" }),
    ).toBeInTheDocument();
  });

  it("propagates the text-only presence preference to the scene", () => {
    setBuddyTextPresence(true);

    render(
      <BuddyPresenceProvider>
        <BuddyWorld {...sceneProps} />
      </BuddyPresenceProvider>,
    );

    expect(screen.getByTestId("buddy-world-canvas")).toHaveStyle({
      visibility: "hidden",
    });
    expect(screen.queryByTestId("buddy-sprite-hit")).toBeNull();
  });

  it("draws the scene character when text-only presence is off", () => {
    setBuddyTextPresence(false);

    render(
      <BuddyPresenceProvider>
        <BuddyWorld {...sceneProps} />
      </BuddyPresenceProvider>,
    );

    expect(screen.getByTestId("buddy-world-canvas")).not.toHaveStyle({
      visibility: "hidden",
    });
    expect(screen.getByTestId("buddy-sprite-hit")).toBeInTheDocument();
  });

  it("leaves a play session unstoppable without the provider", () => {
    render(<BuddyWorld {...sceneProps} />);

    fireEvent.click(screen.getByRole("button", { name: /^Play fetch with/ }));

    expect(screen.queryByRole("button", { name: "Pause play" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Stop play" })).toBeNull();
  });
});
