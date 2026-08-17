import { describe, expect, it } from "vitest";

import { setUpStore } from "../../../app/store";
import {
  advanceEditPlayer,
  clearLiveFileUpdatesForChat,
  type EditPlayerStep,
  enqueueEditPlayerSteps,
  MAX_EDIT_PLAYER_STEPS,
  resetEditPlayer,
  selectActiveEditPlayerStep,
  selectEditPlayer,
  setEditPlayerStatus,
} from "./filesPanelSlice";

const step = (id: string, path = "/w/a.ts"): EditPlayerStep => ({
  id,
  path,
  revision: id,
  line: 1,
  chunks: [],
  operation: "write",
});

describe("edit player queue", () => {
  it("starts playing when the first steps arrive", () => {
    const store = setUpStore();
    store.dispatch(
      enqueueEditPlayerSteps({ chatId: "chat-a", steps: [step("1")] }),
    );

    const player = selectEditPlayer(store.getState());
    expect(player.status).toBe("playing");
    expect(player.chatId).toBe("chat-a");
    expect(selectActiveEditPlayerStep(store.getState())?.id).toBe("1");
  });

  it("ignores steps it has already queued", () => {
    const store = setUpStore();
    store.dispatch(
      enqueueEditPlayerSteps({ chatId: "chat-a", steps: [step("1")] }),
    );
    store.dispatch(
      enqueueEditPlayerSteps({
        chatId: "chat-a",
        steps: [step("1"), step("2")],
      }),
    );

    expect(selectEditPlayer(store.getState()).steps).toHaveLength(2);
  });

  it("advances through queued steps and stops past the end", () => {
    const store = setUpStore();
    store.dispatch(
      enqueueEditPlayerSteps({
        chatId: "chat-a",
        steps: [step("1"), step("2")],
      }),
    );

    store.dispatch(advanceEditPlayer());
    expect(selectActiveEditPlayerStep(store.getState())?.id).toBe("2");

    store.dispatch(advanceEditPlayer());
    expect(selectActiveEditPlayerStep(store.getState())).toBeUndefined();

    store.dispatch(advanceEditPlayer());
    expect(selectEditPlayer(store.getState()).index).toBe(2);
  });

  it("resets the queue when a different chat starts emitting edits", () => {
    const store = setUpStore();
    store.dispatch(
      enqueueEditPlayerSteps({ chatId: "chat-a", steps: [step("1")] }),
    );
    store.dispatch(
      enqueueEditPlayerSteps({ chatId: "chat-b", steps: [step("9")] }),
    );

    const player = selectEditPlayer(store.getState());
    expect(player.chatId).toBe("chat-b");
    expect(player.steps.map((entry) => entry.id)).toEqual(["9"]);
    expect(player.index).toBe(0);
  });

  it("keeps the queue bounded and preserves the active step", () => {
    const store = setUpStore();
    const many = Array.from(
      { length: MAX_EDIT_PLAYER_STEPS + 10 },
      (_, index) => step(String(index)),
    );
    store.dispatch(enqueueEditPlayerSteps({ chatId: "chat-a", steps: many }));

    const player = selectEditPlayer(store.getState());
    expect(player.steps).toHaveLength(MAX_EDIT_PLAYER_STEPS);
    expect(player.index).toBe(0);
    expect(player.steps[0].id).toBe("10");
  });

  it("hides the active step while paused is stopped", () => {
    const store = setUpStore();
    store.dispatch(
      enqueueEditPlayerSteps({ chatId: "chat-a", steps: [step("1")] }),
    );

    store.dispatch(setEditPlayerStatus("paused"));
    expect(selectActiveEditPlayerStep(store.getState())?.id).toBe("1");

    store.dispatch(resetEditPlayer());
    expect(selectActiveEditPlayerStep(store.getState())).toBeUndefined();
    expect(selectEditPlayer(store.getState()).steps).toHaveLength(0);
  });

  it("drops the queue when its chat is cleared", () => {
    const store = setUpStore();
    store.dispatch(
      enqueueEditPlayerSteps({ chatId: "chat-a", steps: [step("1")] }),
    );
    store.dispatch(clearLiveFileUpdatesForChat("chat-a"));

    expect(selectEditPlayer(store.getState()).steps).toHaveLength(0);
    expect(selectEditPlayer(store.getState()).status).toBe("idle");
  });
});
