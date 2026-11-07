export type Point = { x: number; y: number };
export type Tool =
  | "pan"
  | "brush-text"
  | "brush"
  | "text"
  | "arrow"
  | "line"
  | "ellipse"
  | "rectangle";
export type Annotation = {
  tool: Exclude<Tool, "pan">;
  points: Point[];
  color: string;
  size: number;
  text?: string;
  label?: { point: Point; text: string; size: number; color: string };
};
export type ExportSettings = {
  format: "image/png" | "image/webp" | "image/jpeg";
  quality: number;
  scale: number;
  target: number | null;
};
export const DEFAULT_SETTINGS: ExportSettings = {
  format: "image/png",
  quality: 90,
  scale: 100,
  target: null,
};
export const MAX_PIXELS = 16_000_000;
export const MAX_SIDE = 8192;
export const MAX_ANNOTATIONS = 100;
export const MAX_POINTS = 2048;
export function safeDimensions(width: number, height: number) {
  const ratio = Math.min(
    1,
    MAX_SIDE / width,
    MAX_SIDE / height,
    Math.sqrt(MAX_PIXELS / (width * height)),
  );
  return {
    width: Math.max(1, Math.floor(width * ratio)),
    height: Math.max(1, Math.floor(height * ratio)),
  };
}
export function unchanged(annotations: Annotation[], settings: ExportSettings) {
  return (
    annotations.length === 0 &&
    settings.format === DEFAULT_SETTINGS.format &&
    settings.quality === 90 &&
    settings.scale === 100 &&
    settings.target === null
  );
}
export function drawAnnotations(
  ctx: CanvasRenderingContext2D,
  annotations: Annotation[],
) {
  for (const annotation of annotations) {
    if (annotation.tool === "brush-text") {
      drawAnnotations(ctx, [
        { ...annotation, tool: "brush" },
        ...(annotation.label
          ? [
              {
                tool: "text" as const,
                points: [annotation.label.point],
                ...annotation.label,
              },
            ]
          : []),
      ]);
      continue;
    }
    const a = annotation.points[0];
    const b = annotation.points[annotation.points.length - 1];
    ctx.save();
    ctx.strokeStyle = annotation.color;
    ctx.fillStyle = annotation.color;
    ctx.lineWidth = annotation.size;
    ctx.lineCap = "round";
    ctx.lineJoin = "round";
    ctx.beginPath();
    if (annotation.tool === "text") {
      ctx.font = `${annotation.size}px sans-serif`;
      ctx.textBaseline = "top";
      (annotation.text ?? "").split("\n").forEach((line, index) => {
        ctx.fillText(line, a.x, a.y + index * annotation.size * 1.2);
      });
    } else if (annotation.tool === "ellipse") {
      ctx.ellipse(
        (a.x + b.x) / 2,
        (a.y + b.y) / 2,
        Math.abs(b.x - a.x) / 2,
        Math.abs(b.y - a.y) / 2,
        0,
        0,
        2 * Math.PI,
      );
      ctx.stroke();
    } else if (annotation.tool === "rectangle") {
      ctx.strokeRect(a.x, a.y, b.x - a.x, b.y - a.y);
    } else {
      ctx.moveTo(a.x, a.y);
      for (const p of annotation.points.slice(1)) ctx.lineTo(p.x, p.y);
      if (annotation.points.length === 1) {
        ctx.lineTo(a.x + 0.01, a.y);
      }
      ctx.stroke();
      if (annotation.tool === "arrow") {
        const angle = Math.atan2(b.y - a.y, b.x - a.x);
        const length = Math.max(12, annotation.size * 4);
        ctx.beginPath();
        ctx.moveTo(
          b.x - length * Math.cos(angle - Math.PI / 6),
          b.y - length * Math.sin(angle - Math.PI / 6),
        );
        ctx.lineTo(b.x, b.y);
        ctx.lineTo(
          b.x - length * Math.cos(angle + Math.PI / 6),
          b.y - length * Math.sin(angle + Math.PI / 6),
        );
        ctx.stroke();
      }
    }
    ctx.restore();
  }
}
export function renderImage(
  canvas: HTMLCanvasElement,
  image: CanvasImageSource,
  width: number,
  height: number,
  annotations: Annotation[],
  settings: ExportSettings,
) {
  const outputWidth = Math.max(1, Math.round((width * settings.scale) / 100));
  const outputHeight = Math.max(1, Math.round((height * settings.scale) / 100));
  if (canvas.width !== outputWidth) canvas.width = outputWidth;
  if (canvas.height !== outputHeight) canvas.height = outputHeight;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("Canvas rendering is unavailable.");
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  if (settings.format === "image/jpeg") {
    ctx.fillStyle = "#ffffff";
    ctx.fillRect(0, 0, canvas.width, canvas.height);
  }
  ctx.scale(canvas.width / width, canvas.height / height);
  ctx.drawImage(image, 0, 0, width, height);
  drawAnnotations(ctx, annotations);
}
const encode = (canvas: HTMLCanvasElement, format: string, quality: number) =>
  new Promise<Blob>((resolve, reject) =>
    canvas.toBlob(
      (blob) =>
        blob ? resolve(blob) : reject(new Error("Image encoding failed.")),
      format,
      quality / 100,
    ),
  );
export async function exportImage(
  canvas: HTMLCanvasElement,
  settings: ExportSettings,
  stale: () => boolean = () => false,
) {
  let blob = await encode(canvas, settings.format, settings.quality);
  if (blob.type !== settings.format)
    throw new Error("This browser cannot export the selected format.");
  const target = settings.target;
  if (target && blob.size > target && settings.format !== "image/png") {
    let low = 1;
    let high = settings.quality;
    let best: Blob | undefined;
    const smallest = await encode(canvas, settings.format, low);
    if (smallest.size <= target) best = smallest;
    for (
      let attempt = 0;
      attempt < 6 && high - low > 1 && !stale();
      attempt++
    ) {
      const quality = Math.floor((low + high) / 2);
      const candidate = await encode(canvas, settings.format, quality);
      if (candidate.size <= target) {
        best = candidate;
        low = quality;
      } else high = quality;
    }
    blob = best ?? smallest;
  }
  return {
    blob,
    warning:
      target && blob.size > target
        ? "Target size cannot be met at this resolution. Lower the resolution explicitly or increase the target."
        : null,
  };
}
