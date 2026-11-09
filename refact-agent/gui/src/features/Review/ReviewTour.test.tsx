import { http, HttpResponse } from "msw";
import { act } from "react-dom/test-utils";
import { fireEvent, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { server } from "../../utils/mockServer";
import { render } from "../../utils/test-utils";
import type { Anchor, TourStep } from "../../services/refact/review";
import { TourStepContent } from "./ReviewTour";
import { anchor, makeReview } from "./reviewFixtures";

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
});

const DIAGRAM = ["```mermaid", "graph TD", "  A --> B", "```"].join("\n");

const BODY = [
  "## Cache flow",
  "",
  "- reads go through `getCached`",
  "- writes invalidate",
  "",
  DIAGRAM,
  "",
  "```ts",
  "const x = 1;",
  "```",
].join("\n");

function step(id: string, title: string, body: string, anchors: Anchor[]) {
  const value: TourStep = {
    id,
    current: { version: 1, title, body, anchors },
    history: [],
  };
  return value;
}

function stalled(base: Anchor): Anchor {
  return { ...base, status: "stalled", stalled_reason: "lines removed" };
}

function tourReview() {
  return makeReview({
    tour: [
      step("s1", "Entry point", BODY, [
        anchor("c1", "src/a.ts", "head", 3, 7),
        anchor("c0", "src/old.ts", "base", 10),
        stalled(anchor("c1", "src/gone.ts", "head", 4)),
      ]),
      step("s2", "Plain step", "Only **text** here.", []),
    ],
  });
}

function noop() {
  return undefined;
}

describe("full tour body", () => {
  it("keeps legacy visual fences and ordinary code in the tour", () => {
    render(
      <TourStepContent
        review={tourReview()}
        stepId="s1"
        onSelectStep={noop}
        onRevealAnchor={noop}
      />,
    );
    const section = screen.getByRole("region", { name: "Tour step" });
    expect(within(section).getByText("const x = 1;")).toBeInTheDocument();
    expect(section).toHaveTextContent("const x = 1;");
  });

  it("does not treat fences nested in other code blocks as visuals", () => {
    const nested = ["````md", "```mermaid", "graph TD", "```", "````"].join(
      "\n",
    );
    render(
      <TourStepContent
        review={makeReview({ tour: [step("n", "Nested", nested, [])] })}
        stepId="n"
        onSelectStep={noop}
        onRevealAnchor={noop}
      />,
    );
    expect(screen.queryByRole("button", { name: "How it works" })).toBeNull();
    expect(screen.getByRole("region", { name: "Tour step" })).toHaveTextContent(
      "graph TD",
    );
  });
});

describe("TourStepContent", () => {
  it("renders structured Markdown, step position and exact range chips", async () => {
    const onRevealAnchor = vi.fn();
    const review = tourReview();
    const { user } = render(
      <TourStepContent
        review={review}
        stepId="s1"
        onSelectStep={noop}
        onRevealAnchor={onRevealAnchor}
      />,
    );
    const section = screen.getByRole("region", { name: "Tour step" });
    expect(within(section).getByText("Step 1 of 2")).toBeInTheDocument();
    expect(
      within(section).getByRole("heading", { name: "Entry point" }),
    ).toBeInTheDocument();
    expect(
      within(section).getByRole("heading", { name: "Cache flow" }),
    ).toBeInTheDocument();
    expect(within(section).getAllByRole("listitem").length).toBeGreaterThan(1);
    expect(within(section).getByText("getCached")).toBeInTheDocument();
    expect(within(section).getByText("const x = 1;")).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "Show src/a.ts · Head L3–7" }),
    );
    expect(onRevealAnchor).toHaveBeenCalledWith(
      review.tour[0].current.anchors[0],
    );

    const old = screen.getByRole("button", {
      name: /^Show src\/old\.ts · Base L10 on candidate/,
    });
    expect(old).toBeEnabled();
    expect(screen.getByText(/^Historical ·/)).toBeInTheDocument();

    const gone = screen.getByRole("button", { name: /src\/gone\.ts/ });
    expect(gone).toBeDisabled();
    expect(screen.getByText("Stalled · lines removed")).toBeInTheDocument();
    expect(onRevealAnchor).toHaveBeenCalledTimes(1);
  });

  it("navigates by callback without moving focus", async () => {
    const onSelectStep = vi.fn();
    const { user } = render(
      <TourStepContent
        review={tourReview()}
        stepId="s1"
        onSelectStep={onSelectStep}
        onRevealAnchor={noop}
      />,
    );
    expect(document.activeElement).toBe(document.body);
    expect(
      screen.getByRole("button", { name: "Previous step" }),
    ).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Next step" }));
    expect(onSelectStep).toHaveBeenCalledWith("s2");
    expect(screen.queryByRole("button", { name: /How it works/ })).toBeNull();
  });

  it("renders HTML directly and binds its bridge only to the current step revision", async () => {
    const onRevealAnchor = vi.fn();
    const review = tourReview();
    review.tour[0].current.body =
      'Narrative\n\n```html\n<button onclick="window.refactReview.revealAnchor(0)">Show code</button>\n```';
    let callback: (anchor: Anchor) => void = onRevealAnchor;
    const view = () => (
      <TourStepContent
        review={review}
        stepId="s1"
        onSelectStep={noop}
        onRevealAnchor={callback}
      />
    );
    const { rerender } = render(view());
    const frame = await screen.findByTitle<HTMLIFrameElement>("Entry point");
    expect(screen.getByRole("region", { name: "Tour step" })).toContainElement(
      frame,
    );
    expect(frame.srcdoc).toContain("window.refactReview.revealAnchor(0)");
    expect(screen.queryByRole("button", { name: /How it works/ })).toBeNull();
    const source = frame.srcdoc;
    const sourceWindow = frame.contentWindow;
    callback = (anchor) => {
      onRevealAnchor(anchor);
    };
    rerender(view());
    expect(screen.getByTitle("Entry point")).toBe(frame);
    expect(frame.srcdoc).toBe(source);
    expect(frame.contentWindow).toBe(sourceWindow);
    const channel = /var channel = "([^"]+)"/.exec(frame.srcdoc)?.[1];
    const send = (index: number) =>
      fireEvent(
        window,
        new MessageEvent("message", {
          source: frame.contentWindow,
          data: { type: "refact-review-reveal-anchor", channel, index },
        }),
      );
    vi.useFakeTimers();
    send(0);
    await act(() => vi.advanceTimersByTime(200));
    expect(onRevealAnchor).toHaveBeenCalledExactlyOnceWith(
      review.tour[0].current.anchors[0],
    );
    send(2);
    await act(() => vi.advanceTimersByTime(200));
    send(99);
    await act(() => vi.advanceTimersByTime(200));
    expect(onRevealAnchor).toHaveBeenCalledTimes(1);
    send(1);
    review.tour[0].current = { ...review.tour[0].current, version: 2 };
    rerender(view());
    await act(() => vi.advanceTimersByTime(200));
    expect(onRevealAnchor).toHaveBeenCalledTimes(1);
    expect(screen.getByTitle("Entry point")).not.toBe(frame);
    vi.useRealTimers();
  });

  it("renders nothing for an unknown step", () => {
    const { container } = render(
      <TourStepContent
        review={tourReview()}
        stepId="missing"
        onSelectStep={noop}
        onRevealAnchor={noop}
      />,
    );
    expect(container.querySelector("section")).toBeNull();
  });

  it("opens the same step's stored history on demand", async () => {
    let requests = 0;
    server.use(
      http.get("*/v1/reviews/r1/history", () => {
        requests += 1;
        return HttpResponse.json([]);
      }),
    );
    const { user } = render(
      <TourStepContent
        review={tourReview()}
        stepId="s2"
        onSelectStep={noop}
        onRevealAnchor={noop}
      />,
    );
    expect(requests).toBe(0);
    await user.click(screen.getByRole("button", { name: "History" }));
    expect(await screen.findByText("No earlier versions.")).toBeInTheDocument();
    expect(requests).toBe(1);
  });
});
