/**
 * Reveal the transcript card that owns a finished process. The transcript
 * marks those cards with `data-exec-process-id`, so scrolling to the matching
 * card is the whole navigation contract.
 */
export function revealProcessOutput(processId: string): boolean {
  const card = Array.from(
    document.querySelectorAll("[data-exec-process-id]"),
  ).find((item) => item.getAttribute("data-exec-process-id") === processId);
  if (!card) return false;
  card.scrollIntoView({
    block: "center",
    behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches
      ? "instant"
      : "smooth",
  });
  return true;
}
