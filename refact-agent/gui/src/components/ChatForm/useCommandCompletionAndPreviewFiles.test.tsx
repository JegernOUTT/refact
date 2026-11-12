import { StrictMode } from "react";
import { act } from "react-dom/test-utils";
import { renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { useCommandCompletionAndPreviewFiles } from "./useCommandCompletionAndPreviewFiles";
import type { Checkboxes } from "./useCheckBoxes";

const queries = vi.hoisted(() => {
  vi.resetModules();
  return {
    completion: vi.fn(),
    preview: vi.fn(),
  };
});

vi.mock("../../services/refact/commands", () => ({
  commandsApi: {
    useGetCommandCompletionQuery: queries.completion,
    useGetCommandPreviewQuery: queries.preview,
  },
}));
vi.mock("../../hooks/useHasCaps", () => ({ useHasCaps: () => true }));
vi.mock("../../hooks/useAppSelector", () => ({ useAppSelector: () => [] }));
vi.mock("../../features/Chat/Thread", () => ({
  useThreadId: () => "chat-id",
  selectModelById: vi.fn(),
  selectThreadModeById: vi.fn(),
  selectThreadImagesById: vi.fn(),
}));

const checkboxes: Checkboxes = {
  selected_lines: {
    name: "selected_lines",
    label: "Selected lines",
    checked: false,
    disabled: false,
  },
};
const identity = (query: string) => query;

function mount(strict: boolean) {
  return renderHook(
    ({ addFilesToInput }) =>
      useCommandCompletionAndPreviewFiles(checkboxes, addFilesToInput),
    {
      initialProps: { addFilesToInput: identity },
      wrapper: strict
        ? ({ children }) => <StrictMode>{children}</StrictMode>
        : undefined,
    },
  );
}

function completionQuery() {
  return queries.completion.mock.lastCall?.[0] as
    | { query: string; cursor: number }
    | undefined;
}

function previewQuery() {
  const args = queries.preview.mock.lastCall?.[0] as
    | { messages: { content: string }[] }
    | undefined;
  return args?.messages[0].content;
}

describe.each([false, true])(
  "debounce lifecycle (StrictMode: %s)",
  (strict) => {
    beforeEach(() => {
      vi.useFakeTimers();
      queries.completion.mockReset().mockReturnValue({ data: undefined });
      queries.preview.mockReset().mockReturnValue({ data: undefined });
    });

    afterEach(() => {
      vi.clearAllTimers();
      vi.useRealTimers();
    });

    test("completion has a real trailing update and cancels the used timer after rerender/unmount", () => {
      const { result, rerender, unmount } = mount(strict);
      act(() => {
        vi.advanceTimersByTime(500);
      });
      act(() => result.current.requestCompletion("first", 1));
      expect(completionQuery()).toEqual({ query: "first", cursor: 1 });
      act(() => result.current.requestCompletion("trailing", 2));
      expect(completionQuery()?.query).toBe("first");
      act(() => {
        vi.advanceTimersByTime(499);
      });
      expect(completionQuery()?.query).toBe("first");
      act(() => {
        vi.advanceTimersByTime(1);
      });
      expect(completionQuery()).toEqual({ query: "trailing", cursor: 2 });
      act(() => {
        vi.advanceTimersByTime(500);
      });
      expect(vi.getTimerCount()).toBe(0);

      act(() => result.current.requestCompletion("next", 3));
      act(() => result.current.requestCompletion("pending", 4));
      expect(completionQuery()).toEqual({ query: "next", cursor: 3 });
      expect(vi.getTimerCount()).toBeGreaterThan(0);
      const requestCompletion = result.current.requestCompletion;
      rerender({ addFilesToInput: identity });
      expect(result.current.requestCompletion).toBe(requestCompletion);
      expect(vi.getTimerCount()).toBeGreaterThan(0);
      unmount();
      expect(vi.getTimerCount()).toBe(0);
      act(() => {
        vi.advanceTimersByTime(1000);
      });
      expect(vi.getTimerCount()).toBe(0);
    });

    test("preview has a real trailing update and cancels the used timer after rerender/unmount", () => {
      const { rerender, unmount } = mount(strict);
      act(() => {
        vi.advanceTimersByTime(500);
      });
      rerender({ addFilesToInput: () => "first" });
      expect(previewQuery()).toBe("first\n");
      rerender({ addFilesToInput: () => "trailing" });
      expect(previewQuery()).toBe("first\n");
      act(() => {
        vi.advanceTimersByTime(499);
      });
      expect(previewQuery()).toBe("first\n");
      act(() => {
        vi.advanceTimersByTime(1);
      });
      expect(previewQuery()).toBe("trailing\n");
      expect(vi.getTimerCount()).toBe(0);

      rerender({ addFilesToInput: () => "next" });
      rerender({ addFilesToInput: () => "pending" });
      expect(previewQuery()).toBe("next\n");
      expect(vi.getTimerCount()).toBeGreaterThan(0);
      rerender({ addFilesToInput: () => "pending" });
      expect(vi.getTimerCount()).toBeGreaterThan(0);
      unmount();
      expect(vi.getTimerCount()).toBe(0);
      act(() => {
        vi.advanceTimersByTime(1000);
      });
      expect(vi.getTimerCount()).toBe(0);
    });
  },
);
