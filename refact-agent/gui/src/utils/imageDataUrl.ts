export const OMITTED_DATA_PLACEHOLDER = "<omitted>";

export function buildImageDataUrl(mime: unknown, data: unknown): string | null {
  if (typeof mime !== "string" || !mime.startsWith("image/")) return null;
  if (typeof data !== "string" || data.length === 0) return null;
  if (data === OMITTED_DATA_PLACEHOLDER) return null;
  return `data:${mime};base64,${data}`;
}
