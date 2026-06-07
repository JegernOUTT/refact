import React from "react";
import * as HoverCardPrimitive from "@radix-ui/react-hover-card";
import * as RadioGroupPrimitive from "@radix-ui/react-radio-group";
import { CircleHelp } from "lucide-react";

import { type ProjectLabelInfo } from "../../../utils/createProjectLabelsWithConflictMarkers";
import { Icon, Surface } from "../../ui";
import styles from "./IntermediateIntegration.module.css";

export type IntegrationPathFieldProps = {
  configPath: string;
  projectPath: string;
  projectLabels: ProjectLabelInfo[];
  shouldBeFormatted: boolean;
  globalLabel?: string;
};

export const IntegrationPathField: React.FC<IntegrationPathFieldProps> = ({
  configPath,
  projectPath,
  projectLabels,
  shouldBeFormatted,
  globalLabel = "Global, available for all projects",
}) => {
  if (!shouldBeFormatted) {
    return (
      <span className={styles.pathOption}>
        <RadioGroupPrimitive.Item value={configPath} /> {globalLabel}
      </span>
    );
  }

  const projectInfo = projectLabels.find((info) => info.path === projectPath);

  if (!projectInfo) {
    return (
      <span className={styles.pathOption}>
        <RadioGroupPrimitive.Item value={configPath} /> {projectPath}
      </span>
    );
  }

  const content = (
    <span className={styles.pathOption}>
      <RadioGroupPrimitive.Item value={configPath} /> {projectInfo.label}
    </span>
  );

  if (projectInfo.hasConflict) {
    return (
      <span className={styles.pathOption}>
        {content}
        <HoverCardPrimitive.Root>
          <HoverCardPrimitive.Trigger asChild>
            <span className={styles.pathHelp}>
              <Icon icon={CircleHelp} size="sm" tone="muted" />
            </span>
          </HoverCardPrimitive.Trigger>
          <HoverCardPrimitive.Portal>
          <HoverCardPrimitive.Portal>
            <HoverCardPrimitive.Content className="rf-popover-motion" side="right" align="center">
              <Surface variant="overlay" radius="card">
                <p className={styles.text}>Full project path:</p>
                <p className={styles.text}>{projectInfo.fullPath}</p>
              </Surface>
            </HoverCardPrimitive.Content>
          </HoverCardPrimitive.Portal>          </HoverCardPrimitive.Portal>
        </HoverCardPrimitive.Root>
      </span>
    );
  }

  return content;
};
