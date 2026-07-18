import { skipToken } from "@reduxjs/toolkit/query";

import { Badge, Spinner } from "../../../components/ui";
import { Markdown } from "../../../components/Markdown";
import { useGetGitDiffQuery } from "../../../services/refact/gitRead";
import { worktreeErrorText } from "../../Worktrees/worktreeError";
import type { SelectedGitFile } from "./StatusList";
import styles from "./GitPanel.module.css";

type DiffViewProps = {
  root: string;
  selected: SelectedGitFile | null;
};

function diffMarkdown(patch: string): string {
  const safePatch = patch.split("```").join("`\u200b``");
  return `\`\`\`diff\n${safePatch}\n\`\`\``;
}

export function DiffView({ root, selected }: DiffViewProps) {
  const request = selected
    ? { root, path: selected.path, staged: selected.staged }
    : skipToken;
  const { data, isLoading, isFetching, error } = useGetGitDiffQuery(request);
  const result = data?.roots[0];

  return (
    <section className={styles.section} aria-labelledby="git-diff-heading">
      <header className={styles.sectionHeader}>
        <div>
          <h2 id="git-diff-heading">Diff</h2>
          <p>
            {selected
              ? `${selected.staged ? "Staged" : "Working tree"} · ${
                  selected.path
                }`
              : "Select a changed file to inspect its patch."}
          </p>
        </div>
        {result?.truncated ? <Badge tone="warning">Truncated</Badge> : null}
      </header>
      {!selected ? (
        <p className={styles.emptyText}>No file selected.</p>
      ) : isLoading || isFetching ? (
        <div className={styles.loadingBlock}>
          <Spinner label="Loading diff" />
        </div>
      ) : error ? (
        <p className={styles.errorText} role="alert">
          {worktreeErrorText(error)}
        </p>
      ) : result?.patch ? (
        <div className={`${styles.diffScroller} scrollX`}>
          <Markdown variant="tool" wrap={false}>
            {diffMarkdown(result.patch)}
          </Markdown>
        </div>
      ) : (
        <p className={styles.emptyText}>No patch is available for this file.</p>
      )}
    </section>
  );
}
