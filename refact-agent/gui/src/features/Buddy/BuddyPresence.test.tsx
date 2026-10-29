import { describe, it, vi, expect, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "../../utils/test-utils";
import { BuddyPresence } from "./BuddyPresence";
import { createInitialSemanticState } from "./state";
import type { BuddySceneSpeech } from "./buddySceneSpeech";

const speech: BuddySceneSpeech = {
  id: "one",
  text: "Choose the next step",
  source: "speech",
  controls: [
    {
      id: "proceed",
      label: "Proceed",
      action: "open_history",
      style: "primary",
    },
  ],
};

describe("Buddy presence", () => {
  afterEach(() =>
    window.localStorage.removeItem("refact.buddy.text-presence.v1"),
  );
  it("hides the mini avatar while the full scene is mounted and shows it when it is not", () => {
    const props = {
      state: createInitialSemanticState(),
      headline: "Ready",
      speech,
      onToggleScene: vi.fn(),
      onControl: vi.fn(),
    };
    const view = render(
      <BuddyPresence {...props} sceneMounted sceneToggleLabel="Close Play" />,
    );
    expect(screen.queryByTestId("buddy-presence-avatar")).toBeNull();
    expect(
      screen.getByRole("button", { name: "Close Play" }),
    ).not.toHaveAttribute("aria-expanded");
    view.rerender(
      <BuddyPresence
        {...props}
        sceneMounted={false}
        sceneToggleLabel="Open Play"
      />,
    );
    expect(screen.getByTestId("buddy-presence-avatar")).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "Open Play" }),
    ).not.toHaveAttribute("aria-expanded");
  });
  it("hides the mood line when the headline is blank", () => {
    render(
      <BuddyPresence
        state={createInitialSemanticState()}
        headline="   "
        vibe="Cheerful, supportive"
        speech={null}
        sceneMounted
        sceneToggleLabel="Open Play"
        onToggleScene={vi.fn()}
        onControl={vi.fn()}
      />,
    );
    expect(screen.queryByTestId("buddy-presence-mood")).toBeNull();
    expect(screen.getByTestId("buddy-presence-vibe")).toHaveTextContent(
      "Cheerful, supportive",
    );
  });
  it("shows the name as a heading, the vibe as a meta line and the mood and speech as text", () => {
    render(
      <BuddyPresence
        state={createInitialSemanticState()}
        headline="Snack reserves are running low"
        vibe="Cheerful, supportive"
        speech={speech}
        sceneMounted
        sceneToggleLabel="Open Play"
        onToggleScene={vi.fn()}
        onControl={vi.fn()}
      />,
    );
    expect(screen.getByRole("heading", { level: 2 })).toBeTruthy();
    expect(screen.getByTestId("buddy-presence-vibe")).toHaveTextContent(
      "Cheerful, supportive",
    );
    expect(screen.getByTestId("buddy-presence-mood")).toHaveTextContent(
      /^Snack reserves are running low$/,
    );
    expect(screen.getByTestId("buddy-presence-speech")).toHaveTextContent(
      "Choose the next step",
    );
  });
  it("unmounts the decorative avatar in text-only mode while keeping controls usable", () => {
    const props = {
      state: createInitialSemanticState(),
      headline: "Ready",
      speech,
      sceneMounted: false,
      sceneToggleLabel: "Open Play",
      onToggleScene: vi.fn(),
      onControl: vi.fn(),
    };
    const view = render(<BuddyPresence {...props} />);
    expect(screen.getByTestId("buddy-presence-avatar")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Text-only presence" }));
    expect(screen.queryByTestId("buddy-presence-avatar")).toBeNull();
    expect(screen.getByRole("button", { name: "Proceed" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Open Play" })).toBeEnabled();
    view.unmount();
    render(<BuddyPresence {...props} />);
    expect(screen.queryByTestId("buddy-presence-avatar")).toBeNull();
    expect(
      screen.getByRole("button", { name: "Text-only presence" }),
    ).toHaveAttribute("aria-pressed", "true");
  });
  it("keeps controls pending and reports failure instead of pretending success", async () => {
    let fail: (() => void) | undefined;
    const onControl = vi.fn(
      () =>
        new Promise<void>((_resolve, reject) => {
          fail = () => reject(new Error("offline"));
        }),
    );
    render(
      <BuddyPresence
        state={createInitialSemanticState()}
        headline="Ready"
        speech={speech}
        sceneMounted={false}
        sceneToggleLabel="Open Play"
        onToggleScene={vi.fn()}
        onControl={onControl}
      />,
    );
    const button = screen.getByRole("button", { name: "Proceed" });
    fireEvent.click(button);
    fireEvent.click(button);
    expect(onControl).toHaveBeenCalledTimes(1);
    expect(button).toBeDisabled();
    fail?.();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Action did not complete",
    );
    await waitFor(() => expect(button).toBeEnabled());
  });
  it("does not let an old request completion unlock a newer speech request", async () => {
    const finish: (() => void)[] = [];
    const onControl = vi.fn(
      () => new Promise<void>((resolve) => finish.push(resolve)),
    );
    const props = {
      state: createInitialSemanticState(),
      headline: "Ready",
      sceneMounted: false,
      sceneToggleLabel: "Open Play",
      onToggleScene: vi.fn(),
      onControl,
    };
    const view = render(<BuddyPresence {...props} speech={speech} />);
    fireEvent.click(screen.getByRole("button", { name: "Proceed" }));
    view.rerender(
      <BuddyPresence {...props} speech={{ ...speech, id: "two" }} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Proceed" }));
    finish[0]();
    await Promise.resolve();
    expect(screen.getByRole("button", { name: "Proceed" })).toBeDisabled();
    finish[1]();
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Proceed" })).toBeEnabled(),
    );
  });
});
