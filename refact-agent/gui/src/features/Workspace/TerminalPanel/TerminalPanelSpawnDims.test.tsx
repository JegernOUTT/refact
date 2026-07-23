import { screen, waitFor } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { useEffect } from "react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import { server } from "../../../utils/mockServer";

type TerminalPanelComponent = typeof import("./TerminalPanel").TerminalPanel;
type RenderFn = typeof import("../../../utils/test-utils").render;

function StubTerminalSession({
  processId,
  onResize,
}: {
  processId: string;
  onResize?: (processId: string, rows: number, cols: number) => void;
}) {
  useEffect(() => {
    onResize?.(processId, 48, 132);
  }, [onResize, processId]);
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

let TerminalPanel: TerminalPanelComponent;
let render: RenderFn;

beforeEach(async () => {
  vi.resetModules();
  vi.doMock("./TerminalSession", () => ({
    TerminalSession: StubTerminalSession,
  }));
  ({ TerminalPanel } = await import("./TerminalPanel"));
  ({ render } = await import("../../../utils/test-utils"));
});

afterEach(() => {
  vi.doUnmock("./TerminalSession");
});

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

    const { user } = render(<TerminalPanel />, {
      preloadedState: CONFIG_STATE,
    });
    await screen.findByRole("tab", { name: /\/bin\/zsh · seed-123/i });

    await user.click(screen.getByRole("button", { name: "New terminal" }));

    await waitFor(() => expect(spawnBodies).toHaveLength(1));
    expect(spawnBodies).toEqual([{ pty: true, rows: 48, cols: 132 }]);
  });
});
