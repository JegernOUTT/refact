import { http, HttpResponse } from "msw";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { setProjectStorageNamespace } from "../../../utils/chatUiPersistence";
import { FilesPanel } from "./FilesPanel";
import { FileViewer } from "./FileViewer";
import { openFileInFilesPanel } from "./filesPanelSlice";

const rootPath = "/workspace";
const sourcePath = `${rootPath}/src`;
const filePath = `${sourcePath}/main.ts`;

const treeResponse = (path: string, entries: unknown[]) => ({
  path,
  entries,
  truncated: false,
});

const rootHandler = () =>
  http.get("*/v1/files/tree", ({ request }) => {
    const path = new URL(request.url).searchParams.get("path") ?? "";
    if (path === "") {
      return HttpResponse.json(
        treeResponse("", [
          { name: "workspace", path: rootPath, kind: "dir", size: null },
        ]),
      );
    }
    if (path === rootPath) {
      return HttpResponse.json(
        treeResponse(rootPath, [
          { name: "src", path: sourcePath, kind: "dir", size: null },
          {
            name: "README.md",
            path: `${rootPath}/README.md`,
            kind: "file",
            size: 4,
          },
        ]),
      );
    }
    if (path === sourcePath) {
      return HttpResponse.json(
        treeResponse(sourcePath, [
          { name: "main.ts", path: filePath, kind: "file", size: 22 },
        ]),
      );
    }
    return HttpResponse.json(treeResponse(path, []));
  });

const readResponse = (overrides: Record<string, unknown> = {}) => ({
  path: filePath,
  content: "const first = 1;\nconst second = 2;\n",
  language: "typescript",
  size: 34,
  truncated: false,
  line_start: null,
  line_end: null,
  mtime_ms: 1,
  ...overrides,
});

describe("FilesPanel", () => {
  beforeEach(() => {
    setProjectStorageNamespace("files-panel-test");
    vi.spyOn(Element.prototype, "scrollIntoView").mockImplementation(
      () => undefined,
    );
  });

  afterEach(() => {
    localStorage.clear();
    setProjectStorageNamespace(undefined);
    vi.restoreAllMocks();
  });

  it("fetches an expanded directory once and reuses its cached children", async () => {
    let sourceRequests = 0;
    server.use(
      http.get("*/v1/files/tree", ({ request }) => {
        const path = new URL(request.url).searchParams.get("path") ?? "";
        if (path === sourcePath) sourceRequests += 1;
        if (path === "") {
          return HttpResponse.json(
            treeResponse("", [
              { name: "workspace", path: rootPath, kind: "dir", size: null },
            ]),
          );
        }
        if (path === rootPath) {
          return HttpResponse.json(
            treeResponse(rootPath, [
              { name: "src", path: sourcePath, kind: "dir", size: null },
            ]),
          );
        }
        return HttpResponse.json(
          treeResponse(sourcePath, [
            { name: "main.ts", path: filePath, kind: "file", size: 22 },
          ]),
        );
      }),
    );

    const { user } = render(<FilesPanel />);
    await user.click(
      await screen.findByRole("treeitem", { name: /workspace/i }),
    );
    const source = await screen.findByRole("treeitem", { name: /src/i });
    await user.click(source);
    expect(
      await screen.findByRole("treeitem", { name: /main\.ts/i }),
    ).toBeVisible();
    expect(sourceRequests).toBe(1);

    await user.click(source);
    await waitFor(() => {
      expect(screen.queryByRole("treeitem", { name: /main\.ts/i })).toBeNull();
    });
    await user.click(source);
    expect(
      await screen.findByRole("treeitem", { name: /main\.ts/i }),
    ).toBeVisible();
    expect(sourceRequests).toBe(1);
  });

  it("renders file content and highlights the requested line", async () => {
    server.use(
      rootHandler(),
      http.get("*/v1/files/read", () => HttpResponse.json(readResponse())),
    );
    const view = render(<FileViewer path={filePath} />);
    view.store.dispatch(openFileInFilesPanel({ path: filePath, line: 2 }));

    expect(await screen.findByText("const second = 2;")).toBeVisible();
    expect(document.querySelector('[data-target-line="true"]')).toHaveAttribute(
      "data-line-number",
      "2",
    );
    expect(
      screen.getByRole("navigation", { name: "File path" }),
    ).toHaveTextContent("workspace/src/main.ts");
  });

  it("shows an honest privacy-blocked state", async () => {
    server.use(
      rootHandler(),
      http.get(
        "*/v1/files/read",
        () => new HttpResponse(null, { status: 403 }),
      ),
    );
    const view = render(<FileViewer path={filePath} />);
    view.store.dispatch(openFileInFilesPanel({ path: filePath }));

    expect(await screen.findByText("File blocked")).toBeVisible();
    expect(
      screen.getByText("This file is blocked by privacy rules."),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "Retry" })).toBeVisible();
  });

  it("shows the truncation banner returned by the backend", async () => {
    server.use(
      rootHandler(),
      http.get("*/v1/files/read", () =>
        HttpResponse.json(readResponse({ truncated: true })),
      ),
    );
    const view = render(<FileViewer path={filePath} />);
    view.store.dispatch(openFileInFilesPanel({ path: filePath }));

    expect(await screen.findByText("File truncated at 1 MiB")).toBeVisible();
  });

  it("identifies binary files without rendering an empty code view", async () => {
    server.use(
      rootHandler(),
      http.get("*/v1/files/read", () =>
        HttpResponse.json(readResponse({ binary: true, content: "" })),
      ),
    );
    const view = render(<FileViewer path={filePath} />);
    view.store.dispatch(openFileInFilesPanel({ path: filePath }));

    expect(await screen.findByText("Binary file")).toBeVisible();
    expect(screen.getByText(/cannot be previewed/)).toBeVisible();
  });

  it("keeps keyboard navigation in the tree and opens a file with Enter", async () => {
    server.use(
      rootHandler(),
      http.get("*/v1/files/read", () => HttpResponse.json(readResponse())),
    );
    const { user, store } = render(<FilesPanel />);
    const tree = await screen.findByRole("tree", { name: "Workspace files" });
    const workspace = await screen.findByRole("treeitem", {
      name: /workspace/i,
    });
    await user.click(workspace);
    await screen.findByRole("treeitem", { name: /README\.md/i });

    tree.focus();
    fireEvent.keyDown(tree, { key: "ArrowDown" });
    fireEvent.keyDown(tree, { key: "ArrowDown" });
    fireEvent.keyDown(tree, { key: "Enter" });

    await waitFor(() =>
      expect(store.getState().workspace.activeTabId).toBe(
        "file:/workspace/README.md",
      ),
    );
    expect(
      within(tree).getByRole("treeitem", { name: /README\.md/i }),
    ).toHaveAttribute("aria-selected", "true");
  });
});
