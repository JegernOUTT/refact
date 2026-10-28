import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { fireEvent, render, screen } from "../../../utils/test-utils";
import {
  DEFAULT_PREVIEW_HEIGHT,
  ImageFilePreview,
  MAX_ZOOM,
  MIN_PREVIEW_HEIGHT,
} from "./ImageFilePreview";
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
  beforeEach(() => {
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(200);
    vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(100);
  });

  afterEach(() => {
    vi.restoreAllMocks();
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
    expect(viewport.style.height).toBe(`${String(DEFAULT_PREVIEW_HEIGHT)}px`);

    fireEvent.keyDown(handle, { key: "ArrowDown" });
    expect(handle).toHaveAttribute(
      "aria-valuenow",
      String(DEFAULT_PREVIEW_HEIGHT + 40),
    );
    fireEvent.keyDown(handle, { key: "Home" });
    expect(viewport.style.height).toBe(`${String(MIN_PREVIEW_HEIGHT)}px`);
    fireEvent.keyDown(handle, { key: "ArrowUp" });
    expect(handle).toHaveAttribute("aria-valuenow", String(MIN_PREVIEW_HEIGHT));

    fireEvent.pointerDown(handle, { button: 2, clientY: 0, pointerId: 1 });
    fireEvent.pointerMove(handle, { clientY: 300, pointerId: 1 });
    expect(handle).toHaveAttribute("aria-valuenow", String(MIN_PREVIEW_HEIGHT));
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
