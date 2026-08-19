export const MAX_ATTACHED_IMAGES = 50;
export const MAX_ATTACHMENT_FILE_SIZE = 32 * 1024 * 1024;

export const SUPPORTED_IMAGE_MIME_TYPES = [
  "image/png",
  "image/jpeg",
  "image/webp",
  "image/gif",
  "image/avif",
  "image/bmp",
] as const;

const SUPPORTED_IMAGE_MIME_TYPE_SET = new Set<string>(
  SUPPORTED_IMAGE_MIME_TYPES,
);

const TEXT_FILE_EXTENSIONS = new Set([
  ".txt",
  ".md",
  ".json",
  ".yaml",
  ".yml",
  ".toml",
  ".xml",
  ".csv",
  ".js",
  ".ts",
  ".tsx",
  ".jsx",
  ".py",
  ".rs",
  ".go",
  ".java",
  ".kt",
  ".c",
  ".cpp",
  ".h",
  ".hpp",
  ".cs",
  ".rb",
  ".php",
  ".swift",
  ".sh",
  ".bash",
  ".zsh",
  ".html",
  ".css",
  ".scss",
  ".sass",
  ".less",
  ".sql",
  ".graphql",
  ".env",
  ".gitignore",
  ".dockerignore",
]);

export function isSupportedImageFile(file: File): boolean {
  return SUPPORTED_IMAGE_MIME_TYPE_SET.has(file.type.toLowerCase());
}

export function isSupportedTextFile(file: File): boolean {
  if (file.type.toLowerCase().startsWith("text/")) return true;
  const dotIndex = file.name.lastIndexOf(".");
  const extension =
    dotIndex >= 0 ? file.name.slice(dotIndex).toLowerCase() : "";
  return TEXT_FILE_EXTENSIONS.has(extension);
}

export function attachmentFileError(file: File): string | null {
  if (file.size > MAX_ATTACHMENT_FILE_SIZE) {
    return `Could not attach ${file.name}: file exceeds the 32 MB limit`;
  }
  if (isSupportedImageFile(file) || isSupportedTextFile(file)) return null;
  return `Could not attach ${file.name}: unsupported file type`;
}
