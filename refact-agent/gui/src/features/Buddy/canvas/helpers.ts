let pixelFontReady = false;
// eslint-disable-next-line @typescript-eslint/no-unnecessary-condition, @typescript-eslint/prefer-optional-chain
if (typeof document !== "undefined" && document.fonts) {
  void document.fonts.load('8px "Press Start 2P"').then(() => {
    pixelFontReady = true;
  });
}

function softRoundedFill(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  color: string,
): void {
  const width = Math.max(0.5, w);
  const height = Math.max(0.5, h);
  if (Math.min(width, height) > 12) {
    ctx.fillStyle = color;
    ctx.fillRect(x, y, width, height);
    return;
  }
  ctx.fillStyle = color;
  ctx.beginPath();
  if (width <= 1.2 && height <= 1.2) {
    ctx.ellipse(
      x + width / 2,
      y + height / 2,
      width * 0.62,
      height * 0.62,
      0,
      0,
      Math.PI * 2,
    );
    ctx.fill();
    return;
  }
  const radius = Math.min(width, height) * 0.36;
  const right = x + width;
  const bottom = y + height;
  ctx.moveTo(x + radius, y);
  ctx.lineTo(right - radius, y);
  ctx.arc(right - radius, y + radius, radius, -Math.PI / 2, 0);
  ctx.lineTo(right, bottom - radius);
  ctx.arc(right - radius, bottom - radius, radius, 0, Math.PI / 2);
  ctx.lineTo(x + radius, bottom);
  ctx.arc(x + radius, bottom - radius, radius, Math.PI / 2, Math.PI);
  ctx.lineTo(x, y + radius);
  ctx.arc(x + radius, y + radius, radius, Math.PI, Math.PI * 1.5);
  ctx.fill();
}

export function fillPixel(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  color: string,
): void {
  softRoundedFill(ctx, x, y, w || 1, h || 1, color);
}

export function fillRow(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  pattern: string,
  colorMap: Record<string, string>,
): void {
  for (let i = 0; i < pattern.length; i++) {
    const ch = pattern[i];
    if (ch !== " " && colorMap[ch]) {
      softRoundedFill(ctx, x + i, y, 1, 1, colorMap[ch]);
    }
  }
}

export function fillRect(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  color: string,
): void {
  softRoundedFill(ctx, x, y, w, h, color);
}

export function fillCircle(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  radius: number,
  color: string,
): void {
  ctx.fillStyle = color;
  ctx.beginPath();
  ctx.arc(x, y, Math.max(0.2, radius), 0, Math.PI * 2);
  ctx.fill();
}

export function fillEllipse(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  radiusX: number,
  radiusY: number,
  color: string,
  rotation = 0,
): void {
  ctx.fillStyle = color;
  ctx.beginPath();
  ctx.ellipse(
    x,
    y,
    Math.max(0.2, radiusX),
    Math.max(0.2, radiusY),
    rotation,
    0,
    Math.PI * 2,
  );
  ctx.fill();
}

export function strokeSeg(
  ctx: CanvasRenderingContext2D,
  x1: number,
  y1: number,
  x2: number,
  y2: number,
  color: string,
  lineWidth = 1,
): void {
  ctx.save();
  ctx.strokeStyle = color;
  ctx.lineWidth = lineWidth;
  ctx.lineCap = "round";
  ctx.beginPath();
  ctx.moveTo(x1, y1);
  ctx.lineTo(x2, y2);
  ctx.stroke();
  ctx.restore();
}

export function strokeEllipse(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  radiusX: number,
  radiusY: number,
  color: string,
  lineWidth = 1,
): void {
  ctx.save();
  ctx.strokeStyle = color;
  ctx.lineWidth = lineWidth;
  ctx.beginPath();
  ctx.ellipse(x, y, radiusX, radiusY, 0, 0, Math.PI * 2);
  ctx.stroke();
  ctx.restore();
}

export function strokeArc(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  radius: number,
  startAngle: number,
  endAngle: number,
  color: string,
  lineWidth = 1,
): void {
  ctx.save();
  ctx.strokeStyle = color;
  ctx.lineWidth = lineWidth;
  ctx.lineCap = "round";
  ctx.beginPath();
  ctx.arc(x, y, radius, startAngle, endAngle);
  ctx.stroke();
  ctx.restore();
}

export function fillText(
  ctx: CanvasRenderingContext2D,
  text: string,
  x: number,
  y: number,
  size: number,
  color: string,
  align: CanvasTextAlign = "center",
): void {
  ctx.save();
  ctx.font = `${size}px ${pixelFontReady ? '"Press Start 2P",' : ""} monospace`;
  ctx.fillStyle = color;
  ctx.textAlign = align;
  ctx.textBaseline = "top";
  ctx.fillText(text, x, y);
  ctx.restore();
}
