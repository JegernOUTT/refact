import { readFileSync } from "node:fs";
import { act } from "react-dom/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { fireEvent, render, screen } from "../../../utils/test-utils";
import {
  ImageFilePreview,
  MAX_ZOOM,
  MIN_PREVIEW_HEIGHT,
} from "./ImageFilePreview";
import { openTab, splitTab } from "../workspaceSlice";
import { imageDataUrl } from "../../../services/refact/files";

const PNG_DATA = "iVBORw0KGgo=";
const pngSrc = `data:image/png;base64,${PNG_DATA}`;

const loadImage = (width: number, height: number) => {
  const image = screen.getByRole("img");
  Object.defineProperty(image, "naturalWidth", {
    configurable: true,
    value: width,
  });
  Object.defineProperty(image, "naturalHeight", {
    configurable: true,
    value: height,
  });
  fireEvent.load(image);
  return image;
};

const zoomText = () => screen.getByLabelText("Zoom level").textContent;

describe("imageDataUrl", () => {
  it("builds data URLs only for allowlisted raster MIME types", () => {
    expect(imageDataUrl("image/png", PNG_DATA)).toBe(pngSrc);
    expect(imageDataUrl("IMAGE/JPEG", PNG_DATA)).toBe(
      `data:image/jpeg;base64,${PNG_DATA}`,
    );
    expect(imageDataUrl("image/svg+xml", PNG_DATA)).toBeNull();
    expect(imageDataUrl("text/html", PNG_DATA)).toBeNull();
    expect(imageDataUrl("image/png;charset=x", PNG_DATA)).toBeNull();
    expect(imageDataUrl("image/png", "not base64!")).toBeNull();
    expect(imageDataUrl("image/png", "abc")).toBeNull();
    expect(imageDataUrl(null, PNG_DATA)).toBeNull();
    expect(imageDataUrl("image/png", "")).toBeNull();
  });
});

describe("ImageFilePreview", () => {
  let paneHeight = 600;
  let viewportHeight = 100;
  let resize: () => void = () => undefined;
  beforeEach(() => {
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(200);
    paneHeight = 600;
    viewportHeight = 100;
    vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockImplementation(
      function (this: HTMLElement) {
        return this.dataset.testid === "image-preview"
          ? paneHeight
          : viewportHeight;
      },
    );
    vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockReturnValue(0);
    vi.stubGlobal(
      "ResizeObserver",
      class {
        constructor(callback: () => void) {
          resize = callback;
        }
        observe() {
          return;
        }
        disconnect() {
          return;
        }
      },
    );
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("retains image transforms and manual height across structural remounts", async () => {
    const view = render(
      <ImageFilePreview key="before" alt="photo.png" src={pngSrc} />,
    );
    view.store.dispatch(openTab("file:photo.png"));
    loadImage(400, 200);
    await view.user.click(
      screen.getByRole("button", { name: "Rotate clockwise" }),
    );
    await view.user.click(screen.getByRole("button", { name: "Actual size" }));
    fireEvent.keyDown(screen.getByRole("separator"), { key: "Home" });
    view.store.dispatch(splitTab({ tabId: "file:photo.png", dir: "row" }));
    view.rerender(
      <ImageFilePreview key="after" alt="photo.png" src={pngSrc} />,
    );
    const image = loadImage(400, 200);
    expect(image).toHaveAttribute("data-rotation", "90");
    expect(zoomText()).toBe("100%");
    expect(screen.getByTestId("image-preview-viewport")).toHaveAttribute(
      "data-resized",
      "true",
    );
    view.rerender(<ImageFilePreview alt="photo.png" src={`${pngSrc}AA==`} />);
    expect(screen.getByRole("img")).toHaveAttribute("data-rotation", "0");
  });

  it("floats discoverable controls without reserving a metadata row", async () => {
    const { user } = render(<ImageFilePreview alt="photo.png" src={pngSrc} />);
    const toolbar = screen.getByRole("toolbar", { name: "Image controls" });
    expect(toolbar.parentElement).toBe(
      screen.getByTestId("image-preview-viewport").parentElement,
    );
    await user.tab();
    expect(screen.getByRole("button", { name: "Zoom out" })).toHaveFocus();
    const css = readFileSync(
      "src/features/Workspace/FilesPanel/ImageFilePreview.module.css",
      "utf8",
    );
    expect(css).toMatch(/position: absolute/u);
    expect(css).toMatch(/\.preview:focus-within \.toolbar/u);
    expect(css).toMatch(/\(pointer: coarse\)/u);
    expect(
      screen.getByRole("separator", { name: "Resize image preview" }),
    ).toBeVisible();
  });

  it("fits large images below the manual zoom minimum", () => {
    render(<ImageFilePreview alt="huge.png" src={pngSrc} />);
    const image = loadImage(20000, 100);

    expect(zoomText()).toBe("1%");
    expect(image.style.width).toBe("200px");
    expect(screen.getByRole("button", { name: "Fit to view" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });

  it("zooms within bounds and switches between actual size and fit", async () => {
    const { user } = render(<ImageFilePreview alt="photo.png" src={pngSrc} />);
    loadImage(400, 100);
    expect(zoomText()).toBe("50%");

    await user.click(screen.getByRole("button", { name: "Actual size" }));
    expect(zoomText()).toBe("100%");

    await user.click(screen.getByRole("button", { name: "Zoom in" }));
    expect(zoomText()).toBe("125%");

    await user.click(screen.getByRole("button", { name: "Zoom out" }));
    expect(zoomText()).toBe("100%");

    for (let index = 0; index < 20; index += 1) {
      const zoomIn = screen.getByRole("button", { name: "Zoom in" });
      if ((zoomIn as HTMLButtonElement).disabled) break;
      await user.click(zoomIn);
    }
    expect(zoomText()).toBe(`${String(MAX_ZOOM * 100)}%`);
    expect(screen.getByRole("button", { name: "Zoom in" })).toBeDisabled();

    await user.click(screen.getByRole("button", { name: "Fit to view" }));
    expect(zoomText()).toBe("50%");
  });

  it("rotates and refits with swapped dimensions", async () => {
    const { user } = render(<ImageFilePreview alt="tall.png" src={pngSrc} />);
    const image = loadImage(100, 400);
    expect(zoomText()).toBe("25%");

    await user.click(screen.getByRole("button", { name: "Rotate clockwise" }));
    expect(image).toHaveAttribute("data-rotation", "90");
    expect(zoomText()).toBe("50%");
  });

  it("resizes the preview area with the keyboard and only primary pointer", () => {
    render(<ImageFilePreview alt="photo.png" src={pngSrc} />);
    const handle = screen.getByRole("separator", {
      name: "Resize image preview",
    });
    const viewport = screen.getByTestId("image-preview-viewport");
    expect(viewport.style.height).toBe("");

    fireEvent.keyDown(handle, { key: "ArrowUp" });
    expect(handle).toHaveAttribute("aria-valuenow", "560");
    fireEvent.keyDown(handle, { key: "Home" });
    expect(viewport.style.height).toBe(`${String(MIN_PREVIEW_HEIGHT)}px`);
    fireEvent.keyDown(handle, { key: "ArrowUp" });
    expect(handle).toHaveAttribute("aria-valuenow", String(MIN_PREVIEW_HEIGHT));

    fireEvent.keyDown(handle, { key: "End" });
    expect(viewport.style.height).toBe("");
    fireEvent.keyDown(handle, { key: "Home" });
    fireEvent.pointerDown(handle, { button: 2, clientY: 0, pointerId: 1 });
    fireEvent.pointerMove(handle, { clientY: 300, pointerId: 1 });
    expect(handle).toHaveAttribute("aria-valuenow", String(MIN_PREVIEW_HEIGHT));
    const capture = vi.fn();
    const release = vi.fn();
    handle.setPointerCapture = capture;
    handle.hasPointerCapture = () => true;
    handle.releasePointerCapture = release;
    fireEvent.pointerDown(handle, { button: 0, clientY: 100, pointerId: 2 });
    fireEvent.pointerMove(handle, { clientY: 180, pointerId: 2 });
    expect(handle).toHaveAttribute("aria-valuenow", "240");
    fireEvent.pointerUp(handle, { pointerId: 2 });
    expect(capture).toHaveBeenCalledWith(2);
    expect(release).toHaveBeenCalledWith(2);
  });

  it("tracks pane resizing in fit mode and clamps manually resized height", () => {
    render(<ImageFilePreview alt="photo.png" src={pngSrc} />);
    loadImage(100, 400);
    const handle = screen.getByRole("separator");
    const viewport = screen.getByTestId("image-preview-viewport");
    expect(viewport.style.height).toBe("");
    expect(zoomText()).toBe("25%");
    act(() => {
      paneHeight = 900;
      viewportHeight = 300;
      resize();
    });
    expect(zoomText()).toBe("75%");
    expect(handle).toHaveAttribute("aria-valuemax", "900");
    fireEvent.keyDown(handle, { key: "ArrowUp" });
    expect(viewport.style.height).toBe("860px");
    act(() => {
      paneHeight = 200;
      viewportHeight = 200;
      resize();
    });
    expect(viewport.style.height).toBe("200px");
    fireEvent.keyDown(handle, { key: "End" });
    expect(viewport.style.height).toBe("");
  });

  it("cancels wheel scrolling and zooms both directions below the initial fit", () => {
    const listener = vi.spyOn(HTMLElement.prototype, "addEventListener");
    render(<ImageFilePreview alt="huge.png" src={pngSrc} />);
    const image = loadImage(20000, 100);
    const viewport = screen.getByTestId("image-preview-viewport");
    expect(listener).toHaveBeenCalledWith("wheel", expect.any(Function), {
      passive: false,
    });
    expect(fireEvent.wheel(viewport, { deltaY: 100, cancelable: true })).toBe(
      false,
    );
    expect(image.style.width).toBe("160px");
    fireEvent.wheel(viewport, { deltaY: -100, cancelable: true });
    expect(image.style.width).toBe("200px");
    fireEvent.wheel(viewport, { deltaY: 100, cancelable: true });
    expect(image.style.width).toBe("160px");
    for (let index = 0; index < 40; index += 1)
      fireEvent.wheel(viewport, { deltaY: -100 });
    expect(zoomText()).toBe("800%");
  });

  it("compensates existing scroll for the off-center wheel point and removes its listener", async () => {
    const { user, unmount } = render(
      <ImageFilePreview alt="photo.png" src={pngSrc} />,
    );
    const image = loadImage(400, 200);
    await user.click(screen.getByRole("button", { name: "Actual size" }));
    const viewport = screen.getByTestId("image-preview-viewport");
    const stage = image.parentElement;
    if (!stage) throw new Error("Missing image stage");
    viewport.scrollLeft = 90;
    viewport.scrollTop = 30;
    vi.spyOn(stage, "getBoundingClientRect").mockImplementation(() =>
      DOMRect.fromRect({
        x: 40 - viewport.scrollLeft,
        y: 20 - viewport.scrollTop,
        width: Number.parseFloat(stage.style.width),
        height: Number.parseFloat(stage.style.height),
      }),
    );
    const wheel = new MouseEvent("wheel", {
      clientX: 170,
      clientY: 80,
      cancelable: true,
    });
    Object.defineProperty(wheel, "deltaY", { value: -100 });
    fireEvent(viewport, wheel);
    expect(viewport.scrollLeft).toBe(145);
    expect(viewport.scrollTop).toBe(52.5);
    const removeListener = vi.spyOn(viewport, "removeEventListener");
    unmount();
    expect(removeListener).toHaveBeenCalledWith("wheel", expect.any(Function));
  });

  it("shows a decode failure and resets state when the source changes", async () => {
    const { rerender, user } = render(
      <ImageFilePreview alt="broken.png" src={pngSrc} />,
    );
    fireEvent.error(screen.getByRole("img"));
    expect(screen.getByText("Image unavailable")).toBeVisible();
    expect(
      screen.getByText("broken.png could not be decoded as an image."),
    ).toBeVisible();

    rerender(
      <ImageFilePreview
        alt="next.png"
        src={`data:image/gif;base64,${PNG_DATA}`}
      />,
    );
    expect(screen.queryByText("Image unavailable")).toBeNull();
    loadImage(100, 50);
    await user.click(screen.getByRole("button", { name: "Rotate clockwise" }));
    expect(screen.getByRole("img")).toHaveAttribute("data-rotation", "90");

    rerender(<ImageFilePreview alt="other.png" src={pngSrc} />);
    expect(screen.getByRole("img")).toHaveAttribute("data-rotation", "0");
    expect(zoomText()).toBe("100%");
  });
});
