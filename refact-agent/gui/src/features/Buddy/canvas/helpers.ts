let pixelFontReady = false;
void document.fonts.load('8px "Press Start 2P"').then(() => {
  pixelFontReady = true;
});

export function fillPixel(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  color: string,
): void {
  ctx.fillStyle = color;
  ctx.fillRect(x | 0, y | 0, w || 1, h || 1);
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
      ctx.fillStyle = colorMap[ch];
      ctx.fillRect(x + i, y, 1, 1);
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
  ctx.fillStyle = color;
  ctx.fillRect(x | 0, y | 0, w, h);
}

export function strokeRect(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  color: string,
): void {
  ctx.fillStyle = color;
  ctx.fillRect(x, y, w, 1);
  ctx.fillRect(x, y + h - 1, w, 1);
  ctx.fillRect(x, y, 1, h);
  ctx.fillRect(x + w - 1, y, 1, h);
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
