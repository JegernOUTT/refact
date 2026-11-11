import { screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { render } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { setUpStore } from "../../app/store";
import {
  isStackManifest,
  isPreparedStack,
  isReviewHandoff,
  isReviewStackCumulative,
  reviewApi,
  invalidateReviewForChat,
  type StackManifest,
  type StackMember,
} from "../../services/refact/review";
import { ReviewStackNavigator } from "./ReviewStackNavigator";

function member(id: string, parent: string | null = null): StackMember {
  return {
    id,
    branch_ref: `branch-${id}`,
    parent_member_id: parent,
    declared_base_ref: parent ? `branch-${parent}` : "main",
    resolved_base_oid: "base",
    head_oid: "head",
    pr_identity: null,
    review_id: `review-${id}`,
    state: "ready",
    reason: null,
  };
}
function manifest(): StackManifest {
  return {
    schema_version: 1,
    id: "stack",
    owner_chat_id: "chat",
    revision: 2,
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
    members: [member("a"), member("b", "a"), member("c", "a")],
  };
}

describe("Review stack DTO guards", () => {
  it("validates nested members, scope and pending question rather than trusting arrays", () => {
    const stack = manifest();
    expect(isStackManifest(stack)).toBe(true);
    expect(isStackManifest({ ...stack, members: [{}] })).toBe(false);
    expect(isStackManifest({ ...stack, revision: NaN })).toBe(false);
    expect(
      isStackManifest({
        ...stack,
        scope_choice: { ...stack.scope_choice, scope: "all" },
      }),
    ).toBe(false);
    const prepared = {
      status: "pending_scope_choice",
      manifest: stack,
      question: {
        id: "scope",
        type: "single_select",
        text: "Choose",
        options: ["Only PR", "Whole stack"],
      },
      fingerprint: "digest",
      acquisition: "local only",
    };
    expect(isPreparedStack(prepared)).toBe(true);
    expect(
      isPreparedStack({
        ...prepared,
        question: { ...prepared.question, options: [true] },
      }),
    ).toBe(false);
    expect(
      isReviewHandoff({
        review_id: "r",
        revision: 2,
        stack_revision: null,
        markdown: "# Review",
        filename: "review.md",
        scope: "review",
      }),
    ).toBe(true);
    expect(isReviewHandoff({ review_id: "r", revision: 2 })).toBe(false);
    expect(
      isReviewStackCumulative({
        comparison_id: "c",
        stack_id: "s",
        stack_revision: 1,
        member_id: "a",
        read_only: true,
        annotation_target: null,
        candidate: {},
      }),
    ).toBe(false);
  });

  it("returns structured shape errors without leaking response data", async () => {
    server.use(
      http.get("*/v1/review-stacks/by-chat/chat", () =>
        HttpResponse.json({ private_payload: "secret-marker" }),
      ),
    );
    const store = setUpStore();
    const result = await store.dispatch(
      reviewApi.endpoints.getReviewStackByChat.initiate({ chatId: "chat" }),
    );
    expect(result.error).toMatchObject({
      status: "CUSTOM_ERROR",
      data: { code: "invalid_review_response" },
    });
    expect(JSON.stringify(result.error)).not.toContain("secret-marker");
    expect(invalidateReviewForChat("chat").payload).toEqual([
      { type: "Review", id: "chat:chat" },
      { type: "Review", id: "stack:chat" },
    ]);
    store.dispatch(reviewApi.util.resetApiState());
  });
});

describe("ReviewStackNavigator", () => {
  it("shows a captured stack by position and offers no stack comparison", async () => {
    const stack = manifest();
    stack.members[1].reason = "entry:recorded_comparisons:history";
    stack.members[1].declared_base_ref = "saved-base";
    stack.members[1].pr_identity = {
      host: "github.com",
      repository: "acme/widgets",
      number: 12,
    };
    const { user } = render(
      <ReviewStackNavigator
        stack={stack}
        selectedMemberId="b"
        onSelect={vi.fn()}
        comparison="cumulative"
        onComparisonChange={vi.fn()}
      />,
    );
    expect(screen.queryByRole("radio")).toBeNull();
    expect(screen.getByText("2 of 3")).toHaveAttribute(
      "title",
      expect.stringContaining("Base: saved-base (base)\nHead: head"),
    );
    expect(screen.getByRole("combobox")).toHaveTextContent("#12 branch-b");
    screen.getByRole("combobox").focus();
    await user.keyboard("{Enter}");
    expect(
      screen.getByRole("option", { name: /branch-b.*saved-base/ }),
    ).toBeInTheDocument();
  });

  it("browses forks without activating execution or making API requests", async () => {
    const requests = vi.fn();
    server.use(
      http.all("*/v1/review-stacks/*", () => {
        requests();
        return HttpResponse.json(null);
      }),
    );
    const onSelect = vi.fn();
    const onComparisonChange = vi.fn();
    const { user } = render(
      <ReviewStackNavigator
        stack={manifest()}
        selectedMemberId="b"
        onSelect={onSelect}
        comparison="parent"
        onComparisonChange={onComparisonChange}
        activeReviewId="review-a"
      />,
    );
    expect(
      screen.getByLabelText("Current execution member: branch-a"),
    ).toBeInTheDocument();
    screen.getByRole("combobox", { name: "Browse stack branch" }).focus();
    await user.keyboard("{Enter}");
    const b = screen.getByRole("option", { name: /branch-b.*branch-a/ });
    const c = screen.getByRole("option", { name: /branch-c.*branch-a/ });
    expect(b.style.getPropertyValue("--stack-depth")).toBe("1");
    expect(c.style.getPropertyValue("--stack-depth")).toBe("1");
    await user.click(c);
    expect(onSelect).toHaveBeenCalledWith("c");
    expect(screen.getByRole("combobox")).toHaveTextContent("branch-b");
    expect(
      screen.getByRole("radio", { name: "Parent-relative" }),
    ).toBeEnabled();
    expect(screen.getByRole("radio", { name: "Whole stack" })).toBeEnabled();
    await user.click(screen.getByRole("radio", { name: "Whole stack" }));
    expect(onComparisonChange).toHaveBeenCalledWith("cumulative");
    expect(requests).not.toHaveBeenCalled();
  });

  it("retains unavailable historical members and disables outside-scope branches", async () => {
    const stack = manifest();
    stack.scope_choice.scope = "single";
    const onSelect = vi.fn();
    const { user, rerender } = render(
      <ReviewStackNavigator
        stack={stack}
        selectedMemberId="a"
        onSelect={onSelect}
        comparison="parent"
        onComparisonChange={vi.fn()}
      />,
    );
    expect(screen.getByRole("radio", { name: "Whole stack" })).toBeDisabled();
    screen.getByRole("combobox").focus();
    await user.keyboard("{Enter}");
    const outside = screen.getByRole("option", {
      name: /branch-b.*outside selected scope/,
    });
    expect(outside).toHaveAttribute("aria-disabled", "true");
    expect(outside).toHaveAttribute(
      "title",
      expect.stringContaining("outside selected scope"),
    );
    await user.keyboard("{Escape}");
    stack.scope_choice.scope = "whole";
    stack.members[1].state = "unauthorized";
    stack.members[2].state = "removed";
    rerender(
      <ReviewStackNavigator
        stack={stack}
        selectedMemberId="a"
        onSelect={onSelect}
        comparison="parent"
        onComparisonChange={vi.fn()}
      />,
    );
    screen.getByRole("combobox").focus();
    await user.keyboard("{Enter}");
    expect(
      screen.getByRole("option", { name: /branch-b.*unauthorized/ }),
    ).toHaveAttribute("aria-disabled", "true");
    expect(
      screen.getByRole("option", { name: /branch-c.*Historical · removed/ }),
    ).toHaveAttribute("aria-disabled", "true");
    expect(onSelect).not.toHaveBeenCalled();
  });
});
