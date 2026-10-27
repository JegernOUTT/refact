import { http, HttpResponse } from "msw";
import { screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import { server } from "../../../../utils/mockServer";
import { render } from "../../../../utils/test-utils";
import type { GitFileResponse } from "../../../../services/refact/gitRead";
import { selectGitDiffViewerPrefs } from "../gitPanelSlice";
import { GitDiffViewer } from "./GitDiffViewer";
import type { GitDiffDescriptor } from "./gitDiffDescriptor";

type FileKey = `${string}@${string}`;

function file(
  rev: string,
  path: string,
  content: string,
  extra: Partial<GitFileResponse> = {},
): GitFileResponse {
  return {
    root: "/repo",
    path,
    rev,
    exists: true,
    binary: false,
    truncated: false,
    size: content.length,
    content,
    ...extra,
  };
}

function serveFiles(
  files: Record<
    FileKey,
    GitFileResponse | { status: number; body: Record<string, string> }
  >,
  calls: string[] = [],
) {
  server.use(
    http.get("*/v1/git/file", ({ request }) => {
      const params = new URL(request.url).searchParams;
      const key = `${params.get("rev")}@${params.get("path")}` as FileKey;
      calls.push(key);
      const entry = files[key] as
        | GitFileResponse
        | { status: number; body: Record<string, string> }
        | undefined;
      if (!entry) {
        return HttpResponse.json(
          { code: "not_found", error: key },
          { status: 404 },
        );
      }
      if ("status" in entry && "body" in entry) {
        return HttpResponse.json(entry.body, { status: entry.status });
      }
      return HttpResponse.json(entry);
    }),
  );
  return calls;
}

const modified: GitDiffDescriptor = {
  root: "/repo",
  path: "src/app.ts",
  old_path: null,
  left: { rev: "HEAD", path: "src/app.ts" },
  right: { rev: "WORKTREE", path: "src/app.ts" },
  kind: "working",
};

beforeEach(() => {
  vi.spyOn(Element.prototype, "scrollIntoView").mockImplementation(
    () => undefined,
  );
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("GitDiffViewer", () => {
  test("renders a side-by-side diff with labelled panes and a difference count", async () => {
    serveFiles({
      "HEAD@src/app.ts": file("HEAD", "src/app.ts", "const a = 1;\nkeep();\n"),
      "WORKTREE@src/app.ts": file(
        "WORKTREE",
        "src/app.ts",
        "const a = 2;\nkeep();\nadded();\n",
      ),
    });
    render(<GitDiffViewer descriptor={modified} />);

    expect(
      await screen.findByRole("textbox", {
        name: "HEAD version of src/app.ts",
      }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("textbox", {
        name: "Working tree version of src/app.ts",
      }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("toolbar", { name: "Diff viewer toolbar" }),
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByText(/\d+ differences?/)).toBeInTheDocument(),
    );
    expect(
      screen.getByRole("button", { name: "Next Difference (F7)" }),
    ).toBeEnabled();
    expect(screen.getByText("HEAD")).toBeInTheDocument();
    expect(screen.getByText("Working tree")).toBeInTheDocument();
  });

  test("toggles to unified mode and remembers the choice", async () => {
    serveFiles({
      "HEAD@src/app.ts": file("HEAD", "src/app.ts", "one\n"),
      "WORKTREE@src/app.ts": file("WORKTREE", "src/app.ts", "two\n"),
    });
    const { store, user } = render(<GitDiffViewer descriptor={modified} />);
    await screen.findByRole("textbox", { name: "HEAD version of src/app.ts" });

    await user.click(screen.getByRole("radio", { name: "Unified" }));

    expect(selectGitDiffViewerPrefs(store.getState()).mode).toBe("unified");
    await waitFor(() =>
      expect(
        screen.queryByRole("textbox", { name: "HEAD version of src/app.ts" }),
      ).not.toBeInTheDocument(),
    );
    expect(
      screen.getByRole("textbox", {
        name: "Working tree version of src/app.ts",
      }),
    ).toBeInTheDocument();
  });

  test("shows a new-file banner when the left side is missing", async () => {
    serveFiles({
      "WORKTREE@src/new.ts": file("WORKTREE", "src/new.ts", "fresh();\n"),
    });
    render(
      <GitDiffViewer
        descriptor={{
          ...modified,
          path: "src/new.ts",
          left: null,
          right: { rev: "WORKTREE", path: "src/new.ts" },
        }}
      />,
    );
    expect(await screen.findByText("New file")).toBeInTheDocument();
  });

  test("shows a deleted-file banner and disables Jump to Source", async () => {
    serveFiles({
      "HEAD@src/old.ts": file("HEAD", "src/old.ts", "gone();\n"),
    });
    render(
      <GitDiffViewer
        descriptor={{
          ...modified,
          path: "src/old.ts",
          left: { rev: "HEAD", path: "src/old.ts" },
          right: null,
        }}
      />,
    );
    expect(await screen.findByText("Deleted file")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Jump to Source (F4)" }),
    ).toBeDisabled();
  });

  test("reports identical contents", async () => {
    serveFiles({
      "HEAD@src/app.ts": file("HEAD", "src/app.ts", "same\n"),
      "WORKTREE@src/app.ts": file("WORKTREE", "src/app.ts", "same\n"),
    });
    render(<GitDiffViewer descriptor={modified} />);
    expect(
      await screen.findByText("Contents are identical"),
    ).toBeInTheDocument();
    expect(screen.getByText("0 differences")).toBeInTheDocument();
  });

  test("renders binary files with sizes", async () => {
    serveFiles({
      "HEAD@src/app.ts": file("HEAD", "src/app.ts", "", {
        binary: true,
        size: 2048,
      }),
      "WORKTREE@src/app.ts": file("WORKTREE", "src/app.ts", "", {
        binary: true,
        size: 4096,
      }),
    });
    render(<GitDiffViewer descriptor={modified} />);
    expect(await screen.findByText("Binary files differ")).toBeInTheDocument();
    expect(screen.getByText("2.0 KiB → 4.0 KiB")).toBeInTheDocument();
  });

  test("shows a truncation banner", async () => {
    serveFiles({
      "HEAD@src/app.ts": file("HEAD", "src/app.ts", "a\n", { truncated: true }),
      "WORKTREE@src/app.ts": file("WORKTREE", "src/app.ts", "b\n"),
    });
    render(<GitDiffViewer descriptor={modified} />);
    expect(
      await screen.findByText(/File is larger than 2 MiB/),
    ).toBeInTheDocument();
  });

  test("renders friendly git errors", async () => {
    serveFiles({
      "HEAD@src/app.ts": {
        status: 500,
        body: { code: "git_unavailable", error: "" },
      },
      "WORKTREE@src/app.ts": file("WORKTREE", "src/app.ts", "b\n"),
    });
    render(<GitDiffViewer descriptor={modified} />);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Git is not available for this workspace.",
    );
  });

  test("labels commit sides with short oids and renames", async () => {
    const commit = { oid: "c0ffee1234567890", short_oid: "c0ffee1" };
    serveFiles({
      "abc1234@src/prev.ts": file("abc1234", "src/prev.ts", "x\n"),
      [`${commit.oid}@src/next.ts`]: file(commit.oid, "src/next.ts", "y\n"),
    });
    render(
      <GitDiffViewer
        descriptor={{
          root: "/repo",
          path: "src/next.ts",
          old_path: "src/prev.ts",
          left: { rev: "abc1234", path: "src/prev.ts" },
          right: { rev: commit.oid, path: "src/next.ts" },
          kind: "commit",
          commit,
        }}
      />,
    );
    expect(
      await screen.findByRole("textbox", {
        name: "c0ffee1 version of src/next.ts",
      }),
    ).toBeInTheDocument();
    expect(screen.getByText("src/prev.ts → src/next.ts")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Jump to Source (F4)" }),
    ).toBeDisabled();
  });
});
