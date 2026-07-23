import { FileTree } from "./FileTree";
import styles from "./FilesPanel.module.css";

export function FilesPanel() {
  return (
    <div className={styles.panel} data-testid="files-panel">
      <aside className={styles.explorer} aria-label="File explorer">
        <div className={styles.explorerHeader}>Explorer</div>
        <FileTree />
      </aside>
    </div>
  );
}
