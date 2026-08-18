import { Box, Flex } from "@radix-ui/themes";
import { FileCode } from "lucide-react";

import type { BrowserPageSnapshot } from "../../../services/refact/browser";
import { Badge, Icon } from "../../ui";
import { AnimatedCollapsible } from "../shared/AnimatedCollapsible";
import { AriaSnapshotView } from "./AriaSnapshotView";
import styles from "./PageSnapshot.module.css";

export interface PageSnapshotProps {
  snapshot: BrowserPageSnapshot;
}

function formatSize(value: number): string {
  if (value < 1_024) return `${Math.round(value)} B`;
  if (value < 1_048_576) return `${(value / 1_024).toFixed(1)} KB`;
  return `${(value / 1_048_576).toFixed(1)} MB`;
}

export function PageSnapshot({ snapshot }: PageSnapshotProps) {
  const artifact = snapshot.artifact ?? null;
  const header = `Page Snapshot — ${snapshot.lines} line${
    snapshot.lines === 1 ? "" : "s"
  } · ${formatSize(snapshot.bytes)}`;

  return (
    <Box className={styles.section}>
      <AnimatedCollapsible
        data-testid="page-snapshot"
        defaultOpen={!snapshot.truncated}
        header={header}
        icon={<Icon icon={FileCode} />}
        variant="compact"
      >
        {snapshot.truncated || artifact ? (
          <Flex align="center" className={styles.pointer} gap="2" wrap="wrap">
            {snapshot.truncated ? (
              <Badge
                data-testid="page-snapshot-truncated"
                size="xs"
                tone="warning"
              >
                Truncated
              </Badge>
            ) : null}
            {artifact ? (
              <>
                <Badge size="xs" tone="muted">
                  {artifact.mime}
                </Badge>
                <span className={styles.path} title={artifact.path}>
                  {artifact.path}
                </span>
                <span className={styles.meta}>
                  {formatSize(artifact.bytes)}
                </span>
              </>
            ) : null}
          </Flex>
        ) : null}
        <AriaSnapshotView yaml={snapshot.yaml} />
      </AnimatedCollapsible>
    </Box>
  );
}
