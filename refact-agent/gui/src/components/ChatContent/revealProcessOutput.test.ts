import { afterEach, describe, expect, it, vi } from "vitest";
import {
  PROCESS_OUTPUT_REVEAL_EVENT,
  isProcessOutputRevealDetail,
  revealProcessOutput,
} from "./revealProcessOutput";

function card(processId: string): HTMLElement {
  const element = document.createElement("div");
  element.setAttribute("data-exec-process-id", processId);
  document.body.appendChild(element);
  return element;
}

afterEach(() => {
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

describe("revealProcessOutput", () => {
  it("scrolls a mounted card into view without asking groups", () => {
    const scroll = vi
      .spyOn(Element.prototype, "scrollIntoView")
      .mockImplementation(() => undefined);
    const target = card("proc-1");
    card("proc-2");
    const listener = vi.fn();
    window.addEventListener(PROCESS_OUTPUT_REVEAL_EVENT, listener);

    expect(revealProcessOutput("proc-1")).toBe(true);

    window.removeEventListener(PROCESS_OUTPUT_REVEAL_EVENT, listener);
    expect(listener).not.toHaveBeenCalled();
    expect(scroll).toHaveBeenCalledTimes(1);
    expect(scroll.mock.contexts[0]).toBe(target);
  });

  it("asks collapsed groups to claim a card that is not mounted", () => {
    const claim = (event: Event) => {
      if (event instanceof CustomEvent) {
        const detail: unknown = event.detail;
        if (isProcessOutputRevealDetail(detail)) detail.claimed = true;
      }
    };
    window.addEventListener(PROCESS_OUTPUT_REVEAL_EVENT, claim);
    expect(revealProcessOutput("proc-hidden")).toBe(true);
    window.removeEventListener(PROCESS_OUTPUT_REVEAL_EVENT, claim);

    expect(revealProcessOutput("proc-hidden")).toBe(false);
  });

  it("validates the reveal detail shape", () => {
    expect(
      isProcessOutputRevealDetail({ processId: "p", claimed: false }),
    ).toBe(true);
    expect(isProcessOutputRevealDetail({ processId: 1, claimed: false })).toBe(
      false,
    );
    expect(isProcessOutputRevealDetail({ processId: "p" })).toBe(false);
    expect(isProcessOutputRevealDetail(null)).toBe(false);
  });

  it("skips cards inside a closing, opening or inert group panel", () => {
    const scroll = vi
      .spyOn(Element.prototype, "scrollIntoView")
      .mockImplementation(() => undefined);
    const panel = (open: string, expanded: string) => {
      const element = document.createElement("div");
      element.setAttribute("data-tool-run-panel", "");
      element.setAttribute("data-open", open);
      element.setAttribute("data-expanded", expanded);
      document.body.appendChild(element);
      return element;
    };
    panel("false", "true").appendChild(card("proc-closing"));
    panel("true", "false").appendChild(card("proc-opening"));
    const inert = document.createElement("div");
    inert.setAttribute("inert", "");
    document.body.appendChild(inert);
    inert.appendChild(card("proc-inert"));
    const settled = card("proc-open");
    panel("true", "true").appendChild(settled);

    expect(revealProcessOutput("proc-closing")).toBe(false);
    expect(revealProcessOutput("proc-opening")).toBe(false);
    expect(revealProcessOutput("proc-inert")).toBe(false);
    expect(scroll).not.toHaveBeenCalled();
    expect(revealProcessOutput("proc-open")).toBe(true);
    expect(scroll.mock.contexts[0]).toBe(settled);
  });
});
