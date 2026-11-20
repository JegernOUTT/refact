import type { ProjectBrowserFrame } from "../../services/refact/browser";

export function framePosition(
  frame: Pick<ProjectBrowserFrame, "width" | "height" | "viewport">,
  bounds: Pick<DOMRect, "left" | "top" | "width" | "height">,
  clientX: number,
  clientY: number,
) {
  const scale = Math.min(
    bounds.width / frame.width,
    bounds.height / frame.height,
  );
  const width = frame.width * scale;
  const height = frame.height * scale;
  const x = clientX - bounds.left - (bounds.width - width) / 2;
  const y = clientY - bounds.top - (bounds.height - height) / 2;
  if (width <= 0 || height <= 0 || x < 0 || x >= width || y < 0 || y >= height)
    return null;
  return {
    x: (x / width) * frame.viewport.width,
    y: (y / height) * frame.viewport.height,
  };
}
