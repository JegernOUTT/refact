import { expect, test, type Page } from "@playwright/test";

const BUDDY_ROUTE = "/tests/e2e/route-showcase.html?route=buddy";
const ENABLED_ROUTE = `${BUDDY_ROUTE}&operator=enabled`;
const UNAVAILABLE_ROUTE = `${BUDDY_ROUTE}&operator=unavailable`;
const DESK = "[aria-label='Buddy Desk']";
const FOLD_PX = 900;
const NARROW_PX = 360;

const project = { project_id: "showcase", canonical_root: "/workspace/refact" };

const NO_ACTION_PHRASES = [
  "No action to take",
  "No result yet",
  "Action status unavailable",
];

const ENGINE_JARGON = [
  "not execution authority",
  "no verified effects claimed",
  "legacy_work_route_read_only",
  "proposed_action_ids",
  "contains_untrusted_source",
  "needs_attention",
];

const REASSURING_EMPTY_COPY = [
  "No pending questions were found",
  "No approvals or warnings found",
];

const VANITY_STAT_LABELS = ["Care", "Neglect", "Messages", "Tokens", "Success"];

const BARE_UUID =
  /\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b/i;

type Seed = {
  id: string;
  title: string;
  state: "needs_attention" | "blocked" | "watching";
  kind: string;
  subjectKind: string;
  subject: string;
  artifact: string | null;
  minutesAgo: number;
  action: { label: string; description: string } | null;
  actionFails?: boolean;
  conclusion?: string;
  body?: string;
  uncertainty?: string | null;
};

const LONG_TITLE =
  "Review the long-running diagnostic cluster that keeps reopening after every " +
  "restart and decide whether the browser controller or the provider retry loop " +
  "is the thing that actually needs changing";

// Modelled on a real store: duplicated producers folded upstream, persona
// chatter demoted below the headline, run logs mixed in with real work, an
// item whose only durable identifier is a chat UUID, and rows that have
// nothing to offer.
const SEEDS: Seed[] = [
  {
    id: "garden-orphans",
    title: "Knowledge base needs attention: 97 lifecycle candidate(s)",
    state: "needs_attention",
    kind: "assessment",
    subjectKind: "memory_garden",
    subject: "orphan",
    artifact: "artifact-garden",
    minutesAgo: 11,
    action: {
      label: "Tidy 97 memory entries",
      description:
        "Reads stored memory metadata only; nothing is removed without another confirmation.",
    },
    conclusion:
      "Saved from an earlier Buddy. Nothing here has been checked or run.",
    uncertainty: "Kept for reference only — Buddy cannot act on it.",
    body: [
      "Buddy found 97 saved memories that no longer point at anything.",
      "",
      "- Tidy 97 memory entries",
      "- Dismiss this finding",
    ].join("\n"),
  },
  {
    id: "garden-conflicts",
    title: "Two saved memories disagree about the release tag scheme",
    state: "needs_attention",
    kind: "assessment",
    subjectKind: "memory_garden",
    subject: "conflict",
    artifact: "artifact-conflict",
    minutesAgo: 11,
    action: {
      label: "Open both memories",
      description: "Opens the two saved entries side by side for a decision.",
    },
    conclusion: "Two entries claim different things about release tags.",
    body: "One entry says `release/v*`, the other says `engine/v*`.",
  },
  {
    id: "workspace-warmup",
    title: "Warm up this workspace",
    state: "needs_attention",
    kind: "quest",
    subjectKind: "workspace",
    subject: "warmup",
    artifact: "artifact-warmup",
    minutesAgo: 34,
    action: {
      label: "Open the workspace checklist",
      description: "Opens the saved checklist; nothing runs on its own.",
    },
    conclusion: "The workspace has never been indexed on this machine.",
    body: [
      "sunny starter: I am on the trail now.",
      "",
      "Indexing has not completed once in this checkout.",
    ].join("\n"),
  },
  {
    id: "git-uncommitted",
    title: "5681 uncommitted files",
    state: "needs_attention",
    kind: "assessment",
    subjectKind: "git_hygiene",
    subject: "uncommitted",
    artifact: "artifact-git",
    minutesAgo: 58,
    action: null,
    conclusion: "You have many uncommitted changes in this checkout.",
    body: "5681 files are modified and none of them are staged.",
  },
  {
    id: "parity-goal",
    title: "Finish multi-agent parity M1–M10",
    state: "needs_attention",
    kind: "goal",
    subjectKind: "chat",
    subject: "53ee2bb5-9a0e-4a54-9c1f-6a2f0b4d77aa",
    artifact: "artifact-parity",
    minutesAgo: 92,
    action: {
      label: "Check saved evidence links",
      description: "Checks stored metadata only; no finding is re-verified.",
    },
    conclusion: "Seven of ten parity milestones have saved evidence.",
    body: "M1–M7 have saved evidence. M8–M10 do not.",
  },
  {
    id: "provider-key",
    title: "Waiting on a provider key",
    state: "blocked",
    kind: "assessment",
    subjectKind: "provider",
    subject: "anthropic",
    artifact: null,
    minutesAgo: 140,
    action: null,
  },
  {
    id: "diagnostic-cluster",
    title: LONG_TITLE,
    state: "needs_attention",
    kind: "assessment",
    subjectKind: "diagnostic",
    subject: "browser-controller",
    artifact: "artifact-cluster",
    minutesAgo: 190,
    action: {
      label: "Read the saved write-up",
      description: "Opens the saved write-up; nothing is changed.",
    },
    conclusion:
      "The same cluster reopened after each of the last four restarts.",
    body: "Four restarts, four identical clusters.",
  },
  {
    id: "tidy",
    title: "Tidy",
    state: "needs_attention",
    kind: "assessment",
    subjectKind: "trajectory",
    subject: "clutter",
    artifact: "artifact-tidy",
    minutesAgo: 240,
    action: null,
    actionFails: true,
    conclusion: "Eighteen untitled trajectories are older than two weeks.",
    body: "Eighteen untitled trajectories are older than two weeks.",
  },
  {
    id: "memo-extraction",
    title: "memo_extraction — run outcome",
    state: "watching",
    kind: "job_finding",
    subjectKind: "job",
    subject: "memo_extraction",
    artifact: null,
    minutesAgo: 12,
    action: null,
  },
  {
    id: "error-detective",
    title: "Error Detective",
    state: "watching",
    kind: "quest",
    subjectKind: "quest",
    subject: "error_detective",
    artifact: null,
    minutesAgo: 300,
    action: null,
  },
  {
    id: "security-whisperer",
    title: "Security Whisperer",
    state: "watching",
    kind: "quest",
    subjectKind: "quest",
    subject: "security_whisperer",
    artifact: null,
    minutesAgo: 420,
    action: null,
  },
  {
    id: "play-break",
    title: "Give Pixel a play break",
    state: "watching",
    kind: "quest",
    subjectKind: "quest",
    subject: "play_break",
    artifact: null,
    minutesAgo: 600,
    action: null,
  },
];

const VIEW_STATES: Record<string, Seed["state"][]> = {
  needs_you: ["needs_attention", "blocked"],
  working_on: [],
  delivered: [],
  watching: ["watching"],
};

const NEEDS_YOU_TITLES = SEEDS.filter((seed) =>
  VIEW_STATES.needs_you.includes(seed.state),
).map((seed) => seed.title);

const WATCHING_TITLES = SEEDS.filter((seed) =>
  VIEW_STATES.watching.includes(seed.state),
).map((seed) => seed.title);

const ACTION_LABELS = SEEDS.filter((seed) => seed.action !== null).map(
  (seed) => seed.action?.label ?? "",
);

function minutes(value: number): string {
  return new Date(Date.now() - value * 60_000).toISOString();
}

function envelope(data: unknown) {
  return { schema_version: 1, project, data, next_cursor: null };
}

function record(value: unknown) {
  return { value, revision: 1, change_seq: 1 };
}

function workRecord(seed: Seed) {
  return record({
    schema_version: 1,
    id: seed.id,
    project,
    subject: {
      resource_kind: seed.subjectKind,
      resource_id: seed.subject,
      version: "1",
    },
    kind: seed.kind,
    title: seed.title,
    state: seed.state,
    native_sources: [],
    current_artifact_id: seed.artifact,
    current_artifact_revision: seed.artifact === null ? null : 1,
    last_material_change_at: minutes(seed.minutesAgo),
  });
}

function artifactRecord(seed: Seed) {
  return record({
    schema_version: 1,
    artifact_id: seed.artifact,
    revision: 1,
    project,
    source_run_ids: [`run-${seed.id}`],
    title: seed.title,
    conclusion: seed.conclusion ?? "Nothing conclusive was saved.",
    delta: "",
    body: seed.body ?? "No write-up was saved for this item.",
    uncertainty: seed.uncertainty ?? null,
    proposed_action_ids: [],
    evidence: [],
    provenance: {
      source_run_ids: [`run-${seed.id}`],
      evidence_refs: [],
      contains_untrusted_source: true,
    },
  });
}

function deskAction(seed: Seed) {
  return {
    work_id: seed.id,
    spec: {
      schema_version: 1,
      action_id: `${seed.id}-check`,
      kind: "check_saved_evidence",
      adapter: "operator.saved_evidence",
      project,
      resource: {
        resource_kind: "saved_report",
        resource_id: seed.id,
        version: "digest",
      },
      expected_version: "digest",
      parameter_ref: null,
      availability: "available",
      confirmation: "none",
      authority_class: "read_only",
      capabilities: {
        can_preview: true,
        can_execute: true,
        can_verify: false,
        can_recover: false,
        can_cancel: false,
      },
    },
    spec_hash: `${seed.id}-hash`,
    work_revision: 1,
    artifact_id: seed.artifact,
    artifact_revision: 1,
    label: seed.action?.label ?? "",
    description: seed.action?.description ?? "",
    latest_execution: null,
  };
}

const EMPTY_DECISIONS = {
  schema_version: 1,
  project_root: project.canonical_root,
  coverage: "loaded_native_sessions",
  truncated: false,
  questions: [],
};

const EMPTY_APPROVALS = {
  schema_version: 1,
  project_root: project.canonical_root,
  engine_identity: "showcase-engine",
  coverage: "loaded_native_sessions",
  truncated: false,
  warnings: [],
  sessions: [],
};

const EMPTY_NATIVE_WORK = {
  schema_version: 1,
  project_root: project.canonical_root,
  coverage: "loaded_native_sessions",
  next_cursor: null,
  sessions: [],
};

const SUPERVISION_CONTEXT = {
  schema_version: 1,
  project: null,
  worker: null,
  authenticated_owner: null,
  features: {
    storage: false,
    actions: false,
    supervision: false,
    improvement: false,
    editorial: false,
  },
};

function seedById(id: string): Seed | undefined {
  return SEEDS.find((seed) => seed.id === id);
}

function seedByArtifact(id: string): Seed | undefined {
  return SEEDS.find((seed) => seed.artifact === id);
}

async function installOperatorRoutes(
  page: Page,
  options: { nativeFails?: boolean } = {},
) {
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    const path = url.pathname;
    if (!path.startsWith("/v1/")) return route.continue();

    if (path === "/v1/buddy/work") {
      const view = url.searchParams.get("view") ?? "needs_you";
      const states = VIEW_STATES[view] ?? [];
      return route.fulfill({
        json: envelope(
          SEEDS.filter((seed) => states.includes(seed.state)).map(workRecord),
        ),
      });
    }

    const work = /^\/v1\/buddy\/work\/([^/]+)(\/[a-z]+)?$/.exec(path);
    if (work) {
      const seed = seedById(decodeURIComponent(work[1]));
      if (!seed)
        return route.fulfill({ status: 404, json: { detail: "gone" } });
      if (work[2] === "/actions") {
        if (seed.actionFails) {
          return route.fulfill({
            status: 503,
            json: { detail: "unavailable" },
          });
        }
        return route.fulfill({
          json: envelope(seed.action === null ? [] : [deskAction(seed)]),
        });
      }
      if (work[2] === "/outcomes") return route.fulfill({ json: envelope([]) });
      if (work[2] === "/read" || work[2] === "/snooze") {
        return route.fulfill({ json: envelope(null) });
      }
      if (work[2] === undefined) {
        return route.fulfill({ json: envelope(workRecord(seed)) });
      }
      return route.fulfill({ status: 404, json: { detail: "gone" } });
    }

    const artifact = /^\/v1\/buddy\/artifacts\/([^/]+)(\/revisions)?$/.exec(
      path,
    );
    if (artifact) {
      const seed = seedByArtifact(decodeURIComponent(artifact[1]));
      if (!seed)
        return route.fulfill({ status: 404, json: { detail: "gone" } });
      return route.fulfill({
        json: envelope(
          artifact[2] === undefined
            ? artifactRecord(seed)
            : [artifactRecord(seed)],
        ),
      });
    }

    const fail = options.nativeFails === true;
    if (path === "/v1/buddy/decisions/current") {
      return fail
        ? route.fulfill({ status: 503, json: { detail: "unavailable" } })
        : route.fulfill({ json: EMPTY_DECISIONS });
    }
    if (path === "/v1/buddy/approvals/current") {
      return fail
        ? route.fulfill({ status: 503, json: { detail: "unavailable" } })
        : route.fulfill({ json: EMPTY_APPROVALS });
    }
    if (path === "/v1/buddy/supervision/context") {
      return fail
        ? route.fulfill({ status: 503, json: { detail: "unavailable" } })
        : route.fulfill({ json: SUPERVISION_CONTEXT });
    }
    if (path === "/v1/buddy/native-work") {
      return fail
        ? route.fulfill({ status: 503, json: { detail: "unavailable" } })
        : route.fulfill({ json: EMPTY_NATIVE_WORK });
    }

    return route.fulfill({
      status: 503,
      json: { detail: "Not part of the realistic fixture" },
    });
  });
}

async function openBuddy(
  page: Page,
  options: {
    width: number;
    route?: string;
    nativeFails?: boolean;
  },
) {
  await installOperatorRoutes(page, { nativeFails: options.nativeFails });
  await page.setViewportSize({ width: options.width, height: FOLD_PX });
  await page.goto(options.route ?? ENABLED_ROUTE);
  await page.getByTestId("buddy-home-content").waitFor();
}

async function waitForDeskRows(page: Page, expected: number) {
  await expect(page.locator(`${DESK} ul li button`)).toHaveCount(expected);
}

function readRows(page: Page) {
  return page.evaluate((desk: string) => {
    const rows = [...document.querySelectorAll(`${desk} ul li button`)];
    return rows.map((row) => {
      const rect = row.getBoundingClientRect();
      return {
        title: (row.firstElementChild?.textContent ?? "").trim(),
        text: (row.textContent ?? "").replace(/\s+/g, " ").trim(),
        top: Math.round(rect.top),
        bottom: Math.round(rect.bottom),
      };
    });
  }, DESK);
}

function readTabStrip(page: Page) {
  return page.evaluate(() => {
    const tablist = document.querySelector<HTMLElement>("[role='tablist']");
    if (!tablist) return null;
    const style = window.getComputedStyle(tablist);
    return {
      scrollWidth: tablist.scrollWidth,
      clientWidth: tablist.clientWidth,
      scrollable: ["auto", "scroll"].includes(style.overflowX),
      triggers: [...tablist.querySelectorAll<HTMLElement>("[role='tab']")].map(
        (trigger) => ({
          name: (trigger.textContent ?? "").trim(),
          right: trigger.offsetLeft + trigger.offsetWidth,
        }),
      ),
    };
  });
}

test.describe("Buddy with realistic saved work", () => {
  test.beforeEach(({ page }) => {
    page.on("pageerror", (error) => {
      throw error;
    });
  });

  test("shows saved work inside the first viewport, not just navigation", async ({
    page,
  }) => {
    await openBuddy(page, { width: 1280 });
    await waitForDeskRows(page, NEEDS_YOU_TITLES.length);

    const rows = await readRows(page);
    const aboveTheFold = rows.filter(
      (row) => row.bottom > 0 && row.bottom <= FOLD_PX,
    );
    expect(
      aboveTheFold.length,
      `only ${aboveTheFold.length} of ${
        rows.length
      } work rows fit in a ${FOLD_PX}px viewport; the audited build showed the Desk at y=769 with zero rows visible (row edges: ${rows
        .map((row) => `${row.top}-${row.bottom}`)
        .join(", ")})`,
    ).toBeGreaterThanOrEqual(2);
  });

  test(`reaches every tab trigger at ${NARROW_PX}px`, async ({ page }) => {
    await openBuddy(page, { width: NARROW_PX });
    await waitForDeskRows(page, NEEDS_YOU_TITLES.length);

    const strip = await readTabStrip(page);
    expect(strip).not.toBeNull();
    expect(strip!.triggers.length).toBeGreaterThanOrEqual(3);
    expect(
      strip!.triggers.filter(
        (trigger) => trigger.right > strip!.scrollWidth + 1,
      ),
      "a tab trigger sits outside the scrollable extent of its own tab strip, so no amount of scrolling reaches it",
    ).toEqual([]);
    expect(
      strip!.scrollWidth <= strip!.clientWidth + 1 || strip!.scrollable,
      `tab strip overflows (${strip!.scrollWidth} > ${
        strip!.clientWidth
      }) without being scrollable; the audited build parked three triggers at x=342-563 inside a ${NARROW_PX}px viewport`,
    ).toBe(true);

    for (const trigger of strip!.triggers) {
      const tab = page.getByRole("tab", { name: trigger.name, exact: true });
      await tab.click();
      await expect(tab).toHaveAttribute("aria-selected", "true");
      const box = await tab.boundingBox();
      expect(box, `tab "${trigger.name}" has no layout box`).not.toBeNull();
      expect(
        box!.x >= -1 && box!.x + box!.width <= NARROW_PX + 1,
        `tab "${trigger.name}" is still at x=${Math.round(
          box!.x,
        )}..${Math.round(box!.x + box!.width)} after being scrolled into view`,
      ).toBe(true);
    }
  });

  test("never prints serialized internals in the Desk", async ({ page }) => {
    await openBuddy(page, { width: 1280 });
    await waitForDeskRows(page, NEEDS_YOU_TITLES.length);
    await page
      .getByRole("button", { name: /Knowledge base needs attention/ })
      .click();
    await expect(page.getByLabel("Work detail")).toBeVisible();

    const jsonish = await page.evaluate((desk: string) => {
      const node = document.querySelector<HTMLElement>(desk);
      return (node?.innerText ?? "")
        .split("\n")
        .map((line) => line.trim())
        .filter(
          (line) =>
            /^[[\]{}]$/.test(line) ||
            /^"[^"]+"\s*:/.test(line) ||
            /"(kind|action|schema_version|project_id)"\s*:/.test(line),
        );
    }, DESK);
    expect(
      jsonish,
      "a serialized payload reached the screen; bodies must be written for a person",
    ).toEqual([]);
  });

  test("renders no duplicate work titles in any view", async ({ page }) => {
    await openBuddy(page, { width: 1280 });
    await waitForDeskRows(page, NEEDS_YOU_TITLES.length);

    const seen: string[] = [];
    for (const [view, expected] of [
      ["Needs you", NEEDS_YOU_TITLES.length],
      ["Watching", WATCHING_TITLES.length],
    ] as const) {
      await page.getByRole("button", { name: view, exact: true }).click();
      await waitForDeskRows(page, expected);
      const rows = await readRows(page);
      const titles = rows.map((row) => row.title);
      const duplicates = titles.filter(
        (title, index) => titles.indexOf(title) !== index,
      );
      expect(
        duplicates,
        `${view} rendered the same title more than once; the audited store showed "Knowledge base needs attention: 97 lifecycle candidate(s)" three times`,
      ).toEqual([]);
      seen.push(...titles);
    }
    expect(seen.length).toBeGreaterThanOrEqual(SEEDS.length - 1);
  });

  test("never headlines a row with a bare chat UUID", async ({ page }) => {
    await openBuddy(page, { width: 1280 });
    await waitForDeskRows(page, NEEDS_YOU_TITLES.length);

    const headlines = await page.evaluate((desk: string) => {
      const rows = [...document.querySelectorAll(`${desk} ul li button`)].map(
        (row) => (row.firstElementChild?.textContent ?? "").trim(),
      );
      const headings = [
        ...document.querySelectorAll(
          "h1, h2, h3, h4, h5, h6, [role='heading']",
        ),
      ].map((heading) => (heading.textContent ?? "").trim());
      return [...rows, ...headings].filter(Boolean);
    }, DESK);

    expect(headlines.length).toBeGreaterThan(0);
    expect(
      headlines.filter((headline) =>
        /\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b/i.test(
          headline,
        ),
      ),
      "a raw UUID was used as a headline; the human title belongs there and the id belongs in secondary detail",
    ).toEqual([]);
    expect(headlines).toContain("Finish multi-agent parity M1–M10");
  });

  test("stops claiming progress when the work service is broken", async ({
    page,
  }) => {
    await openBuddy(page, {
      width: 1280,
      route: UNAVAILABLE_ROUTE,
      nativeFails: true,
    });

    await expect(page.getByTestId("buddy-operator-degraded")).toBeVisible();
    await expect(page.getByTestId("buddy-summary-strip")).toHaveCount(0);

    const body = await page.evaluate(() => document.body.innerText);
    expect(
      /Success\s+\d+\s*%/.test(body),
      "a cached success rate was presented as live while nothing could confirm it",
    ).toBe(false);
    for (const label of VANITY_STAT_LABELS) {
      expect(
        page.getByTestId("buddy-summary-strip").getByText(label, {
          exact: true,
        }),
        `vanity stat "${label}" survived a total work-service failure`,
      ).toHaveCount(0);
    }
    for (const copy of REASSURING_EMPTY_COPY) {
      await expect(
        page.getByText(copy, { exact: false }),
        `a failed request rendered the reassuring copy "${copy}"`,
      ).toHaveCount(0);
    }
    await expect(
      page.getByRole("button", { name: "Retry work authority", exact: true }),
    ).toBeVisible();
  });

  test("never answers a failed request with a settled empty state", async ({
    page,
  }) => {
    await openBuddy(page, {
      width: 1280,
      route: BUDDY_ROUTE,
      nativeFails: true,
    });

    for (const copy of REASSURING_EMPTY_COPY) {
      await expect(
        page.getByText(copy, { exact: false }),
        `a 503 rendered the reassuring copy "${copy}"`,
      ).toHaveCount(0);
    }
    await expect(page.getByRole("alert").first()).toBeVisible();
  });

  test("gives every row an action verb or an explicit dead end", async ({
    page,
  }) => {
    await openBuddy(page, { width: 1280 });
    await waitForDeskRows(page, NEEDS_YOU_TITLES.length);
    await expect(
      page.getByText("Action status unavailable", { exact: true }),
    ).toBeVisible();

    const rows = await readRows(page);
    const mute = rows.filter(
      (row) =>
        !NO_ACTION_PHRASES.some((phrase) => row.text.includes(phrase)) &&
        !ACTION_LABELS.some(
          (label) => label !== "" && row.text.includes(label),
        ),
    );
    expect(
      mute.map((row) => row.text),
      "a work row offered neither an action verb nor an honest statement that there is none",
    ).toEqual([]);
    expect(
      rows.filter((row) =>
        NO_ACTION_PHRASES.some((phrase) => row.text.includes(phrase)),
      ).length,
      "the fixture must exercise the honest no-action branch",
    ).toBeGreaterThanOrEqual(2);
  });

  test("keeps engine vocabulary out of the visible copy", async ({ page }) => {
    await openBuddy(page, { width: 1280 });
    await waitForDeskRows(page, NEEDS_YOU_TITLES.length);

    const body = await page.evaluate(() => document.body.innerText);
    const leaked = ENGINE_JARGON.filter((phrase) =>
      body.toLowerCase().includes(phrase.toLowerCase()),
    );
    expect(
      leaked,
      "engine vocabulary reached the screen; this surface is read by a person, not a reviewer of the storage contract",
    ).toEqual([]);
    expect(BARE_UUID.test(body.split("\n")[0] ?? "")).toBe(false);
  });
});
