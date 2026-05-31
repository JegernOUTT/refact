import { render, screen, waitFor } from "../utils/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BuddyCanvas } from "../features/Buddy/BuddyCanvas";
import type { BuddySemanticState } from "../features/Buddy/types";

const noopCanvasContext = {
  clearRect: vi.fn(),
  fillRect: vi.fn(),
  fillText: vi.fn(),
  getImageData: vi.fn(() => ({ data: new Uint8ClampedArray(4) }) as ImageData),
  putImageData: vi.fn(),
  restore: vi.fn(),
  save: vi.fn(),
  scale: vi.fn(),
  translate: vi.fn(),
  beginPath: vi.fn(),
  arc: vi.fn(),
  ellipse: vi.fn(),
  fill: vi.fn(),
  moveTo: vi.fn(),
  lineTo: vi.fn(),
  stroke: vi.fn(),
  imageSmoothingEnabled: false,
  globalAlpha: 1,
  fillStyle: "#000000",
  strokeStyle: "#000000",
  lineWidth: 1,
  lineCap: "butt" as CanvasLineCap,
  font: "",
  textAlign: "center" as CanvasTextAlign,
  textBaseline: "top" as CanvasTextBaseline,
} satisfies Partial<CanvasRenderingContext2D>;

const noopContext = noopCanvasContext as unknown as CanvasRenderingContext2D;

function makeSemanticState(): BuddySemanticState {
  return {
    name: "Buddy",
    paletteIndex: 0,
    born: 0,
    mood: {
      happiness: 80,
      energy: 80,
      curiosity: 70,
      anxiety: 0,
      boredom: 10,
      affection: 80,
    },
    personality: {
      playfulness: 70,
      confidence: 60,
      clinginess: 70,
      resilience: 60,
      chaos: 30,
      sociability: 70,
      curiosity: 70,
    },
    progress: { xp: 0, stage: 2 },
    activity: {
      mood: "idle",
      animationType: "idle",
      lastSignalTime: 0,
      lastSignalType: null,
    },
    skills: [],
    log: [],
  };
}

describe("BuddyCanvas compact speech layout", () => {
  let frameCallbacks: FrameRequestCallback[] = [];

  beforeEach(() => {
    frameCallbacks = [];
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      frameCallbacks.push(callback);
      return frameCallbacks.length;
    });
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => {
      return undefined;
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  function runAnimationFrame(): void {
    const callback = frameCallbacks.shift();
    if (callback) callback(0);
  }

  it("caps long compact world speech with real bubble rendering", async () => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(
      noopContext,
    );

    render(
      <BuddyCanvas
        state={makeSemanticState()}
        displaySize={230}
        speechOverride="The observatory has a very long update that should avoid narrow side clipping in compact scenes."
        bubblePosition="top"
        compactBubble
      />,
    );

    runAnimationFrame();

    const bubble = await screen.findByText(/observatory has a very long/u);
    const bubbleElement = bubble.closest("div[data-bubble-position]");

    expect(bubbleElement).not.toBeNull();
    await waitFor(() => {
      expect(bubbleElement).toHaveStyle({
        width: "220px",
        maxWidth: "220px",
        whiteSpace: "normal",
      });
      expect(bubbleElement).toHaveAttribute("data-bubble-position", "top");
    });
  });

  it("expands compact chat companion side bubbles for long speech", async () => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(
      noopContext,
    );

    render(
      <BuddyCanvas
        state={makeSemanticState()}
        displaySize={160}
        speechOverride="Pixel’s here, poking the shadows—your chats are probably playing hide-and-seek with tiny gremlin socks."
        bubblePosition="left"
        compactBubble
        chatCompanionBubble
      />,
    );

    runAnimationFrame();

    const bubble = await screen.findByText(/poking the shadows/u);
    const bubbleElement = bubble.closest("div[data-bubble-position]");

    expect(bubbleElement).not.toBeNull();
    await waitFor(() => {
      expect(bubbleElement).toHaveStyle({
        width: "300px",
      });
      expect(bubbleElement?.getAttribute("style")).toContain(
        "max-width: min(460px, 72vw)",
      );
      expect(bubbleElement?.getAttribute("style")).toContain(
        "right: calc(78% - 0px)",
      );
      expect(bubbleElement?.getAttribute("style")).toContain(
        "white-space: normal",
      );
      expect(bubbleElement).toHaveAttribute("data-bubble-position", "left");
    });
  });

  it("keeps explicit compact bubbles narrow unless chat companion layout is requested", async () => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(
      noopContext,
    );

    render(
      <BuddyCanvas
        state={makeSemanticState()}
        displaySize={160}
        speechOverride="The observatory has a very long update that should keep the compact side bubble narrow."
        bubblePosition="left"
        compactBubble
      />,
    );

    runAnimationFrame();

    const bubble = await screen.findByText(/observatory has a very long/u);
    const bubbleElement = bubble.closest("div[data-bubble-position]");

    expect(bubbleElement).not.toBeNull();
    await waitFor(() => {
      expect(bubbleElement).toHaveStyle({
        width: "220px",
        maxWidth: "220px",
        whiteSpace: "normal",
      });
      expect(bubbleElement).toHaveAttribute("data-bubble-position", "left");
    });
  });

  it("keeps normal world side bubble sizing at the same display size", async () => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(
      noopContext,
    );

    render(
      <BuddyCanvas
        state={makeSemanticState()}
        displaySize={230}
        speechOverride="The observatory has a very long update that should stay side aware in normal scenes."
        bubblePosition="left"
      />,
    );

    runAnimationFrame();

    const bubble = await screen.findByText(/observatory has a very long/u);
    const bubbleElement = bubble.closest("div[data-bubble-position]");

    expect(bubbleElement).not.toBeNull();
    await waitFor(() => {
      expect(bubbleElement).toHaveStyle({
        width: "270px",
        maxWidth: "300px",
      });
      expect(bubbleElement).toHaveAttribute("data-bubble-position", "left");
    });
  });
});
