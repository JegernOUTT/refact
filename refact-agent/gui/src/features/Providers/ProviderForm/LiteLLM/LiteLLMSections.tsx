import React from "react";

import { SegmentedControl } from "../../../../components/ui";
import { LiteLLMAccountPanel } from "./LiteLLMAccountPanel";
import { LiteLLMUsagePanel } from "./LiteLLMUsagePanel";
import { LiteLLMMcpPanel } from "./LiteLLMMcpPanel";
import styles from "./LiteLLM.module.css";

type SectionId = "models" | "quotas" | "mcp";

const SECTION_OPTIONS = [
  { value: "models", label: "Models" },
  { value: "quotas", label: "Quotas & Usage" },
  { value: "mcp", label: "MCP Servers" },
];

export type LiteLLMSectionsProps = {
  providerName: string;
  models: React.ReactNode;
  hasCredentials: boolean;
};

export const LiteLLMSections: React.FC<LiteLLMSectionsProps> = ({
  providerName,
  models,
  hasCredentials,
}) => {
  const [section, setSection] = React.useState<SectionId>("models");
  const idBase = React.useId();

  return (
    <div className={styles.section} key={providerName}>
      <div className={styles.tabs}>
        <SegmentedControl
          aria-label="LiteLLM provider sections"
          layout="content"
          size="sm"
          value={section}
          options={SECTION_OPTIONS}
          onValueChange={(value) => setSection(value as SectionId)}
        />
      </div>
      <div id={`${idBase}-panel`} aria-live="polite">
        {section === "models" ? models : null}
        {section === "quotas" ? (
          hasCredentials ? (
            <div className={styles.section}>
              <LiteLLMAccountPanel providerName={providerName} />
              <LiteLLMUsagePanel providerName={providerName} />
            </div>
          ) : (
            <div className={styles.notice} role="status">
              Configure this provider to see quotas and usage.
            </div>
          )
        ) : null}
        {section === "mcp" ? (
          hasCredentials ? (
            <LiteLLMMcpPanel providerName={providerName} />
          ) : (
            <div className={styles.notice} role="status">
              Configure this provider to discover MCP servers.
            </div>
          )
        ) : null}
      </div>
    </div>
  );
};
