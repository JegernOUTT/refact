export type ArtifactLink = {
  chatId: string;
  relPath: string;
};

const ARTIFACT_PREFIX = "refact://artifact/";
const CHAT_ID_PATTERN = /^[A-Za-z0-9_-]{1,128}$/;
const MAX_REL_PATH_LENGTH = 1024;

function hasForbiddenChar(segment: string): boolean {
  return Array.from(segment).some((char) => {
    const code = char.charCodeAt(0);
    return code < 0x20 || code === 0x7f || char === "/" || char === "\\";
  });
}

function decodeSegment(segment: string): string | null {
  if (!segment) return null;
  let decoded: string;
  try {
    decoded = decodeURIComponent(segment);
  } catch {
    return null;
  }
  if (!decoded || decoded === "." || decoded === "..") return null;
  if (hasForbiddenChar(decoded)) return null;
  return decoded;
}

export function parseArtifactLink(
  href: string | null | undefined,
): ArtifactLink | null {
  if (!href || !href.startsWith(ARTIFACT_PREFIX)) return null;
  const rest = href.slice(ARTIFACT_PREFIX.length);
  if (rest.includes("?") || rest.includes("#")) return null;
  const separator = rest.indexOf("/");
  if (separator <= 0) return null;
  const chatId = rest.slice(0, separator);
  if (!CHAT_ID_PATTERN.test(chatId)) return null;
  const rawSegments = rest.slice(separator + 1).split("/");
  const segments: string[] = [];
  for (const rawSegment of rawSegments) {
    const decoded = decodeSegment(rawSegment);
    if (decoded === null) return null;
    segments.push(decoded);
  }
  const relPath = segments.join("/");
  if (relPath.length > MAX_REL_PATH_LENGTH) return null;
  return { chatId, relPath };
}

export function artifactFileName(relPath: string): string {
  const parts = relPath.split("/");
  return parts[parts.length - 1] || relPath;
}
