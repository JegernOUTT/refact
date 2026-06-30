import { describe, it, expect } from "vitest";
import {
  notificationsSlice,
  processCompleted,
  processCompletionsRecovered,
  selectProcessCompletions,
  selectUnreadNotificationCountByThread,
  type ProcessCompletedEvent,
} from "./notificationsSlice";

function liveEvent(
  overrides: Partial<ProcessCompletedEvent> = {},
): ProcessCompletedEvent {
  return {
    chat_id: "thread-1",
    seq: "5",
    type: "process_completed",
    process_id: "p1",
    status: "exited",
    exit_code: 0,
    short_description: "Build",
    mode: "background",
    ...overrides,
  };
}

describe("notificationsSlice recovery", () => {
  it("recovers completions as silent unread without toasts", () => {
    const state = notificationsSlice.reducer(
      undefined,
      processCompletionsRecovered({
        threadId: "thread-1",
        completions: [
          {
            processId: "p1",
            status: "exited",
            exitCode: 0,
            shortDescription: "Build",
            mode: "background",
          },
        ],
      }),
    );

    expect(
      selectUnreadNotificationCountByThread(
        { notifications: state },
        "thread-1",
      ),
    ).toBe(1);
    expect(selectProcessCompletions({ notifications: state })).toEqual([]);
  });

  it("does not duplicate when a live event and a recovery share a process id", () => {
    let state = notificationsSlice.reducer(
      undefined,
      processCompleted(liveEvent()),
    );
    state = notificationsSlice.reducer(
      state,
      processCompletionsRecovered({
        threadId: "thread-1",
        completions: [
          {
            processId: "p1",
            status: "exited",
            exitCode: 0,
            shortDescription: "Build",
            mode: "background",
          },
        ],
      }),
    );

    expect(
      selectUnreadNotificationCountByThread(
        { notifications: state },
        "thread-1",
      ),
    ).toBe(1);
    expect(selectProcessCompletions({ notifications: state })).toHaveLength(1);
  });

  it("dedups live events by process id regardless of seq", () => {
    let state = notificationsSlice.reducer(
      undefined,
      processCompleted(liveEvent({ seq: "5" })),
    );
    state = notificationsSlice.reducer(
      state,
      processCompleted(liveEvent({ seq: "9" })),
    );

    expect(
      selectUnreadNotificationCountByThread(
        { notifications: state },
        "thread-1",
      ),
    ).toBe(1);
  });
});
