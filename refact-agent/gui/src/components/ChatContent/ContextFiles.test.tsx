import { afterEach, describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";

import { render, screen, waitFor } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { ContextFiles } from "./ContextFiles";
import type { ChatContextFile } from "../../services/refact";
import type { Config } from "../../features/Config/configSlice";

const filePath = "/workspace/src/foo.ts";

const files: ChatContextFile[] = [
  {
    file_name: filePath,
    file_content: "const a = 1;",
    line1: 3,
    line2: 10,
  },
];

function makeConfig(host: Config["host"], overrides?: Config["capabilities"]) {
  const config: Config = {
    host,
    lspPort: 8001,
    themeProps: { appearance: "dark" },
  };
  if (overrides) config.capabilities = overrides;
  return config;
}

function renderContextFiles(config: Config) {
  return render(
    <ContextFiles files={files} open={true} onOpenChange={() => undefined} />,
    { preloadedState: { config } },
  );
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("ContextFiles file clicks", () => {
  it("opens the clicked file in the Files panel on the web host", async () => {
    const { user, store } = renderContextFiles(makeConfig("web"));

    const items = screen.getAllByText("foo.ts:3-10");
    await user.click(items[items.length - 1]);

    expect(store.getState().workspace.tabs).toContain("files:main");
    expect(store.getState().filesPanel.viewerTarget).toEqual({
      path: filePath,
      line: 3,
    });
  });

  it("posts ide/openFile for the clicked file on the vscode host", async () => {
    server.use(
      http.post("*/v1/fullpath", () =>
        HttpResponse.json({ fullpath: filePath, is_directory: false }),
      ),
    );
    const postMessageSpy = vi.spyOn(window, "postMessage");
    const { user, store } = renderContextFiles(makeConfig("vscode"));

    const items = screen.getAllByText("foo.ts:3-10");
    await user.click(items[items.length - 1]);

    await waitFor(() => {
      expect(postMessageSpy).toHaveBeenCalledWith(
        expect.objectContaining({
          type: "ide/openFile",
          payload: { file_path: filePath, line: 3 },
        }),
        "*",
      );
    });
    expect(store.getState().filesPanel.viewerTarget).toBeNull();
  });

  it("renders file names as inert text when no open capability exists", async () => {
    const postMessageSpy = vi.spyOn(window, "postMessage");
    const { user, store } = renderContextFiles(
      makeConfig("web", { openFileInApp: false, openFileInIde: false }),
    );

    const items = screen.getAllByText("foo.ts:3-10");
    await user.click(items[items.length - 1]);

    expect(store.getState().filesPanel.viewerTarget).toBeNull();
    expect(postMessageSpy).not.toHaveBeenCalledWith(
      expect.objectContaining({ type: "ide/openFile" }),
      "*",
    );
  });
});
