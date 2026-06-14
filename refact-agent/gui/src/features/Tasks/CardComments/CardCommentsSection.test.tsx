import { describe, expect, it } from "vitest";
import { http, HttpResponse } from "msw";
import { render, screen, waitFor } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { CardCommentsSection } from "./CardCommentsSection";
import {
  useGetBoardQuery,
  type BoardCard,
  type CardComment,
  type TaskBoard,
} from "../../../services/refact/tasks";

HTMLElement.prototype.hasPointerCapture = () => false;

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "web" as const,
  },
};

const TASK_ID = "task-1";
const CARD_ID = "T-1";
const BOARD_ENDPOINT = `*/v1/tasks/${TASK_ID}/board`;
const COMMENT_ENDPOINT = `*/v1/tasks/${TASK_ID}/cards/${CARD_ID}/comments`;

interface CapturedRequest {
  method: string;
  pathname: string;
  body: unknown;
}

const makeComment = (overrides: Partial<CardComment> = {}): CardComment => ({
  id: "abcd1234xyz",
  author_role: "user",
  author_id: "user-abc123",
  timestamp: new Date(Date.now() - 3 * 60 * 60 * 1000).toISOString(),
  body: "This is a test comment.",
  reply_to: null,
  ...overrides,
});

const makeCard = (overrides: Partial<BoardCard> = {}): BoardCard => ({
  id: CARD_ID,
  title: "Card title",
  column: "planned",
  priority: "P1",
  depends_on: [],
  instructions: "",
  assignee: null,
  agent_chat_id: null,
  status_updates: [],
  final_report: null,
  created_at: "2026-06-15T00:00:00Z",
  started_at: null,
  completed_at: null,
  target_files: [],
  comments: [],
  ...overrides,
});

const makeBoard = (comments: CardComment[] = []): TaskBoard => ({
  schema_version: 1,
  rev: 2,
  columns: [],
  cards: [makeCard({ comments })],
});

async function captureRequest(request: Request): Promise<CapturedRequest> {
  return {
    method: request.method,
    pathname: new URL(request.url).pathname,
    body: await request.json(),
  };
}

function renderSection(comments: CardComment[] = []) {
  return render(
    <CardCommentsSection
      taskId={TASK_ID}
      cardId={CARD_ID}
      comments={comments}
    />,
    { preloadedState: CONFIG_STATE },
  );
}

function BoardBackedComments() {
  const { data } = useGetBoardQuery(TASK_ID);
  const card = data?.cards.find((candidate) => candidate.id === CARD_ID);
  return (
    <CardCommentsSection
      taskId={TASK_ID}
      cardId={CARD_ID}
      comments={card?.comments ?? []}
    />
  );
}

describe("CardCommentsSection", () => {
  it("renders_empty_state_when_no_comments", () => {
    renderSection([]);
    expect(screen.getByText("No comments yet.")).toBeInTheDocument();
    expect(screen.getByText("Comments (0)")).toBeInTheDocument();
  });

  it("renders_comments_with_author_role_badge_and_relative_timestamp", () => {
    const comment = makeComment();
    renderSection([comment]);
    expect(screen.getByText("user")).toBeInTheDocument();
    expect(screen.getByText("user-abc")).toBeInTheDocument();
    expect(screen.getByText(/ago/)).toBeInTheDocument();
    expect(screen.getByText("This is a test comment.")).toBeInTheDocument();
    expect(screen.getByText("Comments (1)")).toBeInTheDocument();
  });

  it("markdown_in_comment_body_renders_inline", () => {
    const comment = makeComment({ body: "**bold text** here" });
    renderSection([comment]);
    expect(screen.getByText("bold text")).toBeInTheDocument();
  });

  it("submit_button_disabled_until_body_non_empty", () => {
    renderSection([]);
    expect(screen.getByRole("button", { name: "Comment" })).toBeDisabled();
  });

  it("submit_posts_top_level_comment_to_canonical_endpoint_and_clears_composer", async () => {
    const requests: CapturedRequest[] = [];
    server.use(
      http.post(COMMENT_ENDPOINT, async ({ request }) => {
        requests.push(await captureRequest(request));
        return HttpResponse.json(makeBoard([makeComment()]));
      }),
    );

    const { user } = renderSection([]);
    const textarea = screen.getByPlaceholderText("Add a comment...");
    await user.type(textarea, " My new comment ");

    const button = screen.getByRole("button", { name: "Comment" });
    expect(button).not.toBeDisabled();
    await user.click(button);

    await waitFor(() => {
      expect(requests).toHaveLength(1);
    });
    expect(requests[0]).toStrictEqual({
      method: "POST",
      pathname: `/v1/tasks/${TASK_ID}/cards/${CARD_ID}/comments`,
      body: {
        body: "My new comment",
        author_role: "user",
      },
    });
    expect(textarea).toHaveValue("");
  });

  it("submit_reply_comment_carries_reply_to_only_when_replying", async () => {
    const requests: CapturedRequest[] = [];
    const parent = makeComment({ id: "parent-comment-id" });
    server.use(
      http.post(COMMENT_ENDPOINT, async ({ request }) => {
        requests.push(await captureRequest(request));
        return HttpResponse.json(makeBoard([parent]));
      }),
    );

    const { user } = renderSection([parent]);
    await user.click(screen.getByRole("button", { name: "Reply" }));
    await user.type(
      screen.getByPlaceholderText("Add a comment..."),
      "Reply body",
    );
    await user.click(screen.getByRole("button", { name: "Comment" }));

    await waitFor(() => {
      expect(requests).toHaveLength(1);
    });
    expect(requests[0]).toStrictEqual({
      method: "POST",
      pathname: `/v1/tasks/${TASK_ID}/cards/${CARD_ID}/comments`,
      body: {
        body: "Reply body",
        author_role: "user",
        reply_to: "parent-comment-id",
      },
    });
  });

  it("reply_button_sets_replyTo_and_shows_badge", async () => {
    const comment = makeComment({ id: "abcd1234xyz" });
    const { user } = renderSection([comment]);

    expect(screen.queryByText(/Replying to/)).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Reply" }));
    expect(screen.getByText(/Replying to abcd1234/)).toBeInTheDocument();
  });

  it("replies_render_indented_below_parent_without_nested_reply_action", () => {
    const parent = makeComment({ id: "parent-1", reply_to: null });
    const reply = makeComment({
      id: "reply-1",
      reply_to: "parent-1",
      body: "This is a reply.",
    });
    renderSection([parent, reply]);

    const replyText = screen.getByText("This is a reply.");
    const indented = replyText.closest("[class*='commentReply']");
    expect(indented).toBeTruthy();
    expect(screen.getAllByRole("button", { name: "Reply" })).toHaveLength(1);
  });

  it("submit_invalidates_board_cache_and_refreshes_comments", async () => {
    const createdComment = makeComment({
      id: "created-comment-id",
      author_id: null,
      body: "Fresh comment from refreshed board.",
    });
    const requests: CapturedRequest[] = [];
    let boardFetches = 0;
    server.use(
      http.get(BOARD_ENDPOINT, () => {
        boardFetches += 1;
        return HttpResponse.json(
          boardFetches === 1 ? makeBoard([]) : makeBoard([createdComment]),
        );
      }),
      http.post(COMMENT_ENDPOINT, async ({ request }) => {
        requests.push(await captureRequest(request));
        return HttpResponse.json(makeBoard([createdComment]));
      }),
    );

    const { user } = render(<BoardBackedComments />, {
      preloadedState: CONFIG_STATE,
    });
    await waitFor(() => {
      expect(boardFetches).toBe(1);
    });

    await user.type(
      screen.getByPlaceholderText("Add a comment..."),
      "Fresh comment from refreshed board.",
    );
    await user.click(screen.getByRole("button", { name: "Comment" }));

    await waitFor(() => {
      expect(requests).toHaveLength(1);
    });
    await screen.findByText("Fresh comment from refreshed board.");
    await waitFor(() => {
      expect(boardFetches).toBeGreaterThanOrEqual(2);
    });
  });

  it("submit_failure_shows_notification", async () => {
    server.use(
      http.post(COMMENT_ENDPOINT, () =>
        HttpResponse.json({ error: "Server error" }, { status: 500 }),
      ),
    );

    const { user } = renderSection([]);
    await user.type(screen.getByPlaceholderText("Add a comment..."), "Test");
    await user.click(screen.getByRole("button", { name: "Comment" }));

    await waitFor(() => {
      expect(screen.getByText(/Failed to add comment/)).toBeInTheDocument();
    });
  });

  it("whitespace_only_body_does_not_enable_submit", async () => {
    const { user } = renderSection([]);
    await user.type(screen.getByPlaceholderText("Add a comment..."), "   ");
    expect(screen.getByRole("button", { name: "Comment" })).toBeDisabled();
  });
});
