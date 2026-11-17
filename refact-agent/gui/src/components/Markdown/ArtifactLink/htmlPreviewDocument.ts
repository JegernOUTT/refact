import { wrapArtifactHtml } from "../renderUtils";

export const HTML_PREVIEW_POLICY =
  "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:; base-uri about:; form-action 'none';";

/**
 * Builds the `srcDoc` for a sandboxed (`allow-scripts` only) HTML preview:
 * strips `<base>` and meta refreshes, then pins a strict CSP and an
 * `about:srcdoc` base as the first head children.
 */
export function buildHtmlPreviewDocument(content: string): string {
  const document = new DOMParser().parseFromString(
    `<!DOCTYPE html><meta http-equiv="Content-Security-Policy" content="${HTML_PREVIEW_POLICY}">${wrapArtifactHtml(
      content,
    )}`,
    "text/html",
  );
  document
    .querySelectorAll('base, meta[http-equiv="refresh" i]')
    .forEach((node) => node.remove());
  const base = document.createElement("base");
  base.href = "about:srcdoc";
  const policy = document.createElement("meta");
  policy.httpEquiv = "Content-Security-Policy";
  policy.content = HTML_PREVIEW_POLICY;
  document.head.prepend(policy, base);
  return `<!DOCTYPE html>\n${document.documentElement.outerHTML}`;
}
