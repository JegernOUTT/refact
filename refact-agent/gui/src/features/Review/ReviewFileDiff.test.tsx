import { fireEvent, screen, waitFor } from "@testing-library/react";
import { EditorView } from "@codemirror/view";
import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";

import { render } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { ReviewFileDiff } from "./ReviewFileDiff";
import { bytes, makeReview } from "./reviewFixtures";

for (const side of ["base", "head"] as const) {
  describe(`${side}-only review`, () => {
    it("uses the populated editor without changing its canonical side or range", async () => {
      const review = makeReview();
      review.candidates[0].files[0][side === "base" ? "head" : "base"] = null;
      server.use(
        http.get("*/v1/reviews/r1/files", () =>
          HttpResponse.json({
            path: "src/a.ts",
            base: side === "base" ? bytes("removed\nline\n") : null,
            head: side === "head" ? bytes("added\nline\n") : null,
            diff: "",
          }),
        ),
      );
      const selected = vi.fn();
      const comment = vi.fn();
      const host = document.createElement("div");
      host.textContent = "Canonical discussion";
      const props = {
        review,
        path: "src/a.ts",
        diffMode: "side-by-side" as const,
        panes: {
          left: { candidateId: "c1", side: "base" as const },
          right: { candidateId: "c1", side: "head" as const },
        },
        activeObjectId: null,
        reveal: null,
        preferSingleSided: true,
        onSelection: selected,
        onCommentLine: comment,
        inlineHosts: {
          left: side === "base" ? [{ id: "thread", endLine: 1, host }] : [],
          right: side === "head" ? [{ id: "thread", endLine: 1, host }] : [],
        },
      };
      const { container, rerender } = render(<ReviewFileDiff {...props} />);
      const editor = await screen.findByRole("textbox", {
        name: side === "base" ? /^Base/ : /^Head/,
      });
      expect(container.querySelectorAll(".cm-editor")).toHaveLength(1);
      const view = EditorView.findFromDOM(editor);
      if (!view) throw new Error("Missing populated editor");
      expect(view.state.doc.line(1).text).toBe(
        side === "base" ? "removed" : "added",
      );
      view.dispatch({ selection: { anchor: 0, head: 3 } });
      expect(selected).toHaveBeenLastCalledWith(
        side === "base" ? "left" : "right",
        { startLine: 1, endLine: 1 },
      );
      fireEvent.keyDown(editor, {
        key: "m",
        code: "KeyM",
        ctrlKey: true,
        altKey: true,
      });
      expect(comment).toHaveBeenCalledWith(
        side === "base" ? "left" : "right",
        1,
      );
      expect(container.textContent).toContain("Canonical discussion");
      rerender(<ReviewFileDiff {...props} preferSingleSided={false} />);
      await waitFor(() =>
        expect(container.querySelectorAll(".cm-editor")).toHaveLength(2),
      );
    });
  });
}
