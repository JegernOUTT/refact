import {
  createEvent,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { act } from "react-dom/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ImageEditor } from "./ImageEditor";

let encodes: BlobCallback[];
const pixels = new Blob(["edited pixels"], { type: "image/png" });

function sourceFile() {
  const bytes = new Uint8Array(24);
  bytes.set([137, 80, 78, 71, 13, 10, 26, 10]);
  const view = new DataView(bytes.buffer);
  view.setUint32(16, 600);
  view.setUint32(20, 400);
  return new File([bytes], "fixture.png", {
    type: "image/png",
    lastModified: 123,
  });
}

function setup() {
  const file = sourceFile();
  const onApply = vi.fn<(file: File) => void>();
  const onCancel = vi.fn();
  render(<ImageEditor file={file} onApply={onApply} onCancel={onCancel} />);
  return { file, onApply, onCancel };
}

const preview = () => screen.getByLabelText("Image annotation preview");
const apply = () => screen.getByRole("button", { name: "Apply" });
const ready = () => waitFor(() => expect(apply()).not.toBeDisabled());
const enter = (target = preview(), options: KeyboardEventInit = {}) =>
  fireEvent.keyDown(target, { key: "Enter", ...options });

function startGesture(tool: string) {
  fireEvent.click(screen.getByRole("button", { name: tool }));
  const viewport = screen.getByLabelText("Image pan and zoom");
  viewport.setPointerCapture = vi.fn();
  viewport.hasPointerCapture = vi.fn(() => false);
  fireEvent.pointerDown(viewport, {
    button: 0,
    isPrimary: true,
    pointerId: 1,
    clientX: 0,
    clientY: 0,
  });
  return viewport;
}

async function finishExport(index = 0) {
  await waitFor(() => expect(encodes.length).toBeGreaterThan(index));
  act(() => encodes[index](pixels));
}

beforeEach(() => {
  encodes = [];
  vi.stubGlobal(
    "createImageBitmap",
    vi.fn().mockResolvedValue({
      width: 600,
      height: 400,
      close: vi.fn(),
    }),
  );
  const context = {
    measureText: () => ({ width: 80 }),
    setTransform: vi.fn(),
    clearRect: vi.fn(),
    scale: vi.fn(),
    drawImage: vi.fn(),
    save: vi.fn(),
    restore: vi.fn(),
    beginPath: vi.fn(),
    moveTo: vi.fn(),
    lineTo: vi.fn(),
    stroke: vi.fn(),
    fillText: vi.fn(),
  } as unknown as CanvasRenderingContext2D;
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(context);
  vi.spyOn(HTMLCanvasElement.prototype, "toBlob").mockImplementation(
    (callback) => {
      encodes.push(callback);
    },
  );
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("ImageEditor Enter attachment shortcut", () => {
  it("initially focuses the dialog so Enter attaches without closing", async () => {
    const { file, onApply, onCancel } = setup();
    const user = userEvent.setup();
    await ready();
    expect(screen.getByRole("dialog")).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(onApply).toHaveBeenCalledExactlyOnceWith(file);
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("attaches the exact ready original once per press and consumes the shortcut", async () => {
    const { file, onApply, onCancel } = setup();
    await ready();
    expect(enter()).toBe(false);
    expect(onApply).toHaveBeenCalledExactlyOnceWith(file);
    enter(preview(), { repeat: true });
    expect(onApply).toHaveBeenCalledTimes(1);
    fireEvent.keyUp(preview(), { key: "Enter" });
    enter();
    expect(onApply).toHaveBeenCalledTimes(2);
    expect(onCancel).not.toHaveBeenCalled();
    expect(
      screen.getByText(
        /Enter attaches the edited image without sending a message/,
      ),
    ).toBeInTheDocument();
  });

  it("attaches the same exported File as Apply, never the unedited source", async () => {
    const { file, onApply } = setup();
    await ready();
    const viewport = startGesture("Brush");
    fireEvent.pointerUp(viewport, { pointerId: 1 });
    await finishExport();
    await ready();
    fireEvent.click(apply());
    const exported = onApply.mock.calls[0][0];
    expect(exported).not.toBe(file);
    expect(exported.name).toBe("fixture.png");
    expect(exported.type).toBe("image/png");
    expect(exported.lastModified).toBe(123);
    expect(await exported.text()).toBe(await pixels.text());
    onApply.mockClear();
    enter();
    expect(onApply).toHaveBeenCalledExactlyOnceWith(exported);
  });

  it("blocks initial, pending and stale exports until the current export completes", async () => {
    const { onApply } = setup();
    enter(screen.getByRole("dialog"));
    expect(onApply).not.toHaveBeenCalled();
    await ready();
    fireEvent.click(screen.getByRole("button", { name: "Export settings" }));
    fireEvent.change(screen.getByLabelText("Resolution scale"), {
      target: { value: "90" },
    });
    enter();
    await waitFor(() => expect(encodes).toHaveLength(1));
    enter();
    fireEvent.change(screen.getByLabelText("Resolution scale"), {
      target: { value: "80" },
    });
    await finishExport(0);
    enter();
    expect(apply()).toBeDisabled();
    expect(onApply).not.toHaveBeenCalled();
    await finishExport(1);
    await ready();
    enter();
    expect(onApply).toHaveBeenCalledTimes(1);
  });

  it("does not attach after an export failure", async () => {
    const { onApply } = setup();
    await ready();
    const viewport = startGesture("Brush");
    fireEvent.pointerUp(viewport, { pointerId: 1 });
    await waitFor(() => expect(encodes).toHaveLength(1));
    act(() => encodes[0](null));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Image encoding failed",
    );
    enter();
    expect(apply()).toBeDisabled();
    expect(onApply).not.toHaveBeenCalled();
  });

  it.each([
    { shiftKey: true },
    { ctrlKey: true },
    { metaKey: true },
    { altKey: true },
    { repeat: true },
    { isComposing: true },
  ])("ignores protected keyboard event %j", async (options) => {
    const { onApply } = setup();
    await ready();
    expect(enter(preview(), options)).toBe(true);
    expect(onApply).not.toHaveBeenCalled();
  });

  it("respects already prevented events and editable controls", async () => {
    const { onApply } = setup();
    await ready();
    const prevented = createEvent.keyDown(preview(), { key: "Enter" });
    prevented.preventDefault();
    fireEvent(preview(), prevented);
    const dialog = screen.getByRole("dialog");
    for (const tag of ["input", "textarea", "select", "div"]) {
      const editable = document.createElement(tag);
      if (tag === "div") {
        editable.contentEditable = "true";
        editable.append(document.createElement("span"));
      }
      dialog.append(editable);
      expect(
        enter((editable.firstElementChild as HTMLElement | null) ?? editable),
      ).toBe(true);
      editable.remove();
    }
    expect(onApply).not.toHaveBeenCalled();
  });

  it("preserves native Enter activation of Cancel, tool buttons and Apply", async () => {
    const { file, onApply, onCancel } = setup();
    const user = userEvent.setup();
    await ready();
    screen.getByRole("button", { name: "Cancel / Discard" }).focus();
    await user.keyboard("{Enter}");
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onApply).not.toHaveBeenCalled();
    const brush = screen.getByRole("button", { name: "Brush" });
    brush.focus();
    await user.keyboard("{Enter}");
    expect(brush).toHaveAttribute("aria-pressed", "true");
    expect(onApply).not.toHaveBeenCalled();
    apply().focus();
    await user.keyboard("{Enter}");
    expect(onApply).toHaveBeenCalledExactlyOnceWith(file);
  });

  it("blocks unfinished strokes and labels, including when focus leaves inline text", async () => {
    const { onApply } = setup();
    await ready();
    const viewport = startGesture("Brush + Text");
    enter();
    expect(apply()).toBeDisabled();
    fireEvent.pointerUp(viewport, { pointerId: 1 });
    enter();
    expect(apply()).toBeDisabled();
    startGesture("Text");
    expect(
      screen.getByRole("textbox", { name: "Annotation text" }),
    ).toBeInTheDocument();
    enter();
    expect(onApply).not.toHaveBeenCalled();
  });

  it.each(["Control", "Meta"])(
    "keeps Enter newlines and %s+Enter annotation commit separate from attachment",
    async (modifier) => {
      const { onApply } = setup();
      const user = userEvent.setup();
      await ready();
      const viewport = startGesture("Text");
      fireEvent.pointerUp(viewport, { pointerId: 1 });
      const input = screen.getByRole("textbox", { name: "Annotation text" });
      input.focus();
      await user.type(input, "first{Enter}second");
      expect(input).toHaveValue("first\nsecond");
      expect(onApply).not.toHaveBeenCalled();
      await user.keyboard(`{${modifier}>}{Enter}{/${modifier}}`);
      expect(
        screen.queryByRole("textbox", { name: "Annotation text" }),
      ).not.toBeInTheDocument();
      expect(onApply).not.toHaveBeenCalled();
      enter();
      expect(onApply).not.toHaveBeenCalled();
      await finishExport();
      await ready();
      enter();
      expect(onApply).toHaveBeenCalledTimes(1);
    },
  );
});
