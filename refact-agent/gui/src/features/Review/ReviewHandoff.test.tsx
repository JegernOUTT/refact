import { screen, waitFor } from "@testing-library/react";
import { act } from "react-dom/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "../../utils/test-utils";
import { ReviewHandoff } from "./ReviewHandoff";
import {
  copyReviewMarkdown,
  downloadReviewMarkdown,
  type HandoffExport,
} from "./reviewHandoffClipboard";

const data: HandoffExport = {
  review_id: "r1",
  revision: 3,
  stack_revision: 8,
  markdown: "# Handoff\n<script>inert()</script>",
  filename: "review-r1.md",
  scope: "whole_stack",
};
afterEach(() => {
  vi.restoreAllMocks();
});
function clipboard(writeText: ReturnType<typeof vi.fn>) {
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText },
  });
}

describe("Review Markdown handoff", () => {
  it("shows success only after the actual async write resolves", async () => {
    const { user } = render(
      <ReviewHandoff
        reviewId="r1"
        revision={3}
        load={() => Promise.resolve(data)}
      />,
    );
    let resolve!: () => void;
    const write = vi.fn(
      () =>
        new Promise<void>((r) => {
          resolve = r;
        }),
    );
    clipboard(write);
    await user.click(
      screen.getByRole("button", { name: "Copy review as Markdown" }),
    );
    await waitFor(() => expect(write).toHaveBeenCalledWith(data.markdown));
    expect(
      screen.queryByText("Review Markdown copied"),
    ).not.toBeInTheDocument();
    resolve();
    expect(
      await screen.findByText("Review Markdown copied"),
    ).toBeInTheDocument();
  });

  it("offers full inert manual copy and download after denied clipboard, with no false success", async () => {
    const { user } = render(
      <ReviewHandoff
        reviewId="r1"
        revision={3}
        load={() => Promise.resolve(data)}
      />,
    );
    clipboard(vi.fn().mockRejectedValue(new Error("denied")));
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: vi.fn(() => false),
    });
    await user.click(
      screen.getByRole("button", { name: "Copy review as Markdown" }),
    );
    await screen.findByRole("dialog");
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
    await user.click(
      screen.getByRole("button", {
        name: "Show full Markdown for manual copy",
      }),
    );
    const field = await screen.findByRole("textbox", {
      name: "Review Markdown",
    });
    expect(field).toHaveValue(data.markdown);
    expect(field).toHaveAttribute("readonly");
    expect(
      screen.queryByText("Review Markdown copied"),
    ).not.toBeInTheDocument();
    expect(document.querySelector("script")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Select all" }));
    expect(field).toHaveFocus();
    expect((field as HTMLTextAreaElement).selectionEnd).toBe(
      data.markdown.length,
    );
    expect(
      screen.getByRole("button", { name: "Download .md" }),
    ).toBeInTheDocument();
  });

  it("does not copy a late response after review identity changes", async () => {
    let resolve!: (value: HandoffExport) => void;
    const load = vi.fn(
      () =>
        new Promise<HandoffExport>((r) => {
          resolve = r;
        }),
    );
    const { user, rerender } = render(
      <ReviewHandoff reviewId="r1" revision={3} load={load} />,
    );
    const write = vi.fn().mockResolvedValue(undefined);
    clipboard(write);
    await user.click(
      screen.getByRole("button", { name: "Copy review as Markdown" }),
    );
    rerender(<ReviewHandoff reviewId="r2" revision={3} load={load} />);
    resolve(data);
    await Promise.resolve();
    expect(write).not.toHaveBeenCalled();
  });

  it("does not copy a response after unmount", async () => {
    let resolve!: (value: HandoffExport) => void;
    const { user, unmount } = render(
      <ReviewHandoff
        reviewId="r1"
        revision={3}
        load={() =>
          new Promise((r) => {
            resolve = r;
          })
        }
      />,
    );
    const write = vi.fn();
    clipboard(write);
    await user.click(
      screen.getByRole("button", { name: "Copy review as Markdown" }),
    );
    unmount();
    resolve(data);
    await Promise.resolve();
    expect(write).not.toHaveBeenCalled();
  });

  it("accepts a newer coherent snapshot while rejecting stale state and load errors", async () => {
    const { user, rerender } = render(
      <ReviewHandoff
        reviewId="r1"
        revision={2}
        load={() => Promise.resolve(data)}
      />,
    );
    const write = vi.fn().mockResolvedValue(undefined);
    clipboard(write);
    await user.click(
      screen.getByRole("button", { name: "Copy review as Markdown" }),
    );
    expect(
      await screen.findByText("Review Markdown copied"),
    ).toBeInTheDocument();
    rerender(
      <ReviewHandoff
        reviewId="r1"
        revision={4}
        load={() => Promise.resolve(data)}
      />,
    );
    await user.click(
      screen.getByRole("button", { name: "Copy review as Markdown" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "review changed",
    );
    expect(write).toHaveBeenCalledTimes(1);
    rerender(
      <ReviewHandoff
        reviewId="r1"
        revision={5}
        disabled
        load={() => Promise.resolve(data)}
      />,
    );
    expect(
      screen.getByRole("button", { name: "Copy review as Markdown" }),
    ).toBeDisabled();
  });

  it.each(["identity", "unmount"])(
    "ignores pending clipboard completion after %s changes",
    async (change) => {
      let resolve!: () => void;
      const { user, rerender, unmount } = render(
        <ReviewHandoff
          reviewId="r1"
          revision={3}
          load={() => Promise.resolve(data)}
        />,
      );
      clipboard(
        vi.fn(
          () =>
            new Promise<void>((done) => {
              resolve = done;
            }),
        ),
      );
      await user.click(
        screen.getByRole("button", { name: "Copy review as Markdown" }),
      );
      await waitFor(() => expect(resolve).toBeTypeOf("function"));
      expect(
        screen.queryByText("Review Markdown copied"),
      ).not.toBeInTheDocument();
      if (change === "unmount") unmount();
      else
        rerender(
          <ReviewHandoff
            reviewId="r2"
            revision={3}
            load={() => Promise.resolve(data)}
          />,
        );
      await act(async () => {
        resolve();
        await Promise.resolve();
      });
      expect(
        screen.queryByText("Review Markdown copied"),
      ).not.toBeInTheDocument();
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    },
  );

  it("verifies DOM copy result and restores focus", async () => {
    clipboard(vi.fn().mockRejectedValue(new Error("denied")));
    const select = vi.spyOn(HTMLTextAreaElement.prototype, "select");
    const command = vi.fn();
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: command,
    });
    expect(await copyReviewMarkdown("x".repeat(20 * 1024 * 1024))).toBe(false);
    expect(select).not.toHaveBeenCalled();
    expect(command).not.toHaveBeenCalled();
    expect(document.querySelector("textarea")).toBeNull();
  });

  it("keeps a 20 MB denied snapshot out of the DOM and invalidates it on revision change", async () => {
    const large = { ...data, markdown: "é" + "x".repeat(20 * 1024 * 1024) };
    const { user, rerender } = render(
      <ReviewHandoff
        reviewId="r1"
        revision={3}
        load={() => Promise.resolve(large)}
      />,
    );
    clipboard(vi.fn().mockRejectedValue(new Error("denied")));
    const select = vi.spyOn(HTMLTextAreaElement.prototype, "select");
    const command = vi.fn();
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: command,
    });
    await user.click(
      screen.getByRole("button", { name: "Copy review as Markdown" }),
    );
    expect(await screen.findByRole("dialog")).toHaveTextContent(
      "Snapshot revision 3",
    );
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
    expect(select).not.toHaveBeenCalled();
    expect(command).not.toHaveBeenCalled();
    rerender(
      <ReviewHandoff
        reviewId="r1"
        revision={4}
        load={() => Promise.resolve(large)}
      />,
    );
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
  });

  it("reports download failures inside the dialog and cleans up the anchor and URL", async () => {
    const { user } = render(
      <ReviewHandoff
        reviewId="r1"
        revision={3}
        load={() => Promise.resolve(data)}
      />,
    );
    clipboard(vi.fn().mockRejectedValue(new Error("denied")));
    Object.defineProperty(URL, "createObjectURL", {
      configurable: true,
      value: vi.fn(() => "blob:local"),
    });
    const revoke = vi.fn();
    Object.defineProperty(URL, "revokeObjectURL", {
      configurable: true,
      value: revoke,
    });
    vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {
      throw new Error("blocked");
    });
    await user.click(
      screen.getByRole("button", { name: "Copy review as Markdown" }),
    );
    await screen.findByRole("dialog");
    await user.click(screen.getByRole("button", { name: "Download .md" }));
    expect(screen.getByRole("dialog")).toHaveTextContent(
      "Download unavailable",
    );
    expect(document.querySelector('a[href="blob:local"]')).toBeNull();
    await waitFor(() => expect(revoke).toHaveBeenCalledWith("blob:local"), {
      timeout: 2000,
    });
  });

  it("downloads exact Markdown bytes, not a transcript or rendered HTML", async () => {
    const create = vi.fn((_blob: Blob) => "blob:local");
    Object.defineProperty(URL, "createObjectURL", {
      configurable: true,
      value: create,
    });
    const click = vi
      .spyOn(HTMLAnchorElement.prototype, "click")
      .mockImplementation(() => undefined);
    downloadReviewMarkdown(data);
    expect(click).toHaveBeenCalledOnce();
    const blob = create.mock.calls[0][0];
    expect(blob).toBeInstanceOf(Blob);
    const bytes = await new Promise<string>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(reader.result as string);
      reader.onerror = reject;
      reader.readAsText(blob);
    });
    expect(bytes).toBe(data.markdown);
  });
});
