function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function str(record: Record<string, unknown>, key: string) {
  const value = record[key];
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

export function recordList(value: unknown, limit: number) {
  return Array.isArray(value) ? value.filter(isRecord).slice(0, limit) : [];
}

export function codeInterpreterOutputs(value: unknown): {
  text: string[];
  images: string[];
} {
  const outputs = recordList(value, 200);
  const text = outputs
    .map((output) => str(output, "text"))
    .filter((entry): entry is string => entry !== undefined);
  const images = outputs
    .map((output) => {
      const direct = str(output, "image_url");
      if (direct) return direct;
      return isRecord(output.image) ? str(output.image, "url") : undefined;
    })
    .filter((entry): entry is string => entry !== undefined);
  return { text, images };
}
