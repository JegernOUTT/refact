import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";
import { cleanup, waitFor } from "@testing-library/react";
import { setFileInfo } from "../features/Chat/activeFile";
import { setSelectedSnippet } from "../features/Chat/selectedSnippet";
import { useEventBusForApp } from "../hooks/useEventBusForApp";
import { usePostUserAction } from "../hooks/usePostUserAction";
import { server } from "../utils/mockServer";
import { postMessage, render } from "../utils/test-utils";

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

function EventBusHarness() {
  useEventBusForApp();
  return null;
}

function HookHarness() {
  const { postFileOpened } = usePostUserAction();
  return (
    <button onClick={() => postFileOpened("/workspace/src/fail.rs")}>
      post
    </button>
  );
}

describe("usePostUserAction", () => {
  it("usePostUserAction_called_on_setFileInfo", async () => {
    let requestBody: unknown;
    server.use(
      http.post("*/v1/buddy/user_action", async ({ request }) => {
        requestBody = await request.json();
        return HttpResponse.text("OK");
      }),
    );

    render(<EventBusHarness />, { preloadedState: CONFIG_STATE });
    postMessage(
      setFileInfo({
        name: "main.rs",
        line1: 1,
        line2: 10,
        can_paste: true,
        path: "/workspace/src/main.rs",
        cursor: 3,
      }),
    );

    await waitFor(() => {
      expect(requestBody).toMatchObject({
        type: "file_opened",
        path: "/workspace/src/main.rs",
      });
    });
  });

  it("usePostUserAction_called_on_setSelectedSnippet", async () => {
    let requestBody: unknown;
    server.use(
      http.post("*/v1/buddy/user_action", async ({ request }) => {
        requestBody = await request.json();
        return HttpResponse.text("OK");
      }),
    );

    render(<EventBusHarness />, { preloadedState: CONFIG_STATE });
    postMessage(
      setSelectedSnippet({
        language: "rust",
        code: "fn main() {}",
        path: "/workspace/src/lib.rs",
        basename: "lib.rs",
        start_line: 5,
        end_line: 7,
      }),
    );

    await waitFor(() => {
      expect(requestBody).toMatchObject({
        type: "snippet_selected",
        path: "/workspace/src/lib.rs",
        lines: [5, 7],
      });
    });
  });

  it("postUserAction_silently_ignores_failures", async () => {
    cleanup();
    let called = false;
    server.use(
      http.post("*/v1/buddy/user_action", () => {
        called = true;
        return HttpResponse.error();
      }),
    );
    const unhandledRejection = vi.fn();
    window.addEventListener("unhandledrejection", unhandledRejection);

    const { user } = render(<HookHarness />, { preloadedState: CONFIG_STATE });
    const button = document.querySelector("button");
    if (!button) throw new Error("button not found");
    await expect(user.click(button)).resolves.toBeUndefined();

    await waitFor(() => {
      expect(called).toBe(true);
    });
    expect(unhandledRejection).not.toHaveBeenCalled();
    window.removeEventListener("unhandledrejection", unhandledRejection);
  });
});
