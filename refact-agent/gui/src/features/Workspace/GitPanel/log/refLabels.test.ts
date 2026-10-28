import { describe, expect, test } from "vitest";

import type {
  GitBranchesRoot,
  GitRef,
} from "../../../../services/refact/gitRead";
import {
  buildRefLabels,
  groupRefLabels,
  refLabelContext,
  type RefLabelContext,
} from "./refLabels";

const ATTACHED: RefLabelContext = {
  remotes: ["origin", "upstream"],
  currentBranch: "main",
  headDetached: false,
};

const DETACHED: RefLabelContext = {
  remotes: ["origin"],
  currentBranch: null,
  headDetached: true,
};

const head: GitRef = { name: "HEAD", kind: "head" };
const local = (name: string): GitRef => ({ name, kind: "local" });
const remote = (name: string): GitRef => ({ name, kind: "remote" });
const tag = (name: string): GitRef => ({ name, kind: "tag" });

const texts = (refs: GitRef[], context: RefLabelContext) =>
  buildRefLabels(refs, context).map((label) => label.text);

describe("buildRefLabels", () => {
  test("merges a local branch with its remote-tracking branch", () => {
    const labels = buildRefLabels(
      [local("feature/log"), remote("origin/feature/log")],
      ATTACHED,
    );
    expect(labels).toHaveLength(1);
    expect(labels[0]).toMatchObject({
      text: "origin & feature/log",
      kind: "local",
      names: ["feature/log", "origin/feature/log"],
      current: false,
    });
  });

  test("lists every remote before the branch name", () => {
    expect(
      texts(
        [remote("upstream/main"), local("main"), remote("origin/main")],
        ATTACHED,
      ),
    ).toEqual(["origin & upstream & main"]);
    expect(
      texts([remote("origin/dev"), remote("upstream/dev")], ATTACHED),
    ).toEqual(["origin & upstream & dev"]);
  });

  test("keeps a lone remote branch and unknown prefixes as full names", () => {
    expect(texts([remote("origin/dev"), remote("fork/dev")], ATTACHED)).toEqual(
      ["origin/dev", "fork/dev"],
    );
  });

  test("hides HEAD when attached and marks the current branch", () => {
    const labels = buildRefLabels(
      [head, local("main"), remote("origin/main"), local("dev")],
      ATTACHED,
    );
    expect(labels.map((label) => label.text)).toEqual(["origin & main", "dev"]);
    expect(labels[0].current).toBe(true);
    expect(labels[1].current).toBe(false);
  });

  test("keeps the HEAD pill when HEAD is detached", () => {
    const labels = buildRefLabels(
      [head, local("main"), remote("origin/main")],
      DETACHED,
    );
    expect(labels.map((label) => [label.kind, label.text])).toEqual([
      ["head", "HEAD"],
      ["local", "origin & main"],
    ]);
    expect(labels.some((label) => label.current)).toBe(false);
  });

  test("keeps the HEAD pill when the current branch is not on the commit", () => {
    expect(texts([head, local("dev")], ATTACHED)).toEqual(["HEAD", "dev"]);
  });

  test("leaves tags as separate pills", () => {
    const labels = buildRefLabels(
      [tag("v1.0"), local("main"), tag("v0.9")],
      ATTACHED,
    );
    expect(labels.map((label) => [label.kind, label.text])).toEqual([
      ["local", "main"],
      ["tag", "v0.9"],
      ["tag", "v1.0"],
    ]);
  });
});

describe("groupRefLabels", () => {
  test("applies the overflow limit after merging", () => {
    const { visible, hidden } = groupRefLabels(
      [
        head,
        local("main"),
        remote("origin/main"),
        remote("upstream/main"),
        local("dev"),
        tag("v1.0"),
        tag("v2.0"),
      ],
      ATTACHED,
    );
    expect(visible.map((label) => label.text)).toEqual([
      "origin & upstream & main",
      "dev",
      "v1.0",
    ]);
    expect(hidden.map((label) => label.text)).toEqual(["v2.0"]);
  });
});

describe("refLabelContext", () => {
  const data: GitBranchesRoot = {
    root: "/repo",
    current: "main",
    branches: [{ name: "main", is_head: true, upstream: "origin/main" }],
    head_detached: false,
    remotes: ["origin"],
    remote_branches: [
      {
        name: "upstream/main",
        remote: "upstream",
        short_name: "main",
        tip_oid: null,
      },
    ],
  };

  test("collects remotes and the attached branch", () => {
    expect(refLabelContext(data)).toEqual({
      remotes: ["origin", "upstream"],
      currentBranch: "main",
      headDetached: false,
    });
  });

  test("drops the current branch when HEAD is detached", () => {
    expect(
      refLabelContext({ ...data, current: null, head_detached: true }),
    ).toMatchObject({ currentBranch: null, headDetached: true });
  });
});
