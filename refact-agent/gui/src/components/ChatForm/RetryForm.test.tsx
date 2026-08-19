import { beforeEach, describe, expect, test, vi } from "vitest";
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

  test("renders the initial value in the textarea", () => {
    render(
      <RetryForm
        value="Initial message"
        onSubmit={vi.fn()}
        onClose={vi.fn()}
      />,
    );

    expect(screen.getByRole("textbox")).toHaveValue("Initial message");
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
