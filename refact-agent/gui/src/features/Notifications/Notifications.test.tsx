import { describe, expect, it } from "vitest";
import { act } from "react-dom/test-utils";

import { render, screen } from "../../utils/test-utils";
import type { ProcessCompletedEvent } from "./notificationsSlice";
import {
  notificationsSlice,
  processCompleted,
  selectUnreadNotificationCountByThread,
} from "./notificationsSlice";
import { ProcessCompletedToasts } from "./Toast";
import { switchToThread } from "../Chat/Thread";

function makeProcessCompletedEvent(
  overrides: Partial<ProcessCompletedEvent> = {},
): ProcessCompletedEvent {
  return {
    chat_id: "thread-1",
    seq: "7",
    type: "process_completed",
    process_id: "exec_done",
    status: "exited",
    exit_code: 0,
    short_description: "Build background worker",
    mode: "background",
    ...overrides,
  };
}

describe("ProcessCompleted notifications", () => {
  it("renders a toast when a ProcessCompleted event is dispatched", async () => {
    const { store } = render(<ProcessCompletedToasts />);

    act(() => {
      store.dispatch(processCompleted(makeProcessCompletedEvent()));
    });

    expect(await screen.findByTestId("process-completed-toast")).toBeVisible();
    expect(screen.getByText("Build background worker")).toBeInTheDocument();
    expect(screen.getByText("exit 0")).toBeInTheDocument();
    expect(screen.getByText("exec_done")).toBeInTheDocument();
    expect(screen.getByText("✅")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "View" })).toBeInTheDocument();
  });

  it("clears pending notifications when switching to the thread", () => {
    let state = notificationsSlice.reducer(
      undefined,
      processCompleted(makeProcessCompletedEvent()),
    );

    expect(
      selectUnreadNotificationCountByThread(
        { notifications: state },
        "thread-1",
      ),
    ).toBe(1);

    state = notificationsSlice.reducer(
      state,
      switchToThread({ id: "thread-1" }),
    );

    expect(
      selectUnreadNotificationCountByThread(
        { notifications: state },
        "thread-1",
      ),
    ).toBe(0);
  });
});
