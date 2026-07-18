import { SettingsSection } from "../../Settings/SettingsSection";
import { DaemonSection } from "./DaemonSection";
import { DangerSection } from "./DangerSection";
import { ProvidersSection } from "./ProvidersSection";
import { UpdatesSection } from "./UpdatesSection";
import styles from "./SettingsPage.module.css";

export function SettingsPage() {
  return (
    <SettingsSection
      className={styles.page}
      title="Settings"
      description="Configure daemon access, project-scoped providers, updates, and recovery controls."
      width="wide"
    >
      <DaemonSection />
      <ProvidersSection />
      <UpdatesSection />
      <DangerSection />
    </SettingsSection>
  );
}
