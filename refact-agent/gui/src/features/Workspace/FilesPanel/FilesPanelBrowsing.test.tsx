import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";

import { render, screen, waitFor } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { FilesPanel } from "./FilesPanel";

const tree = (path: string, entries: unknown[]) => ({
  path,
  entries,
  truncated: false,
});
const file = (path: string) => ({
  path,
  name: path.split("/").at(-1),
  kind: "file",
  size: 4,
});

describe("explicit folder browsing", () => {
  it("browses root and descendants through POST, opens external files manually, and restores the worktree", async () => {
    const requests: string[] = [];
    server.use(
      http.get("*/v1/privacy/policy", () =>
        HttpResponse.json({ policy: { zones: [] }, destinations: [] }),
      ),
      http.get("*/v1/files/tree", ({ request }) => {
        const path = new URL(request.url).searchParams.get("path") ?? "";
        requests.push(`GET:${path}`);
        return HttpResponse.json(tree(path, [file("/worktree/current.txt")]));
      }),
      http.post("*/v1/files/browse", async ({ request }) => {
        const { path } = (await request.json()) as { path: string };
        requests.push(`POST:${path}`);
        return HttpResponse.json(
          tree(
            path,
            path === "/"
              ? [{ path: "/outside", name: "outside", kind: "dir", size: null }]
              : [file("/outside/notes.txt")],
          ),
        );
      }),
    );
    const { user, store } = render(<FilesPanel />, {
      preloadedState: {
        current_project: { name: "worktree", workspaceRoots: ["/worktree"] },
      },
    });
    expect(
      await screen.findByRole("treeitem", { name: "current.txt" }),
    ).toBeVisible();
    await user.type(
      screen.getByRole("textbox", { name: "Absolute file path" }),
      "/",
    );
    await user.click(screen.getByRole("button", { name: "Browse folder" }));
    await user.click(await screen.findByRole("treeitem", { name: "outside" }));
    await user.click(
      await screen.findByRole("treeitem", { name: "notes.txt" }),
    );
    expect(store.getState().filesPanel.viewerTarget).toEqual({
      path: "/outside/notes.txt",
      manual: true,
    });
    expect(requests).toContain("POST:/");
    expect(requests).toContain("POST:/outside");
    expect(requests).not.toContain("GET:/outside");
    expect(store.getState().current_project.workspaceRoots).toEqual([
      "/worktree",
    ]);
    expect(
      screen.getByRole("button", { name: "Parent folder" }),
    ).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Current worktree" }));
    await waitFor(() =>
      expect(
        screen.getByRole("treeitem", { name: "current.txt" }),
      ).toBeVisible(),
    );
    expect(screen.queryByRole("treeitem", { name: "notes.txt" })).toBeNull();
  });
});
