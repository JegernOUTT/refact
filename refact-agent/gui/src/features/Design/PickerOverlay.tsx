import type { DesignElementSelection } from "./surfaceContract";
import styles from "./Design.module.css";

type PickerOverlayProps = {
  selection: DesignElementSelection | null;
};

export function PickerOverlay({ selection }: PickerOverlayProps) {
  if (!selection) return null;
  return (
    <aside className={styles.pickerResult} aria-label="Selected design element">
      <strong>{selection.name || selection.role || selection.selector}</strong>
      <span>{selection.selector}</span>
      {selection.sourceFile ? (
        <span>
          {selection.sourceFile}
          {selection.line === null ? "" : `:${selection.line}`}
        </span>
      ) : null}
      {selection.cropDataUrl ? (
        <img src={selection.cropDataUrl} alt="Selected element crop" />
      ) : null}
    </aside>
  );
}
