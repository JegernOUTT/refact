import { EditorView } from "@codemirror/view";
import {
  cleanup,
  fireEvent,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, it } from "vitest";
import { server } from "../../utils/mockServer";
import { render } from "../../utils/test-utils";
import { reviewApi, type StackManifest } from "../../services/refact/review";
import { ReviewStackSurface } from "./ReviewStackSurface";
import { anchor, bytes, makeReview } from "./reviewFixtures";

HTMLElement.prototype.hasPointerCapture = () => false;
HTMLElement.prototype.scrollIntoView = () => undefined;

const stack: StackManifest = {
  schema_version: 1,
  id: "stack",
  owner_chat_id: "chat-1",
  revision: 7,
  title: "Stack",
  repository: "/repo",
  source: { kind: "provided_manifest", reference: "user" },
  root_base_ref: "main",
  root_base_oid: "base",
  scope_choice: {
    scope: "whole",
    selected_member_id: "a",
    confirmed: true,
    explicit_request: true,
  },
  members: ["a", "b"].map((id) => ({
    id,
    branch_ref: `branch-${id}`,
    parent_member_id: id === "b" ? "a" : null,
    declared_base_ref: id === "b" ? "branch-a" : "main",
    resolved_base_oid: "base",
    head_oid: "head",
    pr_identity: null,
    review_id: `review-${id}`,
    state: "ready",
    reason: null,
  })),
};
const a = makeReview({ id: "review-a" });
const b = makeReview({ id: "review-b" });
const stores: ReturnType<typeof render>["store"][] = [];
afterEach(() => {
  cleanup();
  stores
    .splice(0)
    .forEach((store) => store.dispatch(reviewApi.util.resetApiState()));
});

function setup(manifest: StackManifest = stack) {
  const calls: { method: string; path: string; query: string }[] = [];
  const comments: Record<string, unknown>[] = [];
  let failMember = false;
  server.use(
    http.all("*/v1/*", async ({ request }) => {
      const url = new URL(request.url);
      calls.push({
        method: request.method,
        path: url.pathname,
        query: url.search,
      });
      const path = url.pathname;
      if (path === "/v1/review-stacks/by-chat/chat-1")
        return HttpResponse.json(manifest);
      if (path.endsWith("/members/a"))
        return failMember
          ? HttpResponse.json({ detail: "offline" }, { status: 503 })
          : HttpResponse.json(a);
      if (path.endsWith("/members/b")) return HttpResponse.json(b);
      if (path.endsWith("/cumulative/a/files"))
        return HttpResponse.json({
          path: "src/a.ts",
          base: bytes("stack old\n"),
          head: bytes("stack new\n"),
          diff: "",
        });
      if (path.endsWith("/cumulative/a"))
        return HttpResponse.json({
          comparison_id: "combined",
          stack_id: "stack",
          stack_revision: 7,
          member_id: "a",
          read_only: true,
          annotation_target: null,
          candidate: a.candidates[0],
        });
      if (path.endsWith("/files"))
        return HttpResponse.json({
          path: "src/a.ts",
          base: bytes("old\n"),
          head: bytes(`new ${path}\n`),
          diff: "",
        });
      if (path.endsWith("/delivery-status")) return HttpResponse.json([]);
      if (path.endsWith("/anchors"))
        return HttpResponse.json(anchor("c1", "src/a.ts", "head", 1));
      if (path.endsWith("/comments")) {
        comments.push((await request.json()) as Record<string, unknown>);
        return comments.length === 1
          ? HttpResponse.error()
          : HttpResponse.json(a);
      }
      return HttpResponse.json({ ok: true });
    }),
  );
  return {
    calls,
    comments,
    fail: () => {
      failMember = true;
    },
  };
}
function mount(review: typeof a | null = a) {
  const result = render(
    <>
      <textarea aria-label="Chat composer" />
      <ReviewStackSurface chatId="chat-1" review={review} stale={false} ready />
    </>,
  );
  stores.push(result.store);
  return result;
}
async function choose(user: ReturnType<typeof render>["user"], id: string) {
  await user.click(
    screen.getByRole("combobox", { name: "Browse stack branch" }),
  );
  await user.click(
    await screen.findByRole("option", {
      name: id === "a" ? /branch-a.*main/ : /branch-b/,
    }),
  );
}

describe("ReviewStackSurface integration", () => {
  it.each([1, 2])(
    "retains an unvisited member thread reveal through delayed fetch (version %s)",
    async (version) => {
      const { calls } = setup();
      let release!: () => void;
      const gate = new Promise<void>((resolve) => {
        release = resolve;
      });
      const historical = {
        version: 1,
        title: "Historical destination",
        body: "",
        anchors: [],
        candidate_id: "c1",
        blocks: [
          {
            id: "destination",
            revision: 1,
            content: {
              kind: "markdown" as const,
              body: "Historical block content",
            },
          },
        ],
      };
      const current = {
        ...historical,
        version: 2,
        title: "Current destination",
        blocks: [
          {
            ...historical.blocks[0],
            content: {
              kind: "markdown" as const,
              body: "Current block content",
            },
          },
        ],
      };
      const member = makeReview({
        id: "review-b",
        tour: [{ id: "destination-topic", current, history: [historical] }],
        threads: [
          {
            id: "destination-thread",
            anchors: [],
            annotation_id: null,
            resolved: false,
            target: {
              candidate_id: "c1",
              step_id: "destination-topic",
              version,
              block_id: "destination",
              block_revision: 1,
            },
            contributions: [
              {
                id: "destination-contribution",
                thread_id: "destination-thread",
                actor: { kind: "user", id: "user" },
                body: "Exact delayed discussion",
                created_at_ms: 1,
                chat_message_id: null,
              },
            ],
          },
        ],
      });
      let requested = false;
      server.use(
        http.get("*/v1/review-stacks/by-chat/chat-1/members/b", async () => {
          requested = true;
          await gate;
          return HttpResponse.json(member);
        }),
      );
      mount();
      await screen.findByRole("combobox", { name: "Browse stack branch" });
      expect(requested).toBe(false);
      fireEvent(
        window,
        new CustomEvent("refact:review-reveal", {
          detail: { reviewId: "review-b", threadId: "destination-thread" },
        }),
      );
      await waitFor(() => expect(requested).toBe(true));
      expect(
        screen.queryByText("Exact delayed discussion"),
      ).not.toBeInTheDocument();
      release();
      expect(
        await screen.findByRole("heading", {
          name:
            version === 1 ? "Historical destination" : "Current destination",
        }),
      ).toBeVisible();
      expect(await screen.findByText("Exact delayed discussion")).toBeVisible();
      expect(
        screen.getByRole("combobox", { name: /Version of/ }),
      ).toHaveTextContent(
        `v${version} · ${version === 2 ? "latest" : "historical"}`,
      );
      expect(
        screen.getByRole("combobox", { name: "Browse stack branch" }),
      ).toHaveTextContent("branch-b");
      expect(
        calls.filter((call) => /context|activate/.test(call.path)),
      ).toEqual([]);
    },
  );
  it("disables immutable history actions without making cumulative or refresh requests", async () => {
    const history = {
      ...stack,
      members: stack.members.map((member) => ({
        ...member,
        reason: "entry:recorded_comparisons:history",
      })),
    };
    const { calls } = setup(history);
    const { user } = mount();
    await screen.findByLabelText(/^Head/);
    expect(
      screen.getByRole("radio", { name: "Recorded comparison" }),
    ).toBeChecked();
    const whole = screen.getByRole("radio", { name: "Whole stack" });
    const refresh = screen.getByRole("button", { name: "Refresh stack refs" });
    expect(whole).toBeDisabled();
    expect(refresh).toBeDisabled();
    expect(refresh).toHaveAttribute(
      "title",
      "Refresh is unavailable for immutable recorded comparison history",
    );
    await user.click(whole);
    await user.click(refresh);
    await choose(user, "b");
    await screen.findAllByTitle(/Browsing branch-b/);
    const chat = screen.getByLabelText("Chat composer");
    await user.click(chat);
    expect(chat).toHaveFocus();
    expect(
      screen.getAllByText(/Recorded comparison history/).at(-1),
    ).toHaveAttribute("title", expect.stringContaining("Head: head"));
    expect(
      calls.filter((call) => /cumulative|refresh/.test(call.path)),
    ).toEqual([]);
  });

  it("retains a pending comment and exact retry identity across browse without activating execution", async () => {
    const { calls, comments } = setup();
    const { user } = mount();
    await screen.findByRole("combobox", { name: "Browse stack branch" });
    await user.click(
      await screen.findByRole("radio", { name: "Review", checked: false }),
    );
    const head = await screen.findByLabelText(/^Head/);
    head.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "m",
        code: "KeyM",
        ctrlKey: true,
        altKey: true,
        bubbles: true,
      }),
    );
    const draft = await screen.findByLabelText("New comment on src/a.ts");
    await user.type(draft, "unsent draft");
    await user.click(screen.getByRole("button", { name: "Comment" }));
    await screen.findByRole("button", { name: "Retry" });
    await choose(user, "b");
    await screen.findAllByTitle(/Browsing branch-b/);
    expect(
      screen.getAllByLabelText("Current execution member: branch-a")[0],
    ).toBeInTheDocument();
    const chat = screen.getByLabelText("Chat composer");
    await user.click(chat);
    await choose(user, "a");
    await user.click(chat);
    expect(await screen.findByLabelText("New comment on src/a.ts")).toHaveValue(
      "unsent draft",
    );
    expect(chat).toHaveFocus();
    await user.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(comments).toHaveLength(2));
    expect(comments[1]).toEqual(comments[0]);
    expect(calls.filter((c) => /context|activate/.test(c.path))).toEqual([]);
  });

  it("pins cumulative requests to the stack revision and offers no comment or anchor controls", async () => {
    const { calls } = setup();
    const { user } = mount();
    await screen.findByLabelText(/^Head/);
    expect(
      screen.getByRole("button", { name: "Refresh stack refs" }),
    ).toBeEnabled();
    await user.click(screen.getByRole("radio", { name: "Whole stack" }));
    const region = await screen.findByRole("region", {
      name: "Cumulative stack comparison",
    });
    await waitFor(() =>
      expect(calls.some((c) => c.path.endsWith("/cumulative/a/files"))).toBe(
        true,
      ),
    );
    expect(
      calls
        .filter((c) => c.path.includes("/cumulative/"))
        .every(
          (c) => new URLSearchParams(c.query).get("expected_revision") === "7",
        ),
    ).toBe(true);
    expect(
      within(region).queryByRole("button", { name: /comment/i }),
    ).not.toBeInTheDocument();
    expect(region.querySelector('[aria-label^="Comment on line"]')).toBeNull();
    expect(
      calls.filter(
        (c) => c.method !== "GET" && /anchors|comments|context/.test(c.path),
      ),
    ).toEqual([]);
  });

  it("renders a selected stack member without an active legacy review", async () => {
    const { calls } = setup();
    mount(null);
    await screen.findByLabelText(/^Head/);
    expect(
      screen.getByRole("combobox", { name: "Browse stack branch" }),
    ).toHaveTextContent("branch-a");
    expect(screen.getByTitle(/Browsing branch-a/)).toBeInTheDocument();
    expect(calls.filter((c) => /context|activate/.test(c.path))).toEqual([]);
  });

  it("retains cached member read-only on a failed refresh", async () => {
    const state = setup();
    const { store } = mount();
    await screen.findByLabelText(/^Head/);
    state.fail();
    store.dispatch(
      reviewApi.util.invalidateTags([{ type: "Review", id: "stack:chat-1" }]),
    );
    await screen.findByText(/Branch refresh failed/);
    const head = screen.getByLabelText(/^Head/);
    EditorView.findFromDOM(head)?.dispatch({
      selection: { anchor: 0, head: 2 },
    });
    expect(
      screen.queryByRole("button", { name: /Comment on Head/ }),
    ).not.toBeInTheDocument();
    head.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "m",
        code: "KeyM",
        ctrlKey: true,
        altKey: true,
        bubbles: true,
      }),
    );
    expect(
      screen.queryByLabelText("New comment on src/a.ts"),
    ).not.toBeInTheDocument();
    expect(state.calls.filter((c) => /anchors|comments/.test(c.path))).toEqual(
      [],
    );
  });
});
