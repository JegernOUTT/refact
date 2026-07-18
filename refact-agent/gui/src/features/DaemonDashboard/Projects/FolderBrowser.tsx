import { useEffect } from "react";
import { ChevronRight, Folder, GitBranch, MoveUp } from "lucide-react";

import {
  Badge,
  Button,
  FieldError,
  Icon,
  LoadingState,
} from "../../../components/ui";
import { useBrowseFoldersMutation } from "../../../services/refact/daemon";
import styles from "./Projects.module.css";

type FolderBrowserProps = {
  onSelect: (path: string) => void;
};

function browseError(error: unknown): string | null {
  if (!error || typeof error !== "object" || !("data" in error)) return null;
  const data = error.data;
  if (typeof data === "string") return data;
  if (data && typeof data === "object" && "detail" in data) {
    return typeof data.detail === "string" ? data.detail : null;
  }
  return null;
}

function joinPath(parent: string, child: string): string {
  if (parent.endsWith("/") || parent.endsWith("\\")) return `${parent}${child}`;
  const separator = parent.includes("\\") && !parent.includes("/") ? "\\" : "/";
  return `${parent}${separator}${child}`;
}

export function FolderBrowser({ onSelect }: FolderBrowserProps) {
  const [browse, { data, error, isLoading }] = useBrowseFoldersMutation();

  useEffect(() => {
    void browse({});
  }, [browse]);

  return (
    <div className={styles.browser}>
      <div className={styles.browserToolbar}>
        <Button
          aria-label="Browse parent folder"
          disabled={isLoading || !data?.parent}
          leftIcon={MoveUp}
          onClick={() => void browse({ path: data?.parent ?? undefined })}
          size="sm"
          variant="ghost"
        >
          Up
        </Button>
        <span className={styles.breadcrumb} title={data?.path}>
          {data?.path ?? "Home"}
        </span>
        <Button
          disabled={isLoading || !data?.can_open}
          onClick={() => data && onSelect(data.path)}
          size="sm"
          variant="soft"
        >
          Select this folder
        </Button>
      </div>

      {isLoading ? (
        <LoadingState label="Browsing folders" />
      ) : (
        <div className={styles.folderList} aria-label="Folders">
          {data?.dirs.map((directory) => (
            <button
              className={styles.folderRow}
              disabled={isLoading}
              key={directory.name}
              onClick={() =>
                void browse({ path: joinPath(data.path, directory.name) })
              }
              type="button"
            >
              <Icon icon={Folder} size="sm" tone="muted" />
              <span>{directory.name}</span>
              {directory.has_git ? (
                <Badge size="xs" tone="accent" variant="soft">
                  <Icon icon={GitBranch} size="sm" /> Git
                </Badge>
              ) : null}
              <Icon icon={ChevronRight} size="sm" tone="muted" />
            </button>
          ))}
          {data && data.dirs.length === 0 ? (
            <span className={styles.muted}>No child folders</span>
          ) : null}
        </div>
      )}

      {data?.truncated ? (
        <span className={styles.muted}>Showing the first 500 folders.</span>
      ) : null}
      {error ? (
        <FieldError>
          {browseError(error) ?? "Unable to browse folders."}
        </FieldError>
      ) : null}
    </div>
  );
}
