import {
  Database,
  File,
  FileArchive,
  FileCode,
  FileCog,
  FileImage,
  FileJson,
  FileText,
  type LucideIcon,
} from "lucide-react";

const CODE_EXTENSIONS = new Set([
  "c",
  "cc",
  "cpp",
  "cs",
  "css",
  "cxx",
  "go",
  "h",
  "hpp",
  "html",
  "java",
  "js",
  "jsx",
  "kt",
  "kts",
  "lua",
  "php",
  "py",
  "rb",
  "rs",
  "scss",
  "sh",
  "sql",
  "swift",
  "ts",
  "tsx",
  "vue",
]);

const TEXT_EXTENSIONS = new Set(["log", "md", "mdx", "rst", "txt"]);
const CONFIG_EXTENSIONS = new Set([
  "conf",
  "config",
  "env",
  "ini",
  "properties",
  "toml",
  "xml",
  "yaml",
  "yml",
]);
const IMAGE_EXTENSIONS = new Set([
  "avif",
  "bmp",
  "gif",
  "ico",
  "jpeg",
  "jpg",
  "png",
  "svg",
  "webp",
]);
const ARCHIVE_EXTENSIONS = new Set([
  "7z",
  "bz2",
  "gz",
  "rar",
  "tar",
  "tgz",
  "zip",
]);
const DATABASE_EXTENSIONS = new Set(["db", "sqlite", "sqlite3"]);

export const fileTypeIcon = (name: string): LucideIcon => {
  const fileName = name.toLowerCase();
  if (["dockerfile", "makefile"].includes(fileName)) return FileCog;
  const extension = fileName.includes(".") ? fileName.split(".").pop() : "";
  if (!extension) return File;
  if (CODE_EXTENSIONS.has(extension)) return FileCode;
  if (extension === "json" || extension === "jsonl") return FileJson;
  if (TEXT_EXTENSIONS.has(extension)) return FileText;
  if (CONFIG_EXTENSIONS.has(extension)) return FileCog;
  if (IMAGE_EXTENSIONS.has(extension)) return FileImage;
  if (ARCHIVE_EXTENSIONS.has(extension)) return FileArchive;
  if (DATABASE_EXTENSIONS.has(extension)) return Database;
  return File;
};
