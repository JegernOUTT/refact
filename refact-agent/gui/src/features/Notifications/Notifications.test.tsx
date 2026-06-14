import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
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

function readGuiSource(path: string): Promise<string> {
  return readFile(resolve(process.cwd(), "src", path), "utf8");
}

function readCssBlock(source: string, selector: string): string {
  const escapedSelector = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = new RegExp(`(^|\\n)\\s*${escapedSelector}\\s*{`).exec(source);
  if (match?.index === undefined) {
    throw new Error(`Missing CSS block for ${selector}`);
  }
  const start = source.indexOf("{", match.index);
  const end = source.indexOf("\n}", start);
  if (start === -1 || end === -1) {
    throw new Error(`Malformed CSS block for ${selector}`);
  }
  return source.slice(start + 1, end);
}

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

  it("keeps internal padding on the toast surface", async () => {
    const css = await readGuiSource("features/Notifications/Toast.module.css");
    const toastBlock = readCssBlock(css, ".toast");

    expect(toastBlock).toContain("padding: var(--rf-panel-pad)");
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
