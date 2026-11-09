import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";

import { setUpStore } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { AbortControllerProvider } from "../../src/contexts/AbortControllers";
import { applyChatEvent } from "../../src/features/Chat/Thread";
import { ReviewChatSurface } from "../../src/features/Review";
import surfacePaneStyles from "../../src/features/Workspace/SurfacePane.module.css";
import {
  invalidateReviewForChat,
  type Anchor,
  type Candidate,
  type Review,
} from "../../src/services/refact/review";
import "../../src/lib/render/web.css";

/*
 * Browser fixture: the production ReviewChatSurface rendered against a
 * deterministic in-page transport. Only fetch is intercepted; no UI is
 * copied. Query params: ?theme=dark, ?state=empty. window.__reviewFixture
 * exposes activate(), fail() and recover() to drive async transitions.
 */

const params = new URLSearchParams(window.location.search);
const CHAT_ID = "fixture-chat";

const config = {
  host: "web" as const,
  dev: true,
  lspUrl: window.location.origin,
  lspPort: Number(window.location.port),
  apiKey: null,
  themeProps: {
    appearance:
      params.get("theme") === "dark" ? ("dark" as const) : ("light" as const),
  },
};

const BASE_TEXT = [
  "export function parse(input: string) {",
  "  const parts = input.split(',');",
  "  return parts.map((p) => Number(p));",
  "}",
  "",
  "export function total(values: number[]) {",
  "  let sum = 0;",
  "  for (let i = 0; i <= values.length; i++) sum += values[i];",
  "  return sum;",
  "}",
].join("\n");
const C1_HEAD = BASE_TEXT.replace("Number(p)", "Number(p.trim())");
const C2_HEAD = C1_HEAD.replace("i <= values.length", "i < values.length");
const README = "# Notes\n\nParser now trims whitespace.\n";
const STDOUT = "PASS parse.test.ts\nPASS total.test.ts\n\n2 passed\n";

const texts: Record<string, string> = {
  base: BASE_TEXT,
  c1: C1_HEAD,
  c2: C2_HEAD,
  readme: README,
  stdout: STDOUT,
};

function encode(text: string): number[] {
  return Array.from(new TextEncoder().encode(text));
}

function anchorAt(
  candidateId: string,
  path: string,
  start: number,
  end: number,
  status: Anchor["status"] = "current",
): Anchor {
  return {
    position: {
      candidate_id: candidateId,
      path,
      side: "head",
      start_line: start,
      end_line: end,
      content_hash: `fixture-${candidateId}-${start}`,
      context_before: [],
      context_after: [],
    },
    status,
    stalled_reason: status === "stalled" ? "no exact contextual match" : null,
    history: [],
  };
}

function candidate(id: string, createdAt: number): Candidate {
  return {
    id,
    base_oid: "1111111111111111",
    head_oid: id === "c1" ? "aaaaaaaa11111111" : "bbbbbbbb22222222",
    content_digest: `digest-${id}`,
    created_at_ms: createdAt,
    repository: "/fixture/repo",
    origin: "local",
    policy: { staged: true, working: true, untracked: false },
    snapshot_scope: "tracked regular files; .refact excluded",
    files: [
      {
        path: "src/parse.ts",
        base: { digest: "base", size: BASE_TEXT.length },
        head: { digest: id, size: 1 },
        base_mode: 0o100644,
        head_mode: 0o100644,
      },
      {
        path: "README.md",
        base: { digest: "readme", size: README.length },
        head: { digest: "readme", size: README.length },
        base_mode: 0o100644,
        head_mode: 0o100644,
      },
    ],
  };
}

const now = Date.UTC(2025, 0, 15, 12, 0, 0);

const review: Review = {
  schema_version: 1,
  id: "fixture-review",
  owner_chat_id: CHAT_ID,
  revision: 12,
  current_candidate_id: "c2",
  title: "Review: parser trimming and total() bounds",
  origin: "local",
  target_reference: "feature/parser",
  candidates: [candidate("c1", now - 3600_000), candidate("c2", now)],
  tour: [
    {
      id: "step-1",
      current: {
        version: 2,
        title:
          params.get("title") === "long"
            ? "One operator changes both the result and module loading"
            : "Input trimming",
        body:
          params.get("visual") === "html"
            ? 'parse() now trims each element before conversion.\n\n```html\n<section><p>Input → trim → convert</p><button id="visual-counter" onclick="this.textContent=String(Number(this.textContent)+1)">0</button><button onclick="window.refactReview.revealAnchor(0)">Reveal code</button><script>window.visualInitializations=(window.visualInitializations||0)+1</script></section>\n```'
            : "parse() now trims each element before conversion.",
        anchors: [anchorAt("c2", "src/parse.ts", 3, 3)],
      },
      history: [
        {
          version: 1,
          title: "Input parsing",
          body: "parse() converts elements.",
          anchors: [anchorAt("c1", "src/parse.ts", 3, 3)],
        },
      ],
    },
    {
      id: "step-2",
      current: {
        version: 1,
        title: "Loop bound fix",
        body: "total() no longer reads past the end of the array.",
        anchors: [anchorAt("c2", "src/parse.ts", 8, 8)],
      },
      history: [],
    },
  ],
  annotations: [
    {
      id: "finding-1",
      kind: "finding",
      title: "Off-by-one in total()",
      body: "The loop bound was `<=`, reading `values[length]`.",
      anchors: [anchorAt("c2", "src/parse.ts", 8, 8)],
      severity: "high",
      confidence: "high",
      assessment: "supported",
      disposition: "fixed",
      applicability: "current",
      evidence_ids: ["ev-tests"],
      run_id: "run-1",
    },
    {
      id: "finding-2",
      kind: "finding",
      title: "Empty input yields [0]",
      body: "`''.split(',')` returns `['']`, which parses to `[0]`.",
      anchors: [anchorAt("c1", "src/parse.ts", 2, 2, "stalled")],
      severity: "medium",
      confidence: "medium",
      assessment: "unassessed",
      disposition: "open",
      applicability: "needs_revalidation",
      evidence_ids: [],
      run_id: null,
    },
  ],
  threads: [
    {
      id: "thread-1",
      anchors: [anchorAt("c2", "src/parse.ts", 3, 3)],
      annotation_id: null,
      resolved: false,
      contributions: [
        {
          id: "contrib-1",
          thread_id: "thread-1",
          actor: { kind: "user", id: "user" },
          body: "Should we also reject NaN here?",
          created_at_ms: now - 1800_000,
          chat_message_id: "m1",
          context: {
            candidate_id: "c1",
            run_id: null,
            applicability: "current",
            privacy: [],
          },
        },
        {
          id: "contrib-2",
          thread_id: "thread-1",
          actor: { kind: "agent", id: "agent" },
          body: "Good point; a NaN check belongs in a follow-up.",
          created_at_ms: now - 600_000,
          chat_message_id: "m2",
        },
      ],
    },
  ],
  evidence: [
    {
      id: "ev-tests",
      candidate_id: "c2",
      material_id: "mat-1",
      run_id: "run-1",
      source: {
        kind: "executed",
        command: ["npm", "test", "--", "parse"],
        workdir: "/fixture/worktrees/run-1",
        exit_code: 0,
        stdout: { digest: "stdout", size: STDOUT.length },
        stderr: null,
      },
      summary: "Parser tests pass on the candidate",
      applicability: "current",
      created_at_ms: now - 300_000,
    },
    {
      id: "ev-source",
      candidate_id: "c2",
      material_id: null,
      run_id: null,
      source: {
        kind: "source",
        path: "src/parse.ts",
        anchor: anchorAt("c2", "src/parse.ts", 6, 9),
      },
      summary: "total() implementation consulted",
      applicability: "current",
      created_at_ms: now - 200_000,
    },
  ],
  runs: [
    {
      id: "run-1",
      candidate_id: "c2",
      agent_id: "agent-verify",
      purpose: "Run parser tests",
      model: "fixture-model",
      domain: null,
      status: "completed",
      started_at_ms: now - 400_000,
      finished_at_ms: now - 300_000,
      error: null,
    },
    {
      id: "run-2",
      candidate_id: "c2",
      agent_id: "agent-probe",
      purpose: "Probe empty-input behaviour",
      model: "fixture-model",
      domain: null,
      status: "running",
      started_at_ms: now - 60_000,
      finished_at_ms: null,
      error: null,
    },
  ],
  materials: [
    {
      id: "mat-1",
      kind: "investigation",
      candidate_id: "c2",
      agent_id: "agent-verify",
      description: "Test run workspace",
      content_digest: "materialdigest00",
      input_digests: { candidate: "digest-c2" },
      environment: "node 20",
      worktree_id: "wt-1",
      worktree_root: "/fixture/worktrees/run-1",
      artifacts: [{ digest: "stdout", size: STDOUT.length }],
    },
  ],
  activity: [
    {
      id: "act-1",
      at_ms: now - 300_000,
      actor: { kind: "agent", id: "agent-verify" },
      action: "add_evidence",
      candidate_id: "c2",
    },
  ],
};

if (params.get("tour") === "rich") {
  const anchor = anchorAt("c2", "src/parse.ts", 3, 3);
  review.tour[0].current = {
    ...review.tour[0].current,
    candidate_id: "c2",
    blocks: [
      {
        id: "intro",
        revision: 1,
        content: { kind: "markdown", body: "A living explanation of parsing." },
      },
      {
        id: "visual",
        revision: 1,
        content: {
          kind: "html",
          html: '<button id="rich-counter" onclick="this.textContent=String(Number(this.textContent)+1)">0</button><button id="invalid-anchor" onclick="window.refactReview.revealAnchor(-1)">Invalid anchor</button><button id="valid-anchor" onclick="window.refactReview.revealAnchor(0)">Valid anchor</button><script>window.visualInitializations=(window.visualInitializations||0)+1</script>',
          ...(params.get("dynamic") === "evaluation"
            ? {
                static_fallback:
                  "Evaluation order: the caller supplies a comma-separated string. The parser splits it into parts, trims each part, then converts each trimmed part to a number. Trimming occurs before conversion; the caller receives the mapped array. Source: src/parse.ts, head lines 1–4.",
                html: `<h3>Caller and parser evaluation</h3>
<p role="status" aria-live="polite" id="evaluation">1. Caller supplies input: ' 2 , 3 '</p>
<button id="step">Next step</button><button id="play">Play</button><button id="pause">Pause</button><button id="replay">Replay</button>
<button onclick="window.refactReview.revealAnchor(0)">Show parser source</button>
<script>
const stages = ["1. Caller supplies input: ' 2 , 3 '", "2. Parser splits into parts: [' 2 ', ' 3 ']", "3. Each part is trimmed: ['2', '3']", "4. Number conversion returns [2, 3] to caller"];
let current = 0, timer;
const output = document.getElementById('evaluation');
function pause() { clearInterval(timer); timer = undefined; }
function step() { current = Math.min(current + 1, stages.length - 1); output.textContent = stages[current]; if (current === stages.length - 1) pause(); }
document.getElementById('step').onclick = () => { pause(); step(); };
document.getElementById('play').onclick = () => { pause(); if (current < stages.length - 1) timer = setInterval(step, 900); };
document.getElementById('pause').onclick = pause;
document.getElementById('replay').onclick = () => { pause(); current = 0; output.textContent = stages[0]; };
</script>`,
              }
            : {}),
          anchors: [anchor],
        },
      },
      { id: "code", revision: 1, content: { kind: "code", anchor } },
      {
        id: "diff",
        revision: 1,
        content: {
          kind: "diff",
          candidate_id: "c2",
          path: "src/parse.ts",
          anchors: [anchor],
        },
      },
      {
        id: "sources",
        revision: 1,
        content: {
          kind: "sources",
          relation: "consulted",
          evidence_ids: ["ev-readme", "ev-external", "ev-tests"],
        },
      },
      {
        id: "evidence",
        revision: 1,
        content: { kind: "evidence", evidence_ids: ["ev-tests"] },
      },
    ],
  };
  review.evidence.push(
    {
      ...review.evidence[1],
      id: "ev-readme",
      source: { kind: "source", path: "README.md", anchor: null },
      summary: "Unchanged context file",
    },
    {
      ...review.evidence[1],
      id: "ev-external",
      source: { kind: "external", url: "https://example.com/reference" },
      summary: "External reference",
    },
  );
  review.evidence[0].summary = "Fixture record only; no process was executed";
}

// Opt-in synthetic UI laboratory, never evidence of an actual backend run.
if (params.get("polish") === "1") {
  const execution = review.evidence[0].source;
  if (execution.kind === "executed")
    execution.stderr = { digest: "stdout", size: STDOUT.length };
  review.evidence[1].summary = "Synthetic source context. ".repeat(16);
  const blocks = review.tour[0].current.blocks;
  if (blocks) {
    blocks[0].content = {
      kind: "markdown",
      body: "**Synthetic Review interaction laboratory** — deterministic fixture data, not an actual server or execution record.\n\n```mermaid\nflowchart LR\n  Input --> Trim\n  Trim --> Convert\n  Convert --> Result\n```",
    };
    const visual = blocks.find((block) => block.id === "visual");
    if (visual?.content.kind === "html") {
      visual.content.html = `<!doctype html><html><head><style>
        body{font:14px system-ui;margin:0;padding:16px;color:inherit}
        button{font:inherit;padding:8px 12px;margin:4px;border:1px solid #8090a0;border-radius:6px;background:transparent;color:inherit}
        #dynamic-panel{padding:12px;border:1px dashed #8090a0;border-radius:6px}
      </style></head><body><strong>Synthetic interactive parser model</strong><p>Counter and disclosure stay local to this sandbox.</p>
      <button id="rich-counter" aria-label="Increment synthetic counter" onclick="this.textContent=String(Number(this.textContent)+1)">0</button>
      <button id="dynamic-toggle" aria-expanded="false" onclick="const panel=document.getElementById('dynamic-panel');panel.hidden=!panel.hidden;this.setAttribute('aria-expanded',String(!panel.hidden))">Toggle synthetic detail</button>
      <div id="dynamic-panel" hidden>Whitespace is removed before numeric conversion.</div>
      <button id="invalid-anchor" onclick="window.refactReview.revealAnchor(-1)">Invalid anchor</button>
      <button id="valid-anchor" onclick="window.refactReview.revealAnchor(0)">Reveal native source</button>
      <script>window.visualInitializations=(window.visualInitializations||0)+1</script></body></html>`;
    }
  }
  review.threads.push({
    id: "synthetic-rich-thread",
    anchors: [],
    target: {
      candidate_id: "c2",
      step_id: "step-1",
      version: 2,
      block_id: "intro",
      block_revision: 1,
    },
    annotation_id: null,
    resolved: false,
    contributions: [
      {
        id: "synthetic-human",
        thread_id: "synthetic-rich-thread",
        actor: { kind: "user", id: "synthetic-human" },
        body:
          "Synthetic human: why trim before conversion?\n\n" +
          "Synthetic long discussion detail about the input boundary. ".repeat(
            14,
          ),
        created_at_ms: now,
      },
      {
        id: "synthetic-agent",
        thread_id: "synthetic-rich-thread",
        actor: { kind: "agent", id: "synthetic-agent" },
        body: "Synthetic agent: trim normalizes the input without changing the numeric result.",
        created_at_ms: now + 1000,
        blocks: [
          {
            id: "synthetic-reply-explanation",
            revision: 1,
            content: {
              kind: "markdown",
              body:
                "**Synthetic agent explanation**\n\n" +
                Array.from(
                  { length: 8 },
                  (_, index) =>
                    `Detail ${
                      index + 1
                    }: review the whitespace boundary before conversion.`,
                ).join("\n\n"),
            },
          },
        ],
      },
    ],
  });
}

if (params.get("navigation") === "exact") {
  const blocks = [
    {
      id: "navigation-spacer",
      revision: 1,
      content: {
        kind: "markdown" as const,
        body: Array.from(
          { length: 30 },
          (_, index) => `Context paragraph ${index}.`,
        ).join("\n\n"),
      },
    },
    {
      id: "navigation-destination",
      revision: 1,
      content: { kind: "markdown" as const, body: "Exact destination block" },
    },
  ];
  blocks.push({
    id: "navigation-tail",
    revision: 1,
    content: {
      kind: "markdown",
      body: Array.from({ length: 30 }, () => "Following context.").join("\n\n"),
    },
  });
  const historical = {
    version: 1,
    title: "Historical loop explanation",
    body: "",
    candidate_id: "c1",
    anchors: [],
    blocks,
  };
  review.tour[1] = {
    id: "step-2",
    current: {
      ...historical,
      version: 2,
      candidate_id: "c2",
      title: "Current loop explanation",
    },
    history: [historical],
  };
  for (const version of [1, 2]) {
    for (const suffix of ["first", "exact"]) {
      const id = `navigation-${version}-${suffix}`;
      review.threads.push({
        id,
        anchors: [],
        annotation_id: null,
        resolved: false,
        target: {
          candidate_id: version === 1 ? "c1" : "c2",
          step_id: "step-2",
          version,
          block_id: "navigation-destination",
          block_revision: 1,
        },
        contributions: [
          {
            id: `${id}-contribution`,
            thread_id: id,
            actor: { kind: "user", id: "user" },
            body:
              suffix === "first"
                ? Array.from(
                    { length: 25 },
                    () => "Earlier discussion context.",
                  ).join("\n\n")
                : `Exact discussion version ${version}`,
            created_at_ms: now,
          },
        ],
      });
    }
  }
}

const scenario = params.get("state") ?? "review";
const state: { review: Review | null; failing: boolean } = {
  review: scenario === "empty" ? null : review,
  failing: false,
};

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function fileText(candidateId: string, path: string, side: "base" | "head") {
  if (path === "README.md") return texts.readme;
  return side === "base" ? texts.base : texts[candidateId];
}

let releaseFiles: () => void = () => undefined;
const filesReady =
  params.get("reader") === "delayed"
    ? new Promise<void>((resolve) => {
        releaseFiles = resolve;
      })
    : Promise.resolve();
const fixtureComments: unknown[] = [];
const realFetch = window.fetch.bind(window);
window.fetch = async (input, init) => {
  const request = input instanceof Request ? input : new Request(input, init);
  const url = new URL(request.url);
  if (url.pathname.startsWith("/v1/review-stacks/by-chat/")) return json(null);
  if (!url.pathname.startsWith("/v1/reviews")) return realFetch(input, init);
  const path = url.pathname;
  if (state.failing) return json({ detail: "chat unavailable" }, 404);
  if (path === `/v1/reviews/by-chat/${CHAT_ID}`) return json(state.review);
  if (!state.review) return json({ detail: "not found" }, 404);
  const current = state.review;
  if (path.endsWith("/files")) {
    await filesReady;
    const candidateId = url.searchParams.get("candidate_id") ?? "";
    const filePath = url.searchParams.get("path") ?? "";
    return json({
      path: filePath,
      base: encode(fileText(candidateId, filePath, "base")),
      head: encode(fileText(candidateId, filePath, "head")),
      diff: "",
    });
  }
  if (path.includes("/blobs/")) {
    const digest = decodeURIComponent(path.split("/").at(-1) ?? "");
    return json(encode(texts[digest] ?? ""));
  }
  if (path.endsWith("/delivery-status")) return json([]);
  if (path.endsWith("/history")) {
    return json([
      {
        review_id: current.id,
        revision: 4,
        actor: { kind: "agent", id: "agent" },
        at_ms: now - 3000_000,
        object: {
          kind: "annotation",
          value: {
            ...current.annotations[0],
            body: "Loop may read past the array end.",
            disposition: "open",
            anchors: [anchorAt("c1", "src/parse.ts", 8, 8)],
          },
        },
      },
    ]);
  }
  if (path.endsWith("/activity")) return json({ ok: true });
  if (path.endsWith("/anchors")) {
    const body = (await request.json()) as {
      candidate_id: string;
      path: string;
      start_line: number;
      end_line: number;
    };
    return json(
      anchorAt(body.candidate_id, body.path, body.start_line, body.end_line),
    );
  }
  if (path.endsWith("/comments")) {
    const body = (await request.json()) as {
      body: string;
      thread_id?: string;
      anchor?: Anchor;
      target?: import("../../src/services/refact/review").DocumentTarget;
    };
    fixtureComments.push(body);
    const threadId = body.thread_id ?? `thread-${current.revision + 1}`;
    const contribution = {
      id: `contrib-${current.revision + 1}`,
      thread_id: threadId,
      actor: { kind: "user" as const, id: "user" },
      body: body.body,
      created_at_ms: Date.now(),
      chat_message_id: null,
    };
    const exists = current.threads.some((t) => t.id === threadId);
    state.review = {
      ...current,
      revision: current.revision + 1,
      threads: exists
        ? current.threads.map((t) =>
            t.id === threadId
              ? { ...t, contributions: [...t.contributions, contribution] }
              : t,
          )
        : [
            ...current.threads,
            {
              id: threadId,
              anchors: body.anchor ? [body.anchor] : [],
              target: body.target,
              annotation_id: null,
              resolved: false,
              contributions: [contribution],
            },
          ],
    };
    return json(state.review);
  }
  if (path.includes("/runs/") && path.endsWith("/cancel")) {
    const runId = path.split("/").at(-2);
    state.review = {
      ...current,
      revision: current.revision + 1,
      runs: current.runs.map((r) =>
        r.id === runId ? { ...r, status: "cancel_requested" } : r,
      ),
    };
    return json(state.review);
  }
  if (path.endsWith("/mutations")) {
    state.review = { ...current, revision: current.revision + 1 };
    return json(state.review);
  }
  return json({ detail: `fixture: unhandled ${path}` }, 404);
};

const store = setUpStore({ config });
store.dispatch(
  applyChatEvent({
    chat_id: CHAT_ID,
    seq: "1",
    type: "snapshot",
    thread: {
      id: CHAT_ID,
      title: "Review fixture",
      model: "fixture-model",
      mode: "review",
      tool_use: "agent",
      boost_reasoning: false,
      context_tokens_cap: null,
      include_project_info: true,
      checkpoints_enabled: true,
      is_title_generated: false,
    },
    runtime: {
      state: "idle",
      paused: false,
      error: null,
      queue_size: 0,
      pause_reasons: [],
      queued_items: [],
    },
    background_agents: [],
    messages: [],
  }),
);

declare global {
  interface Window {
    __reviewFixture?: {
      activate: () => void;
      fail: () => void;
      recover: () => void;
      publishTopic: () => void;
      updateTopic: () => void;
      comments: () => unknown[];
      releaseFiles: () => void;
      syntheticReply: () => void;
      syntheticNewVersion: () => void;
    };
  }
}

window.__reviewFixture = {
  syntheticReply: () => {
    if (!state.review || params.get("polish") !== "1") return;
    const current = state.review;
    state.review = {
      ...current,
      revision: current.revision + 1,
      threads: current.threads.map((thread) =>
        thread.id === "synthetic-rich-thread"
          ? {
              ...thread,
              contributions: [
                ...thread.contributions,
                {
                  id: `synthetic-live-reply-${current.revision}`,
                  thread_id: thread.id,
                  actor: { kind: "agent" as const, id: "synthetic-agent" },
                  body: "Synthetic live agent reply: the whitespace edge case is now documented.",
                  created_at_ms: now + 2000,
                },
              ],
            }
          : thread,
      ),
    };
    store.dispatch(invalidateReviewForChat(CHAT_ID));
  },
  syntheticNewVersion: () => {
    if (params.get("polish") === "1") window.__reviewFixture?.updateTopic();
  },
  releaseFiles: () => releaseFiles(),
  comments: () => fixtureComments,
  activate: () => {
    state.review = review;
    store.dispatch(invalidateReviewForChat(CHAT_ID));
  },
  fail: () => {
    state.failing = true;
    store.dispatch(invalidateReviewForChat(CHAT_ID));
  },
  recover: () => {
    state.failing = false;
    store.dispatch(invalidateReviewForChat(CHAT_ID));
  },
  publishTopic: () => {
    if (!state.review) return;
    state.review = {
      ...state.review,
      revision: state.review.revision + 1,
      tour: [
        ...state.review.tour,
        {
          id: "new-topic",
          current: {
            version: 1,
            title: "Later investigation",
            body: "Published after opening.",
            anchors: [],
          },
          history: [],
        },
      ],
    };
    store.dispatch(invalidateReviewForChat(CHAT_ID));
  },
  updateTopic: () => {
    if (!state.review) return;
    const [first, ...rest] = state.review.tour;
    state.review = {
      ...state.review,
      revision: state.review.revision + 1,
      tour: [
        {
          ...first,
          current: {
            ...first.current,
            version: first.current.version + 1,
            title: "Updated input trimming",
          },
          history: [first.current, ...first.history],
        },
        ...rest,
      ],
    };
    store.dispatch(invalidateReviewForChat(CHAT_ID));
  },
};

function ChatPlaceholder() {
  return (
    <section aria-label="Chat (fixture placeholder)" className="fixture-chat">
      <div className="fixture-chat-log">
        <p>Chat transcript placeholder (the real Chat mounts here).</p>
      </div>
      <textarea aria-label="Chat composer" rows={4} />
    </section>
  );
}

const root = document.getElementById("root");
if (!root) throw new Error("Review fixture root is missing");
createRoot(root).render(
  <Provider store={store}>
    <Theme>
      <AbortControllerProvider>
        {params.get("polish") === "1" && (
          <div
            style={{
              position: "fixed",
              bottom: 4,
              left: 4,
              zIndex: 2147483647,
              padding: "3px 7px",
              background: "#17202a",
              color: "#ffffff",
              font: "11px system-ui",
              pointerEvents: "none",
              borderRadius: 4,
            }}
          >
            SYNTHETIC UI FIXTURE · no actual backend execution
          </div>
        )}
        <div className={surfacePaneStyles.surfacePane}>
          <ReviewChatSurface chatId={CHAT_ID}>
            <ChatPlaceholder />
          </ReviewChatSurface>
        </div>
      </AbortControllerProvider>
    </Theme>
  </Provider>,
);
