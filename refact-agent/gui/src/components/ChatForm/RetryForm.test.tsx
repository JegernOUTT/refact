import { beforeEach, describe, expect, test, vi } from "vitest";
import { fireEvent } from "@testing-library/react";
import { render, screen } from "../../utils/test-utils";
import { RetryForm } from "./RetryForm";

vi.mock("../../hooks", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../hooks")>();

  return {
    ...actual,
    useCapsForToolUse: () => ({
      currentModel: "",
      data: undefined,
      isMultimodalitySupportedForCurrentModel: false,
      loading: false,
      setCapModel: vi.fn(),
      usableModelsForPlan: [],
    }),
  };
});

describe("RetryForm", () => {
  const scrollIntoViewSpy = vi.spyOn(
    window.HTMLElement.prototype,
    "scrollIntoView",
  );

  beforeEach(() => {
    scrollIntoViewSpy.mockClear();
  });

  test("mixed image paste replaces selected text once and keeps it when editing is cancelled", async () => {
    const onSubmit = vi.fn();
    const { user } = render(
      <RetryForm
        value="before OLD after"
        onSubmit={onSubmit}
        onClose={vi.fn()}
      />,
    );
    const textarea = screen.getByRole<HTMLTextAreaElement>("textbox");
    textarea.setSelectionRange(7, 10);
    const file = new File(["png"], "paste.png", { type: "image/png" });
    const getData = vi.fn(() => "new\ntext");
    const nativeAllowed = fireEvent.paste(textarea, {
      clipboardData: {
        items: [{ type: "image/png", getAsFile: () => file }],
        getData,
      },
    });
    expect(nativeAllowed).toBe(false);
    expect(getData).toHaveBeenCalledWith("text/plain");
    expect(textarea).toHaveValue("before new\ntext after");
    expect(textarea.selectionStart).toBe(15);
    expect(textarea.selectionEnd).toBe(15);
    // The queue never calls its recipient on Cancel; text is independent of it.
    await user.click(screen.getByRole("button", { name: "Submit" }));
    expect(onSubmit).toHaveBeenCalledWith("before new\ntext after");
  });

  test("scrolls into view on mount", () => {
    render(<RetryForm value="Message" onSubmit={vi.fn()} onClose={vi.fn()} />);

    expect(scrollIntoViewSpy).toHaveBeenCalledOnce();
    expect(scrollIntoViewSpy).toHaveBeenCalledWith({
      block: "nearest",
      behavior: "smooth",
    });
  });

  test("calls onClose when Cancel is clicked", async () => {
    const onClose = vi.fn();
    const { user } = render(
      <RetryForm value="Message" onSubmit={vi.fn()} onClose={onClose} />,
    );

    await user.click(screen.getByRole("button", { name: "Cancel" }));

    expect(onClose).toHaveBeenCalledOnce();
  });
});
