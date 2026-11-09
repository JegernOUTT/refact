import { useEffect, useRef } from "react";
import { act } from "react-dom/test-utils";
import { http, HttpResponse } from "msw";
import {
  cleanup,
  fireEvent,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { EditorView } from "@codemirror/view";

import { server } from "../../utils/mockServer";
import {
  createDefaultChatState,
  render as renderWithProviders,
} from "../../utils/test-utils";
import type { AppStore } from "../../app/store";
import { applyChatEvent } from "../Chat/Thread";
import {
  invalidateReviewForChat,
  reviewApi,
  type Review,
  type StackManifest,
} from "../../services/refact/review";
import { ReviewChatSurface } from "./ReviewChatSurface";
import { anchor, bytes, candidate, makeReview } from "./reviewFixtures";

HTMLElement.prototype.hasPointerCapture = () => false;
HTMLElement.prototype.scrollIntoView = () => undefined;

let mounts = 0;
const stores: AppStore[] = [];

function render(...args: Parameters<typeof renderWithProviders>) {
  const chat = createDefaultChatState();
  const runtime = chat.threads[chat.current_thread_id];
  runtime.thread.id = "chat-1";
  chat.threads = { "chat-1": runtime };
  chat.current_thread_id = "chat-1";
  chat.open_thread_ids = ["chat-1"];
  const result = renderWithProviders(args[0], {
    ...args[1],
    preloadedState: { chat, ...args[1]?.preloadedState },
  });
  stores.push(result.store);
  return result;
}

afterEach(() => {
  cleanup();
  stores.splice(0).forEach((store) => {
    store.dispatch(reviewApi.util.resetApiState());
  });
});

function ChatProbe() {
  const ref = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    mounts += 1;
  }, []);
  return <textarea ref={ref} aria-label="Chat composer" />;
}

type Activity = Record<string, unknown>;

function serveReview(state: { review: Review | null; status?: number }) {
  const activity: Activity[] = [];
  server.use(
    http.get("*/v1/review-stacks/by-chat/chat-1", () =>
      HttpResponse.json(null),
    ),
    http.get("*/v1/reviews/by-chat/chat-1", () =>
      state.status
        ? HttpResponse.json(
            { detail: "chat unavailable" },
            {
              status: state.status,
            },
          )
        : HttpResponse.json(state.review),
    ),
    http.get("*/v1/reviews/r1/files", ({ request }) => {
      const params = new URL(request.url).searchParams;
      const id = params.get("candidate_id") ?? "";
      return HttpResponse.json({
        path: params.get("path"),
        base: bytes(`base ${id}\nline\n`),
        head: bytes(`head ${id}\nline\nmore\n`),
        diff: "",
      });
    }),
    http.get("*/v1/reviews/r1/delivery-status", () => HttpResponse.json([])),
    http.post("*/v1/reviews/r1/activity", async ({ request }) => {
      activity.push((await request.json()) as Activity);
      return HttpResponse.json({ ok: true });
    }),
  );
  return activity;
}

function twoCandidateReview(revision = 3): Review {
  const file = {
    path: "src/a.ts",
    base: { digest: "a0", size: 1 },
    head: { digest: "a1", size: 1 },
    base_mode: 1,
    head_mode: 1,
  };
  return makeReview({
    revision,
    candidates: [
      candidate("c1", 1, [file]),
      candidate("c2", 2, [{ ...file, head: { digest: "a2", size: 1 } }]),
    ],
    current_candidate_id: "c2",
  });
}

describe("ReviewChatSurface", () => {
  it("observes width, hides the divider in narrow layouts, and disconnects", async () => {
    serveReview({ review: twoCandidateReview() });
    let notify: ResizeObserverCallback | undefined;
    const disconnect = vi.fn();
    const original = globalThis.ResizeObserver;
    globalThis.ResizeObserver = class {
      constructor(private callback: ResizeObserverCallback) {}
      observe(element: Element) {
        if (element.className.includes("surfaceContainer"))
          notify = this.callback;
        return undefined;
      }
      unobserve() {
        return undefined;
      }
      disconnect = disconnect;
    };
    try {
      const { unmount } = render(
        <ReviewChatSurface chatId="chat-1">
          <ChatProbe />
        </ReviewChatSurface>,
      );
      await screen.findByRole("region", { name: "Review canvas" });
      const observer = {} as ResizeObserver;
      const entry = (width: number) => [
        { contentRect: { width } } as ResizeObserverEntry,
      ];
      notify?.(entry(1000), observer);
      await waitFor(() =>
        expect(
          screen.getByRole("separator", { name: "Resize chat" }),
        ).toBeInTheDocument(),
      );
      notify?.(entry(600), observer);
      await waitFor(() =>
        expect(
          screen.queryByRole("separator", { name: "Resize chat" }),
        ).not.toBeInTheDocument(),
      );
      unmount();
      expect(disconnect).toHaveBeenCalled();
    } finally {
      globalThis.ResizeObserver = original;
    }
  });
  it("preserves both trees, reader position and chat draft across focus and width changes", async () => {
    const review = twoCandidateReview();
    review.threads.push({
      id: "unplaced",
      anchors: [],
      annotation_id: null,
      contributions: [],
      resolved: false,
    });
    serveReview({ review });
    let notify: ResizeObserverCallback | undefined;
    const original = globalThis.ResizeObserver;
    globalThis.ResizeObserver = class {
      constructor(private callback: ResizeObserverCallback) {}
      observe(element: Element) {
        if (element.className.includes("surfaceContainer"))
          notify = this.callback;
      }
      unobserve() {
        return undefined;
      }
      disconnect() {
        return undefined;
      }
    };
    try {
      mounts = 0;
      const { user } = render(
        <ReviewChatSurface chatId="chat-1">
          <ChatProbe />
        </ReviewChatSurface>,
      );
      const canvas = await screen.findByRole("region", {
        name: "Review canvas",
      });
      const composer = screen.getByLabelText("Chat composer");
      await user.type(composer, "retained draft");
      const resize = (width: number) =>
        act(
          () =>
            notify?.(
              [{ contentRect: { width } } as ResizeObserverEntry],
              {} as ResizeObserver,
            ),
        );
      resize(390);
      expect(composer).toHaveFocus();
      expect(canvas.closest("[hidden]")).not.toBeNull();
      const canvasSwitch = screen.getByRole("radio", {
        name: "Canvas",
      });
      await user.click(canvasSwitch);
      expect(canvasSwitch).toHaveFocus();
      expect(canvas.closest("[hidden]")).toBeNull();
      const reader = canvas.querySelector<HTMLElement>(
        "[data-review-document]",
      );
      if (!reader) throw new Error("Missing reader");
      reader.scrollTop = 75;
      await user.click(screen.getByRole("radio", { name: "Chat" }));
      resize(1000);
      expect(screen.getByRole("region", { name: "Review canvas" })).toBe(
        canvas,
      );
      expect(screen.getByLabelText("Chat composer")).toBe(composer);
      expect(reader.scrollTop).toBe(75);
      expect(composer).toHaveValue("retained draft");
      expect(mounts).toBe(1);
      resize(390);
      fireEvent(
        window,
        new CustomEvent("refact:review-reveal", {
          detail: { reviewId: "other", candidateId: "c2", path: "src/a.ts" },
        }),
      );
      expect(screen.getByRole("radio", { name: "Chat" })).toBeChecked();
      for (const detail of [
        { reviewId: "r1" },
        { reviewId: "r1", threadId: "missing" },
        { reviewId: "r1", threadId: "unplaced" },
        {
          reviewId: "r1",
          anchor: { ...anchor("c2", "src/a.ts", "head", 1), status: "stalled" },
        },
        { reviewId: "r1", candidateId: "c2", path: "missing" },
      ]) {
        fireEvent(window, new CustomEvent("refact:review-reveal", { detail }));
        expect(screen.getByRole("radio", { name: "Chat" })).toBeChecked();
      }
      fireEvent(
        window,
        new CustomEvent("refact:review-reveal", {
          detail: { reviewId: "r1", candidateId: "c2", path: "src/a.ts" },
        }),
      );
      expect(screen.getByRole("radio", { name: "Canvas" })).toBeChecked();
      expect(canvas.closest("[hidden]")).toBeNull();
    } finally {
      globalThis.ResizeObserver = original;
    }
  });
  it("reveals a since-last BASE fallback in Split and allows returning to Unified", async () => {
    const review = twoCandidateReview();
    review.threads = [
      {
        id: "base-thread",
        annotation_id: null,
        anchors: [anchor("c2", "src/a.ts", "base", 1)],
        contributions: [],
        resolved: false,
      },
    ];
    serveReview({ review });
    const { user } = render(
      <ReviewChatSurface chatId="chat-1">
        <ChatProbe />
      </ReviewChatSurface>,
    );
    await user.click(await screen.findByRole("radio", { name: "Review" }));
    await screen.findByLabelText(/^Head/);
    await user.click(screen.getByRole("radio", { name: "Since last" }));
    await user.click(screen.getByRole("radio", { name: "Unified" }));
    const fallback = await screen.findByRole("button", {
      name: "Comment thread",
    });
    await user.click(fallback);
    await waitFor(() =>
      expect(screen.getByRole("radio", { name: "Split" })).toBeChecked(),
    );
    await user.click(screen.getByRole("radio", { name: "Unified" }));
    await waitFor(() =>
      expect(screen.getByRole("radio", { name: "Unified" })).toBeChecked(),
    );
  });
  it("retains a late resolved draft without focusing it after candidate navigation", async () => {
    serveReview({ review: twoCandidateReview() });
    let release: () => void = () => undefined;
    const ready = new Promise<void>((resolve) => {
      release = resolve;
    });
    let requested = false;
    server.use(
      http.post("*/v1/reviews/r1/anchors", async () => {
        requested = true;
        await ready;
        return HttpResponse.json(anchor("c2", "src/a.ts", "head", 1));
      }),
    );
    const { user } = render(
      <ReviewChatSurface chatId="chat-1">
        <ChatProbe />
      </ReviewChatSurface>,
    );
    await user.click(await screen.findByRole("radio", { name: "Review" }));
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
    await waitFor(() => expect(requested).toBe(true));
    await user.click(
      screen.getByRole("combobox", { name: "Candidate revision" }),
    );
    await user.click(await screen.findByRole("option", { name: /#1/ }));
    const chat = screen.getByLabelText("Chat composer");
    await user.click(chat);
    release();
    await waitFor(() =>
      expect(
        screen.queryByLabelText("New comment on src/a.ts"),
      ).not.toBeInTheDocument(),
    );
    await user.click(screen.getByRole("button", { name: "Back to current" }));
    await screen.findByLabelText("New comment on src/a.ts");
    expect(screen.getByLabelText("New comment on src/a.ts")).not.toHaveFocus();
  });
  it("waits for the server snapshot before requesting the review", async () => {
    let requests = 0;
    server.use(
      http.get("*/v1/reviews/by-chat/chat-1", () => {
        requests += 1;
        return HttpResponse.json(null);
      }),
    );
    const chat = createDefaultChatState();
    const runtime = chat.threads[chat.current_thread_id];
    runtime.thread.id = "chat-1";
    runtime.snapshot_received = false;
    chat.threads = { "chat-1": runtime };
    chat.current_thread_id = "chat-1";
    chat.open_thread_ids = ["chat-1"];
    const { store } = render(
      <ReviewChatSurface chatId="chat-1">
        <ChatProbe />
      </ReviewChatSurface>,
      { preloadedState: { chat } },
    );
    expect(requests).toBe(0);
    expect(
      reviewApi.endpoints.getReviewByChat.select({ chatId: "chat-1" })(
        store.getState(),
      ).isUninitialized,
    ).toBe(true);
    store.dispatch(
      applyChatEvent({
        chat_id: "chat-1",
        seq: "1",
        type: "snapshot",
        thread: {
          id: "chat-1",
          title: "Test",
          model: "",
          mode: "AGENT",
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
    await waitFor(() => expect(requests).toBe(1));
  });

  it("renders only the chat when the chat owns no review", async () => {
    serveReview({ review: null });
    const { store } = render(
      <ReviewChatSurface chatId="chat-1">
        <ChatProbe />
      </ReviewChatSurface>,
    );
    expect(screen.getByLabelText("Chat composer")).toBeInTheDocument();
    await waitFor(() =>
      expect(
        reviewApi.endpoints.getReviewByChat.select({ chatId: "chat-1" })(
          store.getState(),
        ).isSuccess,
      ).toBe(true),
    );
    await waitFor(() =>
      expect(
        screen.queryByRole("region", { name: "Review canvas" }),
      ).not.toBeInTheDocument(),
    );
  });

  it("opens the canvas on activation without remounting the chat or moving focus", async () => {
    const state: { review: Review | null } = { review: null };
    serveReview(state);
    mounts = 0;
    const { user, store } = render(
      <ReviewChatSurface chatId="chat-1">
        <ChatProbe />
      </ReviewChatSurface>,
    );
    const composer = screen.getByLabelText("Chat composer");
    await user.click(composer);
    await user.type(composer, "draft in progress");

    state.review = twoCandidateReview();
    store.dispatch(invalidateReviewForChat("chat-1"));
    await screen.findByRole("region", { name: "Review canvas" });

    state.review = {
      ...twoCandidateReview(4),
      annotations: [
        {
          id: "an1",
          kind: "finding",
          title: "Unchecked index",
          body: "May overflow.",
          anchors: [anchor("c2", "src/a.ts", "head", 2)],
          severity: "high",
          confidence: "medium",
          assessment: "supported",
          disposition: "open",
          applicability: "current",
          evidence_ids: [],
          run_id: null,
        },
      ],
    };
    store.dispatch(invalidateReviewForChat("chat-1"));
    await screen.findByText("Unchecked index");

    expect(mounts).toBe(1);
    expect(screen.getByLabelText("Chat composer")).toBe(composer);
    expect(composer).toHaveFocus();
    expect(composer).toHaveValue("draft in progress");
  });

  it("discovers a tool-imported stack while mounted without a review event or losing the chat draft", async () => {
    const state: { review: Review | null } = { review: null };
    serveReview(state);
    const manifest: StackManifest = {
      schema_version: 1,
      id: "imported-stack",
      owner_chat_id: "chat-1",
      revision: 1,
      title: "Imported stack",
      repository: "/repo",
      source: { kind: "provided_manifest", reference: "user" },
      root_base_ref: "main",
      root_base_oid: "base",
      scope_choice: {
        scope: "whole",
        selected_member_id: "member-0",
        confirmed: true,
        explicit_request: true,
      },
      members: Array.from({ length: 15 }, (_, index) => ({
        id: `member-${index}`,
        branch_ref: `branch-${index}`,
        parent_member_id: index ? `member-${index - 1}` : null,
        declared_base_ref: index ? `branch-${index - 1}` : "main",
        resolved_base_oid: "base",
        head_oid: "head",
        pr_identity: null,
        review_id: index === 0 ? "r1" : null,
        state: "ready",
        reason: null,
      })),
    };
    let imported = false;
    let stackRequests = 0;
    let memberRequests = 0;
    const mutations: string[] = [];
    server.use(
      http.get("*/v1/review-stacks/by-chat/chat-1", () => {
        stackRequests += 1;
        return HttpResponse.json(imported ? manifest : null);
      }),
      http.get("*/v1/review-stacks/by-chat/chat-1/members/member-0", () => {
        memberRequests += 1;
        return HttpResponse.json(state.review);
      }),
      http.post("*/v1/review-stacks/by-chat/chat-1/*", ({ request }) => {
        mutations.push(request.url);
        return HttpResponse.json({ ok: true });
      }),
    );
    mounts = 0;
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      const { user, store } = render(
        <ReviewChatSurface chatId="chat-1">
          <ChatProbe />
        </ReviewChatSurface>,
      );
      await vi.waitFor(() =>
        expect(
          reviewApi.endpoints.getReviewStackByChat.select({ chatId: "chat-1" })(
            store.getState(),
          ).data,
        ).toBeNull(),
      );
      const composer = screen.getByLabelText("Chat composer");
      fireEvent.change(composer, { target: { value: "draft in progress" } });
      composer.focus();
      expect(stackRequests).toBe(1);
      expect(
        screen.queryByRole("combobox", { name: "Browse stack branch" }),
      ).toBeNull();

      // The engine imports the manifest and captures its root without a GUI
      // mutation or review-bearing SSE event. No manual invalidation/refetch.
      imported = true;
      state.review = twoCandidateReview();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(15000);
      });
      await vi.waitFor(() =>
        expect(
          screen.getByRole("combobox", { name: "Browse stack branch" }),
        ).toBeInTheDocument(),
      );
      await vi.waitFor(() => expect(memberRequests).toBe(1));
      expect(mounts).toBe(1);
      expect(screen.getByLabelText("Chat composer")).toBe(composer);
      expect(composer).toHaveFocus();
      expect(composer).toHaveValue("draft in progress");

      // Manifest polling must not invalidate/refetch the member review cache.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(15000);
      });
      await vi.waitFor(() => expect(stackRequests).toBeGreaterThanOrEqual(3));
      expect(memberRequests).toBe(1);
      vi.useRealTimers();
      await user.click(
        screen.getByRole("combobox", { name: "Browse stack branch" }),
      );
      const options = await screen.findAllByRole("option");
      expect(options).toHaveLength(15);
      options.forEach((option, index) => {
        expect(option).toHaveTextContent(`branch-${index}`);
      });
      expect(mutations).toEqual([]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("reports the historical candidate actually being viewed", async () => {
    const activity = serveReview({ review: twoCandidateReview() });
    const { user } = render(
      <ReviewChatSurface chatId="chat-1">
        <ChatProbe />
      </ReviewChatSurface>,
    );
    await screen.findByRole("region", { name: "Review canvas" });
    await waitFor(() => expect(activity.at(-1)?.candidate_id).toBe("c2"));
    const revision = screen.getByRole("combobox", {
      name: "Candidate revision",
    });
    await waitFor(() =>
      expect(revision).toHaveTextContent(/^#2 headc200 \(current\)$/),
    );

    await user.click(
      screen.getByRole("combobox", { name: "Candidate revision" }),
    );
    await user.click(await screen.findByRole("option", { name: /#1/ }));

    expect(await screen.findByText(/Historical headc100/)).toBeInTheDocument();
    await waitFor(() => expect(activity.at(-1)?.candidate_id).toBe("c1"));
    expect(activity.at(-1)?.path).toBe("src/a.ts");
    expect(revision).toHaveTextContent(/^#1 headc100$/);
    await user.click(screen.getByRole("button", { name: "Back to current" }));
    await waitFor(() =>
      expect(revision).toHaveTextContent(/^#2 headc200 \(current\)$/),
    );
  });

  it("shows distinct basenames while retaining full paths and changed-file ordering", async () => {
    const paths = [
      "refact-agent/engine/src/review/api.rs",
      "refact-agent/engine/src/review/model.rs",
      "refact-agent/engine/src/review/storage.rs",
      "refact-agent/engine/src/review/unchanged.rs",
    ];
    const review = makeReview({
      target_reference:
        "https://app.graphite.dev/github/pr/owner/repository/861",
      candidates: [
        candidate(
          "c1",
          1,
          paths.map((path, index) => ({
            path,
            base: { digest: "base", size: 1 },
            head: { digest: index === 3 ? "base" : "head", size: 1 },
            base_mode: 1,
            head_mode: 1,
          })),
        ),
      ],
    });
    serveReview({ review });
    const { user } = render(
      <ReviewChatSurface chatId="chat-1">
        <ChatProbe />
      </ReviewChatSurface>,
    );
    await user.click(await screen.findByRole("radio", { name: "Review" }));
    const list = await screen.findByRole("list", { name: "Changed files" });
    expect(
      within(list)
        .getAllByRole("button")
        .map((button) => button.getAttribute("aria-label")),
    ).toEqual(paths.slice(0, 3));
    for (const path of paths.slice(0, 3)) {
      const button = within(list).getByRole("button", {
        name: path,
      });
      expect(button).toHaveAttribute("title", path);
      expect(
        within(button).getByText(path.split("/").at(-1) ?? "", { exact: true }),
      ).toBeVisible();
      expect(
        within(button).getByText("…/src/review", { exact: true }),
      ).toBeVisible();
    }
    const reference = screen.getByRole("link", {
      name: "app.graphite.dev #861",
    });
    expect(reference).toHaveAttribute("href", review.target_reference);
    expect(reference).toHaveAttribute("title", review.target_reference);
    expect(
      screen.getByLabelText(
        "Base basec10000000, head headc10000000, current candidate c1",
      ),
    ).toHaveTextContent("basec100 → headc100");
    await user.click(
      screen.getByRole("button", { name: "More review options" }),
    );
    await user.click(
      screen.getByRole("menuitem", { name: "Show unchanged files" }),
    );
    expect(
      within(list)
        .getAllByRole("button")
        .map((button) => button.getAttribute("aria-label")),
    ).toEqual(paths);
    await user.click(within(list).getByRole("button", { name: paths[2] }));
    expect(
      within(list).getByRole("button", { name: paths[2] }),
    ).toHaveAttribute("aria-current", "true");
  });

  it("keeps the last loaded review read-only when refresh fails", async () => {
    const state: { review: Review | null; status?: number } = {
      review: twoCandidateReview(),
    };
    serveReview(state);
    mounts = 0;
    const { store } = render(
      <ReviewChatSurface chatId="chat-1">
        <ChatProbe />
      </ReviewChatSurface>,
    );
    await screen.findByRole("region", { name: "Review canvas" });

    state.status = 404;
    store.dispatch(invalidateReviewForChat("chat-1"));
    expect(
      await screen.findByText(/Review unavailable: chat unavailable/),
    ).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Review canvas" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Retry" })).toBeEnabled();
    expect(mounts).toBe(1);
  });

  it("anchors a since-last left-pane selection to the previous candidate head", async () => {
    serveReview({ review: twoCandidateReview() });
    const anchorRequests: Activity[] = [];
    server.use(
      http.post("*/v1/reviews/r1/anchors", async ({ request }) => {
        const body = (await request.json()) as Activity;
        anchorRequests.push(body);
        return HttpResponse.json(
          anchor(
            String(body.candidate_id),
            String(body.path),
            body.side === "base" ? "base" : "head",
            Number(body.start_line),
            Number(body.end_line),
          ),
        );
      }),
    );
    const { user } = render(
      <ReviewChatSurface chatId="chat-1">
        <ChatProbe />
      </ReviewChatSurface>,
    );
    await screen.findByRole("region", { name: "Review canvas" });
    await user.click(screen.getByRole("radio", { name: "Review" }));
    await user.click(screen.getByRole("radio", { name: "Since last" }));
    const leftContent = await screen.findByLabelText(/^Candidate head/);
    const leftView = EditorView.findFromDOM(leftContent);
    expect(leftView?.state.doc.line(1).text).toBe("head c1");
    leftView?.dispatch({ selection: { anchor: 0, head: 3 } });

    const comment = await screen.findByRole("button", {
      name: /Comment on Head L1/,
    });
    await user.click(comment);
    await waitFor(() => expect(anchorRequests).toHaveLength(1));
    expect(anchorRequests[0]).toMatchObject({
      candidate_id: "c1",
      side: "head",
      path: "src/a.ts",
      start_line: 1,
    });
  });

  it("never shows the previous file's text under a newly selected file", async () => {
    const review = makeReview({
      candidates: [
        candidate("c1", 1, [
          {
            path: "src/a.ts",
            base: { digest: "a0", size: 1 },
            head: { digest: "a1", size: 1 },
            base_mode: 1,
            head_mode: 1,
          },
          {
            path: "src/b.ts",
            base: { digest: "b0", size: 1 },
            head: { digest: "b1", size: 1 },
            base_mode: 1,
            head_mode: 1,
          },
        ]),
      ],
    });
    serveReview({ review });
    let releaseB: () => void = () => undefined;
    const bReady = new Promise<void>((resolve) => {
      releaseB = resolve;
    });
    let requestedB = false;
    const anchorRequests: Activity[] = [];
    server.use(
      http.get("*/v1/reviews/r1/files", async ({ request }) => {
        const path = new URL(request.url).searchParams.get("path") ?? "";
        if (path === "src/b.ts") {
          requestedB = true;
          await bReady;
        }
        return HttpResponse.json({
          path,
          base: bytes(`old ${path}\n`),
          head: bytes(`new ${path}\n`),
          diff: "",
        });
      }),
      http.post("*/v1/reviews/r1/anchors", async ({ request }) => {
        anchorRequests.push((await request.json()) as Activity);
        return HttpResponse.json(anchor("c1", "src/b.ts", "head", 1));
      }),
    );
    const { user } = render(
      <ReviewChatSurface chatId="chat-1">
        <ChatProbe />
      </ReviewChatSurface>,
    );
    await user.click(await screen.findByRole("radio", { name: "Review" }));
    const firstHead = await screen.findByLabelText(/^Head/);
    expect(EditorView.findFromDOM(firstHead)?.state.doc.toString()).toBe(
      "new src/a.ts\n",
    );

    await user.click(screen.getByRole("button", { name: /src\/b\.ts/ }));
    try {
      await waitFor(() => expect(requestedB).toBe(true));
      expect(screen.getByText("Loading file")).toBeInTheDocument();
      expect(
        screen.queryByTestId("review-diff-editors"),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: /Comment on/ }),
      ).not.toBeInTheDocument();
      expect(anchorRequests).toHaveLength(0);
    } finally {
      releaseB();
    }
    const nextHead = await screen.findByLabelText(/^Head/);
    await waitFor(() =>
      expect(EditorView.findFromDOM(nextHead)?.state.doc.toString()).toBe(
        "new src/b.ts\n",
      ),
    );
  });

  it.each(["navigation", "document scrolling"])(
    "leaves follow-tour when the user steps manually in %s without stealing focus",
    async (surface) => {
      const review = {
        ...twoCandidateReview(),
        tour: ["s1", "s2", "s3"].map((id, index) => ({
          id,
          current: {
            version: 1,
            title: `Step ${index + 1}`,
            body: "",
            anchors: [anchor("c2", "src/a.ts", "head", 1)],
          },
          history: [],
        })),
      };
      const state = { review };
      serveReview(state);
      const { user, store } = render(
        <ReviewChatSurface chatId="chat-1">
          <ChatProbe />
        </ReviewChatSurface>,
      );
      await screen.findByLabelText(/^Head/);
      await user.click(
        await screen.findByRole("button", { name: "Follow tour" }),
      );
      expect(
        screen.getByRole("button", { name: "Stop following" }),
      ).toBeInTheDocument();
      await waitFor(() =>
        expect(screen.getByRole("button", { name: /Step 3/ })).toHaveAttribute(
          "aria-current",
          "true",
        ),
      );

      const composer = screen.getByLabelText("Chat composer");
      await user.click(composer);
      if (surface === "navigation")
        fireEvent.click(
          within(screen.getByRole("list", { name: "Tour steps" })).getByRole(
            "button",
            { name: /Step 2/ },
          ),
        );
      else {
        const doc = document.querySelector<HTMLElement>(
          "[data-review-document]",
        );
        if (!doc) throw new Error("Missing living document");
        fireEvent.wheel(doc);
      }
      expect(
        screen.getByRole("button", { name: "Follow tour" }),
      ).toHaveAttribute("aria-pressed", "false");
      expect(
        screen.getByRole("button", {
          name: surface === "navigation" ? /Step 2/ : /Step 3/,
        }),
      ).toHaveAttribute("aria-current", "true");
      expect(composer).toHaveFocus();

      state.review = {
        ...review,
        revision: review.revision + 1,
        tour: [
          ...review.tour,
          {
            ...review.tour[2],
            id: "s4",
            current: {
              ...review.tour[2].current,
              title: "Step 4",
            },
          },
        ],
      };
      store.dispatch(invalidateReviewForChat("chat-1"));
      await screen.findByRole("button", { name: /Step 4/ });
      expect(
        screen.getByRole("button", {
          name: surface === "navigation" ? /Step 2/ : /Step 3/,
        }),
      ).toHaveAttribute("aria-current", "true");
      expect(composer).toHaveFocus();

      expect(
        screen.getByRole("heading", { name: "Step 1" }),
      ).toBeInTheDocument();
      expect(
        screen.getByRole("heading", { name: "Step 2" }),
      ).toBeInTheDocument();
      expect(
        Array.from(
          screen.getByRole("region", { name: "Review canvas" }).children,
        ).filter((child) => child.tagName === "HEADER").length,
      ).toBe(1);

      fireEvent.click(screen.getByRole("button", { name: "Follow tour" }));
      await waitFor(() =>
        expect(screen.getByRole("button", { name: /Step 4/ })).toHaveAttribute(
          "aria-current",
          "true",
        ),
      );
      state.review = {
        ...state.review,
        revision: state.review.revision + 1,
        tour: [
          ...state.review.tour,
          {
            ...review.tour[2],
            id: "s5",
            current: {
              ...review.tour[2].current,
              title: "Step 5",
            },
          },
        ],
      };
      store.dispatch(invalidateReviewForChat("chat-1"));
      await waitFor(() =>
        expect(screen.getByRole("button", { name: /Step 5/ })).toHaveAttribute(
          "aria-current",
          "true",
        ),
      );
      expect(
        screen.getByRole("button", { name: "Stop following" }),
      ).toHaveAttribute("aria-pressed", "true");
      expect(composer).toHaveFocus();
    },
  );
});
