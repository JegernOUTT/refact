import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react-dom/test-utils";
import userEvent from "@testing-library/user-event";
import { fireEvent, render, screen } from "../../utils/test-utils";
import { BuddyOperatorPlayPreview } from "./BuddyOperatorPlayPreview";
import { FETCH_PHASE_DURATIONS_MS } from "./buddyPlaySessions";

let clockMs = 1_000_000;
const now = () => clockMs;

function advance(ms: number) {
  act(() => {
    clockMs += ms;
    vi.advanceTimersByTime(ms);
  });
}

function setDocumentHidden(hidden: boolean) {
  Object.defineProperty(document, "hidden", {
    configurable: true,
    get: () => hidden,
  });
  act(() => {
    document.dispatchEvent(new Event("visibilitychange"));
  });
}

function preview() {
  return screen.getByTestId("buddy-operator-play-preview");
}

describe("BuddyOperatorPlayPreview", () => {
  beforeEach(() => {
    localStorage.clear();
    clockMs = 1_000_000;
    vi.useFakeTimers();
  });

  afterEach(() => {
    setDocumentHidden(false);
    vi.useRealTimers();
  });

  it("is disabled by default and renders nothing", () => {
    render(<BuddyOperatorPlayPreview nowMs={now} />);
    expect(
      screen.queryByTestId("buddy-operator-play-preview"),
    ).not.toBeInTheDocument();
  });

  it("renders the motion label for the given work state without changing work", () => {
    render(
      <BuddyOperatorPlayPreview enabled workState="blocked" nowMs={now} />,
    );
    expect(preview()).toHaveAttribute("data-work-state", "blocked");
    expect(preview()).toHaveAttribute("data-pose", "shield");
    expect(
      screen.getByText(/play never changes your project or permissions/i),
    ).toBeInTheDocument();
  });

  it.each(["fetch", "firefly"])(
    "keeps real keyboard focus through %s start, aim, pause and resume",
    async (kind) => {
      vi.useRealTimers();
      const user = userEvent.setup();
      render(<BuddyOperatorPlayPreview enabled nowMs={now} />);
      const start = screen.getByRole("button", { name: `Start ${kind}` });
      start.focus();
      await user.keyboard("{Enter}");
      expect(start).toBeDisabled();
      expect(preview()).toHaveFocus();
      await user.keyboard("1");
      expect(preview()).toHaveFocus();
      await user.keyboard("{Escape}");
      expect(preview()).toHaveAttribute("data-suspended", "true");
      clockMs += 60_000;
      expect(preview()).toHaveAttribute(
        "data-play",
        kind === "fetch" ? "fetch:throwing" : "firefly:pouncing",
      );
      await user.keyboard("{Enter}");
      expect(preview()).toHaveAttribute("data-suspended", "false");
      await user.click(screen.getByRole("button", { name: "Pause play" }));
      expect(preview()).toHaveFocus();
      await user.click(screen.getByRole("button", { name: "Resume play" }));
      expect(preview()).toHaveFocus();
      await user.keyboard("{Escape}");
      expect(preview()).toHaveAttribute("data-suspended", "true");
    },
  );

  it.each(["keyboard", "pointer"])(
    "hands focus off before an armed target unmounts via %s",
    async (input) => {
      vi.useRealTimers();
      const user = userEvent.setup();
      render(<BuddyOperatorPlayPreview enabled nowMs={now} />);
      await user.click(screen.getByRole("button", { name: "Start fetch" }));
      const target = screen.getByRole("button", { name: /Throw left/ });
      target.focus();
      if (input === "keyboard") await user.keyboard("1");
      else await user.click(target);
      expect(target).not.toBeInTheDocument();
      expect(preview()).toHaveFocus();
      await user.keyboard("{Escape}");
      expect(preview()).toHaveAttribute("data-suspended", "true");
    },
  );

  it("leaves external decision focus and Escape alone", async () => {
    vi.useRealTimers();
    const user = userEvent.setup();
    const onKeyDown = vi.fn();
    render(
      <>
        <BuddyOperatorPlayPreview enabled nowMs={now} />
        <button onKeyDown={onKeyDown}>Native decision</button>
      </>,
    );
    await user.click(screen.getByRole("button", { name: "Start fetch" }));
    const decision = screen.getByRole("button", { name: "Native decision" });
    decision.focus();
    await user.keyboard("{Escape}");
    expect(decision).toHaveFocus();
    expect(onKeyDown).toHaveBeenCalledOnce();
    expect(preview()).toHaveAttribute("data-suspended", "false");
  });

  it("plays a round entirely from the keyboard", () => {
    render(<BuddyOperatorPlayPreview enabled nowMs={now} />);
    fireEvent.click(screen.getByRole("button", { name: "Start fetch" }));
    expect(preview()).toHaveAttribute("data-play", "fetch:armed");

    fireEvent.keyDown(preview(), { key: "1" });
    expect(preview()).toHaveAttribute("data-play", "fetch:throwing");

    advance(FETCH_PHASE_DURATIONS_MS.throwing + 100);
    expect(preview()).toHaveAttribute("data-play", "fetch:chasing");
  });

  it("ignores keys that are not mapped to a play target", () => {
    render(<BuddyOperatorPlayPreview enabled nowMs={now} />);
    fireEvent.click(screen.getByRole("button", { name: "Start fetch" }));
    fireEvent.keyDown(preview(), { key: "z" });
    expect(preview()).toHaveAttribute("data-play", "fetch:armed");
  });

  it("offers a DOM button for every keyboard target", () => {
    render(<BuddyOperatorPlayPreview enabled nowMs={now} />);
    fireEvent.click(screen.getByRole("button", { name: "Start firefly" }));
    const targets = screen.getByLabelText("Pounce targets");
    expect(targets.querySelectorAll("button")).toHaveLength(3);
    fireEvent.click(
      screen.getByRole("button", { name: /Pounce on the middle glow/i }),
    );
    expect(preview()).toHaveAttribute("data-play", "firefly:pouncing");
  });

  it("interrupts with Escape and resumes at the same phase progress", () => {
    render(<BuddyOperatorPlayPreview enabled nowMs={now} />);
    fireEvent.click(screen.getByRole("button", { name: "Start fetch" }));
    fireEvent.keyDown(preview(), { key: "1" });
    advance(200);

    fireEvent.keyDown(preview(), { key: "Escape" });
    expect(preview()).toHaveAttribute("data-suspended", "true");
    expect(screen.getByRole("status")).toHaveTextContent(/paused/i);

    // Time passes while paused; it must not consume the phase.
    advance(60_000);
    expect(preview()).toHaveAttribute("data-play", "fetch:throwing");

    fireEvent.keyDown(preview(), { key: "Enter" });
    expect(preview()).toHaveAttribute("data-suspended", "false");
    advance(FETCH_PHASE_DURATIONS_MS.throwing - 200 + 100);
    expect(preview()).toHaveAttribute("data-play", "fetch:chasing");
  });

  it("suspends the loop and the session when the document is hidden", () => {
    render(<BuddyOperatorPlayPreview enabled nowMs={now} />);
    fireEvent.click(screen.getByRole("button", { name: "Start fetch" }));
    fireEvent.keyDown(preview(), { key: "2" });

    setDocumentHidden(true);
    expect(preview()).toHaveAttribute("data-loop-running", "false");
    expect(preview()).toHaveAttribute("data-loop-stop", "document_hidden");
    expect(preview()).toHaveAttribute("data-suspended", "true");
    expect(screen.getByTestId("buddy-play-loop-paused")).toBeInTheDocument();

    advance(60_000);
    expect(preview()).toHaveAttribute("data-play", "fetch:throwing");

    setDocumentHidden(false);
    expect(preview()).toHaveAttribute("data-loop-running", "true");
    fireEvent.click(screen.getByRole("button", { name: "Resume play" }));
    expect(preview()).toHaveAttribute("data-suspended", "false");
  });

  it("records an honest outcome on stop and restores focus", () => {
    render(<BuddyOperatorPlayPreview enabled nowMs={now} />);
    const start = screen.getByRole("button", { name: "Start fetch" });
    fireEvent.click(start);
    fireEvent.click(screen.getByRole("button", { name: "Stop play" }));

    const completion = screen.getByText(/Play stopped before it started/i);
    expect(completion).toHaveAttribute("data-outcome", "abandoned");
    expect(preview()).toHaveAttribute("data-play", "none");
    expect(start).toHaveFocus();
  });

  it("keeps animating at a reduced rate instead of stopping under reduced motion", () => {
    render(<BuddyOperatorPlayPreview enabled reducedMotion nowMs={now} />);
    expect(preview()).toHaveAttribute("data-loop-running", "true");
    expect(preview()).toHaveAttribute("data-loop-stop", "none");
  });

  it("keeps animating at a reduced rate under a low-power host hint", () => {
    render(<BuddyOperatorPlayPreview enabled lowPower nowMs={now} />);
    expect(preview()).toHaveAttribute("data-loop-running", "true");
    expect(preview()).toHaveAttribute("data-loop-stop", "none");
  });

  it("never punishes a long absence", () => {
    render(
      <BuddyOperatorPlayPreview
        enabled
        lastSeenAtMs={clockMs - 8 * 24 * 60 * 60 * 1000}
        nowMs={now}
      />,
    );
    expect(
      screen.getByText(/Everything is exactly where you left it/i),
    ).toBeInTheDocument();
  });

  it("reads and writes only after explicit consent, exports and resets the bounded timestamp", () => {
    const key = "buddy.presence.preview.lastSeen.v1";
    localStorage.setItem(key, "100");
    const view = render(<BuddyOperatorPlayPreview enabled nowMs={now} />);
    setDocumentHidden(true);
    expect(localStorage.getItem(key)).toBe("100");
    setDocumentHidden(false);
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByText("Export continuity"));
    expect(screen.getByLabelText("Continuity export")).toHaveTextContent(
      '"lastSeenAtMs":100',
    );
    setDocumentHidden(true);
    expect(localStorage.getItem(key)).toBe(String(clockMs));
    fireEvent.click(screen.getByText("Reset continuity"));
    view.unmount();
    expect(localStorage.getItem(key)).toBeNull();
  });

  it("does not steal Enter from the focused Stop button while paused", () => {
    render(<BuddyOperatorPlayPreview enabled nowMs={now} />);
    fireEvent.click(screen.getByText("Start fetch"));
    fireEvent.click(screen.getByText("Pause play"));
    const stop = screen.getByText("Stop play");
    stop.focus();
    fireEvent.keyDown(stop, { key: "Enter" });
    expect(preview()).toHaveAttribute("data-suspended", "true");
    fireEvent.click(stop);
    expect(screen.getByText("Start fetch")).toHaveFocus();
  });

  it("suspends on real work takeover and does not resume behind a decision", () => {
    const view = render(<BuddyOperatorPlayPreview enabled nowMs={now} />);
    fireEvent.click(screen.getByText("Start firefly"));
    fireEvent.keyDown(preview(), { key: "2" });
    view.rerender(
      <BuddyOperatorPlayPreview
        enabled
        workState="awaiting_decision"
        nowMs={now}
      />,
    );
    expect(preview()).toHaveAttribute("data-suspended", "true");
    fireEvent.click(screen.getByText("Resume play"));
    expect(preview()).toHaveAttribute("data-suspended", "true");
  });
});
