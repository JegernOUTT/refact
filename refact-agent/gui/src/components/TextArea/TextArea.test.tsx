import React from "react";
import { act } from "react-dom/test-utils";
import { afterEach, beforeEach, describe, test, expect, vi } from "vitest";
import { render } from "../../utils/test-utils";
import { TextArea, TextAreaProps } from ".";

const App: React.FC<Partial<TextAreaProps>> = (props) => {
  const [value, setValue] = React.useState(props.value ?? "");
  const defaultProps: TextAreaProps = {
    onChange: (e) => setValue(e.target.value),
    value,
    ...props,
  };
  return <TextArea {...defaultProps} />;
};

describe("TextArea", () => {
  beforeEach(() => {
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) =>
      window.setTimeout(() => callback(performance.now()), 0),
    );
    vi.stubGlobal("cancelAnimationFrame", (id: number) =>
      window.clearTimeout(id),
    );
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  test("inserting text before previous text", async () => {
    const testId = "textarea";
    const { user, ...app } = render(<App data-testid={testId} />);
    const textarea = app.getByTestId(testId) as HTMLTextAreaElement;
    await user.type(textarea, "\nworld");
    await user.type(textarea, "hello", {
      initialSelectionStart: 0,
      initialSelectionEnd: 0,
    });
    expect(textarea.textContent).toEqual("hello\nworld");
  });

  test("undo / redo", async () => {
    const testId = "textarea";
    const { user, ...app } = render(<App data-testid={testId} />);
    const textarea = app.getByTestId(testId) as HTMLTextAreaElement;
    await user.type(textarea, "hello world");
    expect(textarea.textContent).toEqual("hello world");
    await user.keyboard("{Control>}{z}{/Control}");
    await user.keyboard("{Control>}{z}{/Control}");
    await user.keyboard("{Control>}{z}{/Control}");
    await user.keyboard("{Control>}{z}{/Control}");
    await user.keyboard("{Control>}{z}{/Control}");
    await user.keyboard("{Control>}{z}{/Control}");
    expect(textarea.textContent).toEqual("hello");
    await user.keyboard("{Shift>}{Control>}{z}{/Control}{/Shift}");
    await user.keyboard("{Shift>}{Control>}{z}{/Control}{/Shift}");
    await user.keyboard("{Shift>}{Control>}{z}{/Control}{/Shift}");
    await user.keyboard("{Shift>}{Control>}{z}{/Control}{/Shift}");
    await user.keyboard("{Shift>}{Control>}{z}{/Control}{/Shift}");
    await user.keyboard("{Shift>}{Control>}{z}{/Control}{/Shift}");
    expect(textarea.textContent).toEqual("hello world");
  });

  test("coalesces large incremental values into one undo step", async () => {
    vi.useFakeTimers();
    const onChange = vi.fn();
    const { rerender, getByTestId } = render(
      <TextArea
        data-testid="textarea"
        value={"x".repeat(200_000)}
        onChange={onChange}
      />,
    );

    for (let index = 1; index <= 1_000; index++) {
      rerender(
        <TextArea
          data-testid="textarea"
          value={`${"x".repeat(200_000)}${"y".repeat(index)}`}
          onChange={onChange}
        />,
      );
      vi.advanceTimersByTime(1);
    }

    const textarea = getByTestId("textarea") as HTMLTextAreaElement;
    await act(async () => {
      textarea.dispatchEvent(
        new KeyboardEvent("keydown", {
          bubbles: true,
          ctrlKey: true,
          key: "z",
        }),
      );
      await Promise.resolve();
    });

    const undoEvent = onChange.mock.lastCall?.[0] as
      | React.ChangeEvent<HTMLTextAreaElement>
      | undefined;
    expect(undoEvent?.target.value).toBe("x".repeat(200_000));
  });

  test("coalesces resize frames and skips height writes when unchanged", async () => {
    vi.useFakeTimers();
    const { rerender, getByTestId } = render(
      <App data-testid="textarea" value="first" />,
    );
    const textarea = getByTestId("textarea") as HTMLTextAreaElement;
    Object.defineProperty(textarea, "scrollHeight", {
      configurable: true,
      value: 40,
    });

    await act(() => vi.runOnlyPendingTimers());
    expect(textarea.style.height).toBe("42px");

    const setProperty = vi.spyOn(textarea.style, "height", "set");
    rerender(<App data-testid="textarea" value="first value" />);
    rerender(<App data-testid="textarea" value="first value again" />);
    await act(() => vi.runOnlyPendingTimers());

    expect(setProperty).not.toHaveBeenCalled();
  });
});
