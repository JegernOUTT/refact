import { describe, expect, it } from "vitest";
import { resolveWorkspaceFilePath } from "./resolveWorkspaceFilePath";

describe("resolveWorkspaceFilePath", () => {
  it.each([
    "/tmp/a.rs",
    "C:\\src\\a.rs",
    "C:/src/a.rs",
    "\\\\server\\share\\a.rs",
    "//server/share/a.rs",
  ])("preserves absolute path %s", (path) => {
    expect(resolveWorkspaceFilePath(path, [])).toEqual({ path });
  });
  it("prefers the explicit source worktree even with multiple projects", () => {
    expect(
      resolveWorkspaceFilePath("src/a.rs", ["/one", "/two"], "/worktree"),
    ).toEqual({ path: "/worktree/src/a.rs" });
  });
  it("uses only a sole project root", () => {
    expect(resolveWorkspaceFilePath("./src/a.rs", ["/project/"])).toEqual({
      path: "/project/src/a.rs",
    });
    expect(resolveWorkspaceFilePath("a.rs", [])).toHaveProperty("error");
    expect(resolveWorkspaceFilePath("a.rs", ["/one", "/two"])).toHaveProperty(
      "error",
    );
  });
  it("joins windows and filesystem roots", () => {
    expect(resolveWorkspaceFilePath("src/a.rs", ["C:\\repo\\"])).toEqual({
      path: "C:\\repo\\src\\a.rs",
    });
    expect(resolveWorkspaceFilePath("src/a.rs", ["/"])).toEqual({
      path: "/src/a.rs",
    });
  });
  it.each([
    "../secret",
    "src/../secret",
    "src\\..\\secret",
    "%2e%2e/secret",
    "%252e%252e/secret",
    "src%2fsecret",
    "src%ZZ",
    "C:secret",
    "https://host/file",
    "file:///tmp/a",
    "\\secret",
    "",
    ".",
    "a\u0000b",
  ])("rejects unsafe relative input %s", (path) => {
    expect(resolveWorkspaceFilePath(path, ["/project"])).toHaveProperty(
      "error",
    );
  });
  it("does not substitute a project for an invalid explicit worktree", () => {
    expect(
      resolveWorkspaceFilePath("a", ["/project"], "relative"),
    ).toHaveProperty("error");
  });
});
