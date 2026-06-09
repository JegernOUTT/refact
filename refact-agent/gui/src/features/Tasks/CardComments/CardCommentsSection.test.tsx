import { describe, expect, it } from "vitest";
import { http, HttpResponse } from "msw";
import { render, screen, waitFor } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { CardCommentsSection } from "./CardCommentsSection";
import type { CardComment } from "../../../services/refact/tasks";

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

const makeComment = (overrides: Partial<CardComment> = {}): CardComment => ({
  id: "abcd1234xyz",
  author_role: "user",
  author_id: "user-abc123",
  timestamp: new Date(Date.now() - 3 * 60 * 60 * 1000).toISOString(),
  body: "This is a test comment.",
  reply_to: null,
  ...overrides,
});

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

const emptyBoardResponse = {
  schema_version: 1,
  rev: 2,
  columns: [],
  cards: [],
};

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

  it("submit_calls_add_comment_mutation_and_clears_composer", async () => {
    const requests: unknown[] = [];
    server.use(
      http.post(`*/v1/tasks/${TASK_ID}/board`, async ({ request }) => {
        requests.push(await request.json());
        return HttpResponse.json(emptyBoardResponse);
      }),
    );

    const { user } = renderSection([]);
    const textarea = screen.getByPlaceholderText("Add a comment...");
    await user.type(textarea, "My new comment");

    const button = screen.getByRole("button", { name: "Comment" });
    expect(button).not.toBeDisabled();
    await user.click(button);

    await waitFor(() => {
      expect(requests).toHaveLength(1);
    });
    expect(textarea).toHaveValue("");
  });

  it("reply_button_sets_replyTo_and_shows_badge", async () => {
    const comment = makeComment({ id: "abcd1234xyz" });
    const { user } = renderSection([comment]);

    expect(screen.queryByText(/Replying to/)).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Reply" }));
    expect(screen.getByText(/Replying to abcd1234/)).toBeInTheDocument();
  });

  it("replies_render_indented_below_parent", () => {
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
  });

  it("submit_failure_shows_notification", async () => {
    server.use(
      http.post(`*/v1/tasks/${TASK_ID}/board`, () =>
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
