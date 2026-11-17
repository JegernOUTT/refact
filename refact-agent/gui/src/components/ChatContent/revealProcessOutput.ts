export const PROCESS_OUTPUT_REVEAL_EVENT = "refact:process-output-reveal";

export type ProcessOutputRevealDetail = {
  processId: string;
  claimed: boolean;
};

export function isProcessOutputRevealDetail(
  value: unknown,
): value is ProcessOutputRevealDetail {
  return (
    typeof value === "object" &&
    value !== null &&
    "processId" in value &&
    typeof value.processId === "string" &&
    "claimed" in value &&
    typeof value.claimed === "boolean"
  );
}

function isSettledVisible(card: Element): boolean {
  if (card.closest("[inert]")) return false;
  const panel = card.closest("[data-tool-run-panel]");
  return (
    !panel ||
    (panel.getAttribute("data-open") === "true" &&
      panel.getAttribute("data-expanded") === "true")
  );
}

function findProcessCard(processId: string): Element | undefined {
  return Array.from(document.querySelectorAll("[data-exec-process-id]")).find(
    (item) =>
      item.getAttribute("data-exec-process-id") === processId &&
      isSettledVisible(item),
  );
}

/**
 * Reveal the transcript card that owns a finished process. The transcript
 * marks those cards with `data-exec-process-id`, so scrolling to the matching
 * card is the whole navigation contract. A card hidden inside a collapsed,
 * closing or still-opening tool run group is skipped; the group that owns it
 * claims the request, opens itself and reveals the card once it has settled.
 */
export function revealProcessOutput(processId: string): boolean {
  const card = findProcessCard(processId);
  if (card) {
    card.scrollIntoView({
      block: "center",
      behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches
        ? "instant"
        : "smooth",
    });
    return true;
  }
  const detail: ProcessOutputRevealDetail = { processId, claimed: false };
  window.dispatchEvent(
    new CustomEvent<ProcessOutputRevealDetail>(PROCESS_OUTPUT_REVEAL_EVENT, {
      detail,
    }),
  );
  return detail.claimed;
}
