import { describe, expect, it } from "vitest";
import {
  isTaskDocumentDetail,
  isTaskDocumentSummary,
  isTaskDocumentHistoryResponse,
} from "./taskDocumentsApi";

const validDetail = {
  slug: "main-plan",
  name: "Main Plan",
  kind: "plan",
  content: "# Plan\n\nDo stuff.",
  version: 3,
  pinned: true,
  created_at: "2026-05-22T00:00:00Z",
  updated_at: "2026-05-23T00:00:00Z",
  author_role: "planner",
  relevant_cards: [],
};

describe("isTaskDocumentDetail", () => {
  it("transform_response_accepts_valid_shape", () => {
    expect(isTaskDocumentDetail(validDetail)).toBe(true);
  });

  it("transform_response_throws_on_missing_slug", () => {
    const { slug: _slug, ...withoutSlug } = validDetail;
    expect(isTaskDocumentDetail(withoutSlug)).toBe(false);
  });

  it("rejects null", () => {
    expect(isTaskDocumentDetail(null)).toBe(false);
  });

  it("rejects when version is a string", () => {
    expect(isTaskDocumentDetail({ ...validDetail, version: "3" })).toBe(false);
  });

  it("rejects when content is missing", () => {
    const { content: _content, ...withoutContent } = validDetail;
    expect(isTaskDocumentDetail(withoutContent)).toBe(false);
  });
});

describe("isTaskDocumentSummary", () => {
  it("accepts valid summary", () => {
    const summary = {
      slug: "main-plan",
      name: "Main Plan",
      kind: "plan",
      pinned: false,
      version: 1,
      updated_at: "2026-05-22T00:00:00Z",
      created_at: "2026-05-21T00:00:00Z",
      author_role: "planner",
      relevant_cards: [],
    };
    expect(isTaskDocumentSummary(summary)).toBe(true);
  });

  it("rejects when slug is missing", () => {
    expect(isTaskDocumentSummary({ name: "x", kind: "plan" })).toBe(false);
  });
});

describe("isTaskDocumentHistoryResponse", () => {
  it("accepts valid history response", () => {
    expect(
      isTaskDocumentHistoryResponse({
        task_id: "task-1",
        slug: "main-plan",
        history: [],
      }),
    ).toBe(true);
  });

  it("rejects when history is not an array", () => {
    expect(
      isTaskDocumentHistoryResponse({
        task_id: "task-1",
        slug: "main-plan",
        history: null,
      }),
    ).toBe(false);
  });
});
