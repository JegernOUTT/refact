import { screen, waitFor } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { useEffect } from "react";
import { describe, expect, test, vi } from "vitest";

import { server } from "../../../utils/mockServer";
import { render } from "../../../utils/test-utils";
import { TerminalPanel } from "./TerminalPanel";

vi.hoisted(() => vi.resetModules());
vi.mock("./TerminalSession", () => ({
  TerminalSession: StubTerminalSession,
}));

function StubTerminalSession({
  processId,
  chatId,
  onResize,
}: {
  processId: string;
  chatId: string;
  onResize?: (processId: string, rows: number, cols: number) => void;
}) {
  useEffect(() => {
    expect(chatId).toBe("chat-a");
    onResize?.(processId, 48, 132);
  }, [chatId, onResize, processId]);
  return null;
}

const CONFIG_STATE = {
  config: {
    host: "web" as const,
    lspPort: 8001,
    apiKey: null,
    themeProps: {},
  },
};

describe("TerminalPanel spawn dimensions", () => {
  test("spawn body carries the current fitted rows and cols", async () => {
    const spawnBodies: unknown[] = [];
    server.use(
      http.get("*/v1/exec/list", () =>
        HttpResponse.json({
          processes: [
            {
              process_id: "seed-1234567",
              status: "running",
              command_preview: "/bin/zsh",
              created_at_ms: 1,
              tty: true,
              service_name: null,
            },
          ],
        }),
      ),
      http.post("*/v1/exec/spawn", async ({ request }) => {
        spawnBodies.push(await request.json());
        return HttpResponse.json({
          process_id: "spawned-9999",
          status: "running",
          command_preview: "/bin/zsh",
        });
      }),
    );

    const { user, store } = render(<TerminalPanel chatId="chat-a" />, {
      preloadedState: {
        ...CONFIG_STATE,
        current_project: { name: "workspace", workspaceRoots: ["/project"] },
      },
    });
    const [{ createChatWithId }, { openTab }, { makeSurfaceKey }] =
      await Promise.all([
        import("../../Chat/Thread"),
        import("../workspaceSlice"),
        import("../surfaceKey"),
      ]);
    store.dispatch(
      createChatWithId({
        id: "chat-a",
        worktree: {
          id: "worktree-a",
          kind: "task_agent",
          root: "/worktrees/chat-a",
          source_workspace_root: "/project",
          repo_root: "/project",
          enforce: true,
        },
      }),
    );
    store.dispatch(openTab(makeSurfaceKey("chat", "chat-a")));
    const seedTab = await screen.findByRole("tab", {
      name: /\/bin\/zsh · seed-123/i,
    });
    await user.click(seedTab);
    expect(seedTab).toHaveAttribute("aria-expanded", "true");

    await user.click(screen.getByRole("button", { name: "New terminal" }));

    await waitFor(() => expect(spawnBodies).toHaveLength(1));
    expect(spawnBodies).toEqual([
      {
        chat_id: "chat-a",
        cwd: "/worktrees/chat-a",
        env: { TERM: "xterm-256color", COLORTERM: "truecolor", NO_COLOR: "" },
        pty: true,
        rows: 48,
        cols: 132,
      },
    ]);
  });
});
