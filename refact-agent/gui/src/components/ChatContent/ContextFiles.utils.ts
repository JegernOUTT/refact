const BYTES_PER_KB = 1024;

export function formatTextSize(text: string): string {
  const bytes = new TextEncoder().encode(text).length;
  if (bytes < BYTES_PER_KB) return `${bytes} B`;
  const kb = bytes / BYTES_PER_KB;
  if (kb < BYTES_PER_KB) return `${kb.toFixed(1)} KB`;
  return `${(kb / BYTES_PER_KB).toFixed(1)} MB`;
}
