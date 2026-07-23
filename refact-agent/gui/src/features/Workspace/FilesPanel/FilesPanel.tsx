import type { CSSProperties } from "react";

import { FileTree } from "./FileTree";
import { FileViewer } from "./FileViewer";
import { useExplorerSplitter } from "./useExplorerSplitter";
import styles from "./FilesPanel.module.css";

type PanelStyle = CSSProperties & {
  "--files-explorer-w": string;
};

export function FilesPanel() {
  const { panelRef, width, dragging, handleSplitterPointerDown } =
    useExplorerSplitter();

  return (
    <div
      className={styles.panel}
      data-testid="files-panel"
      ref={panelRef}
      style={{ "--files-explorer-w": `${width}px` } as PanelStyle}
    >
      <aside className={styles.explorer} aria-label="File explorer">
        <div className={styles.explorerHeader}>Explorer</div>
        <FileTree />
        <div
          aria-label="Resize file explorer"
          aria-orientation="vertical"
          className={styles.splitter}
          data-dragging={dragging || undefined}
          onPointerDown={handleSplitterPointerDown}
          role="separator"
        >
          <div className={styles.splitterHandle} />
        </div>
      </aside>
      <main className={styles.viewerPane}>
        <FileViewer />
      </main>
    </div>
  );
}
