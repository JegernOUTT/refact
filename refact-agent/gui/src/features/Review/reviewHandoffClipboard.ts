export interface HandoffExport {
  review_id: string;
  revision: number;
  stack_revision: number | null;
  markdown: string;
  filename: string;
  scope: string;
}

export async function copyReviewMarkdown(text: string): Promise<boolean> {
  try {
    if (
      typeof navigator.clipboard === "undefined" ||
      typeof navigator.clipboard.writeText !== "function"
    )
      return false;
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

export function downloadReviewMarkdown(data: HandoffExport): void {
  const url = URL.createObjectURL(
    new Blob([data.markdown], { type: "text/markdown;charset=utf-8" }),
  );
  let link: HTMLAnchorElement | undefined;
  try {
    link = document.createElement("a");
    link.href = url;
    link.download =
      data.filename.replace(/[^a-zA-Z0-9._-]/g, "_").replace(/^\.+/, "") ||
      "review.md";
    document.body.appendChild(link);
    link.click();
  } finally {
    try {
      link?.remove();
    } finally {
      setTimeout(() => URL.revokeObjectURL(url), 1000);
    }
  }
}
