import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { BuddySpeechBubble } from "../features/Buddy/BuddySpeechBubble";
import { PALETTES } from "../features/Buddy/constants";
import type { BuddySpeechBubbleProps } from "../features/Buddy/BuddySpeechBubble";

function renderBubble(overrides: Partial<BuddySpeechBubbleProps> = {}) {
  const props: BuddySpeechBubbleProps = {
    text: "Hello meadow",
    textKey: 1,
    enterKey: 1,
    position: "top",
    palette: PALETTES[0],
    visible: true,
    opacity: 1,
    compact: false,
    width: "max-content",
    maxWidth: "300px",
    whiteSpace: "nowrap",
    anchorStyle: {},
    ...overrides,
  };
  return render(<BuddySpeechBubble {...props} />);
}

function bubbleAnchor(container: HTMLElement): HTMLElement {
  const anchor = container.querySelector("[data-bubble-position]");
  if (!(anchor instanceof HTMLElement)) throw new Error("anchor not found");
  return anchor;
}

describe("BuddySpeechBubble styles", () => {
  it("defaults to the say style with triangle tails", () => {
    const { container } = renderBubble();
    const anchor = bubbleAnchor(container);
    expect(anchor).toHaveAttribute("data-style", "say");
    expect(anchor).toHaveAttribute("data-closing", "false");
    expect(anchor).toHaveAttribute("data-exit-kind", "natural");
    expect(container.querySelectorAll("[class*='tailOuter']")).toHaveLength(1);
    expect(container.querySelectorAll("[class*='thinkTail']")).toHaveLength(0);
  });

  it("renders three cloud tail circles for the think style", () => {
    const { container } = renderBubble({ bubbleStyle: "think" });
    expect(bubbleAnchor(container)).toHaveAttribute("data-style", "think");
    expect(container.querySelectorAll("[class*='thinkTail']")).toHaveLength(3);
    expect(container.querySelectorAll("[class*='tailOuter']")).toHaveLength(0);
  });

  it("renders drifting notes for the sing style", () => {
    const { container } = renderBubble({ bubbleStyle: "sing" });
    expect(container.querySelectorAll("[class*='singNote']").length).toBe(4);
  });

  it("renders a ring pulse for the alert style", () => {
    const { container } = renderBubble({ bubbleStyle: "alert" });
    expect(container.querySelectorAll("[class*='alertRing']")).toHaveLength(1);
  });

  it("marks the closing phase with its exit kind", () => {
    const { container } = renderBubble({
      closing: true,
      exitKind: "accept",
    });
    const anchor = bubbleAnchor(container);
    expect(anchor).toHaveAttribute("data-closing", "true");
    expect(anchor).toHaveAttribute("data-exit-kind", "accept");
    expect(anchor.style.pointerEvents).toBe("none");
  });

  it("renders a media slot below the text", () => {
    const { container } = renderBubble({
      media: <canvas data-testid="dream-canvas" width={132} height={76} />,
    });
    expect(screen.getByTestId("dream-canvas")).toBeInTheDocument();
    expect(container.querySelector("[class*='media']")).not.toBeNull();
  });
});

describe("BuddySpeechBubble control choreography", () => {
  const controls = [
    { id: "yes", label: "Throw it", action: "fetch", style: "primary" },
    { id: "later", label: "Later", action: "dismiss", style: "secondary" },
  ];

  it("dispatches the control immediately on click", () => {
    const onControlClick = vi.fn();
    renderBubble({ controls, onControlClick });
    fireEvent.click(screen.getByRole("button", { name: "Throw it" }));
    expect(onControlClick).toHaveBeenCalledTimes(1);
    expect(onControlClick).toHaveBeenCalledWith(
      expect.objectContaining({ id: "yes" }),
    );
  });

  it("marks the clicked button and fades its siblings", () => {
    renderBubble({ controls, onControlClick: vi.fn() });
    const accept = screen.getByRole("button", { name: "Throw it" });
    const later = screen.getByRole("button", { name: "Later" });
    expect(accept).toHaveAttribute("data-clicked", "false");
    fireEvent.click(accept);
    expect(accept).toHaveAttribute("data-clicked", "true");
    expect(accept).toHaveAttribute("data-faded", "false");
    expect(later).toHaveAttribute("data-clicked", "false");
    expect(later).toHaveAttribute("data-faded", "true");
  });

  it("resets click state when the text changes", () => {
    const { rerender, container } = renderBubble({
      controls,
      onControlClick: vi.fn(),
    });
    fireEvent.click(screen.getByRole("button", { name: "Throw it" }));
    expect(container.querySelectorAll("[data-clicked='true']").length).toBe(1);
    rerender(
      <BuddySpeechBubble
        text="New line"
        textKey={2}
        enterKey={1}
        position="top"
        palette={PALETTES[0]}
        visible
        opacity={1}
        compact={false}
        width="max-content"
        maxWidth="300px"
        whiteSpace="nowrap"
        anchorStyle={{}}
        controls={controls}
        onControlClick={vi.fn()}
      />,
    );
    expect(container.querySelectorAll("[data-clicked='true']").length).toBe(0);
  });

  it("keeps pointer events enabled only while interactive", () => {
    const { container } = renderBubble({ controls, onControlClick: vi.fn() });
    expect(bubbleAnchor(container).style.pointerEvents).toBe("auto");
    const { container: closedContainer } = renderBubble({
      controls,
      onControlClick: vi.fn(),
      closing: true,
    });
    expect(bubbleAnchor(closedContainer).style.pointerEvents).toBe("none");
  });
});
