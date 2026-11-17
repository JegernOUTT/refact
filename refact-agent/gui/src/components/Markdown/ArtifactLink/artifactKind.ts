export type ArtifactKind =
  | "image"
  | "svg"
  | "html"
  | "markdown"
  | "csv"
  | "tsv"
  | "text"
  | "none";

const IMAGE_EXTENSIONS = new Set(["png", "jpg", "jpeg", "gif", "webp"]);
const TEXT_EXTENSIONS = new Set([
  "c",
  "cc",
  "conf",
  "cpp",
  "cs",
  "css",
  "go",
  "h",
  "hpp",
  "ini",
  "java",
  "js",
  "json",
  "jsonl",
  "jsx",
  "kt",
  "log",
  "lua",
  "php",
  "py",
  "rb",
  "rs",
  "scss",
  "sh",
  "sql",
  "swift",
  "toml",
  "ts",
  "tsx",
  "txt",
  "xml",
  "yaml",
  "yml",
]);

const KIND_LABELS: Partial<Record<string, string>> = {
  htm: "HTML",
  md: "Markdown",
  mdx: "Markdown",
  txt: "Text",
  log: "Log",
};

export const MAX_TEXT_PREVIEW_BYTES = 2 * 1024 * 1024;
export const MAX_HTML_PREVIEW_BYTES = 1024 * 1024;
export const MAX_IMAGE_PREVIEW_BYTES = 10 * 1024 * 1024;

export function artifactExtension(fileName: string): string {
  const dot = fileName.lastIndexOf(".");
  return dot > 0 ? fileName.slice(dot + 1).toLowerCase() : "";
}

export function artifactKind(fileName: string): ArtifactKind {
  const extension = artifactExtension(fileName);
  if (IMAGE_EXTENSIONS.has(extension)) return "image";
  if (extension === "svg") return "svg";
  if (extension === "html" || extension === "htm") return "html";
  if (extension === "md" || extension === "mdx") return "markdown";
  if (extension === "csv") return "csv";
  if (extension === "tsv") return "tsv";
  if (TEXT_EXTENSIONS.has(extension)) return "text";
  return "none";
}

export function artifactTypeLabel(fileName: string): string {
  const extension = artifactExtension(fileName);
  if (!extension) return "File";
  return KIND_LABELS[extension] ?? extension.toUpperCase();
}

export function previewByteLimit(kind: ArtifactKind): number {
  if (kind === "image") return MAX_IMAGE_PREVIEW_BYTES;
  if (kind === "html") return MAX_HTML_PREVIEW_BYTES;
  if (kind === "none") return 0;
  return MAX_TEXT_PREVIEW_BYTES;
}

export function previewLineCount(kind: ArtifactKind): number | null {
  if (kind === "markdown") return 40;
  if (kind === "csv" || kind === "tsv") return 24;
  if (kind === "text") return 12;
  return null;
}

export function svgDataUrl(content: string): string {
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(content)}`;
}

const MAX_TABLE_ROWS = 6;
const MAX_TABLE_COLUMNS = 6;

export function parseDelimitedPreview(
  content: string,
  delimiter: "," | "\t",
): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let cell = "";
  let quoted = false;
  let index = 0;
  const pushCell = () => {
    if (row.length < MAX_TABLE_COLUMNS) row.push(cell);
    cell = "";
  };
  const pushRow = () => {
    pushCell();
    if (row.some((value) => value !== "")) rows.push(row);
    row = [];
  };
  while (index < content.length && rows.length < MAX_TABLE_ROWS) {
    const char = content[index];
    if (quoted) {
      if (char === '"' && content[index + 1] === '"') {
        cell += '"';
        index += 2;
        continue;
      }
      if (char === '"') quoted = false;
      else cell += char;
      index += 1;
      continue;
    }
    if (char === '"' && cell === "") quoted = true;
    else if (char === delimiter) pushCell();
    else if (char === "\n") pushRow();
    else if (char !== "\r") cell += char;
    index += 1;
  }
  if (rows.length < MAX_TABLE_ROWS && (cell !== "" || row.length > 0)) {
    pushRow();
  }
  return rows;
}
