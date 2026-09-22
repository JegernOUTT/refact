import { afterEach, describe, it, expect } from "vitest";
import { renderHook } from "@testing-library/react";
import { Provider } from "react-redux";
import { setUpStore } from "../app/store";
import { useBuddyWorkDeepLink } from "./useBuddyWorkDeepLink";

describe("Buddy work deep links", () => {
  afterEach(() => window.history.replaceState(null, "", "/"));
  it("waits for readiness and opens the project-bound work without loading a worker chat", () => {
    window.history.replaceState(
      null,
      "",
      "/?buddy_work=work-1&buddy_project=project-1",
    );
    const store = setUpStore({
      config: {
        host: "web",
        engineServed: true,
        lspPort: 8001,
        themeProps: {},
      },
    });
    const { rerender } = renderHook(
      ({ ready }) => useBuddyWorkDeepLink(ready),
      {
        initialProps: { ready: false },
        wrapper: ({ children }) => (
          <Provider store={store}>{children}</Provider>
        ),
      },
    );
    expect(store.getState().pages.at(-1)?.name).not.toBe("buddy");
    rerender({ ready: true });
    expect(store.getState().pages.at(-1)).toEqual({
      name: "buddy",
      workId: "work-1",
      workProjectId: "project-1",
    });
    expect(window.location.search).toContain("buddy_work=work-1");
  });
  it.each(["vscode", "jetbrains"] as const)(
    "leaves %s host navigation unchanged",
    (host) => {
      window.history.replaceState(
        null,
        "",
        "/?buddy_work=work-1&buddy_project=project-1",
      );
      const store = setUpStore({
        config: { host, engineServed: true, lspPort: 8001, themeProps: {} },
      });
      renderHook(() => useBuddyWorkDeepLink(true), {
        wrapper: ({ children }) => (
          <Provider store={store}>{children}</Provider>
        ),
      });
      expect(store.getState().pages.at(-1)?.name).not.toBe("buddy");
    },
  );
});
