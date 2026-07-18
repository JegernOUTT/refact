import { FileTree } from "./FileTree";
import { FileViewer } from "./FileViewer";
import styles from "./FilesPanel.module.css";

export function FilesPanel() {
  return (
    <div className={styles.panel}>
      <aside className={styles.explorer} aria-label="File explorer">
        <div className={styles.explorerHeader}>Explorer</div>
        <FileTree />
      </aside>
      <main className={styles.viewerPane}>
        <FileViewer />
      </main>
    </div>
  );
}
