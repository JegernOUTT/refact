import React, { useEffect, useMemo, useState } from "react";
import { Box, Button, Card, Flex, Text } from "@radix-ui/themes";
import { shallowEqual } from "react-redux";
import { useAppSelector } from "../../../hooks";
import {
  selectCurrentPlan,
  selectPlanHistory,
} from "../../../features/Chat/Thread/selectors";
import { Markdown } from "../../Markdown";
import { PlanEditor } from "./PlanEditor";
import { PlanHistoryModal } from "./PlanHistoryModal";
import styles from "./PlanBanner.module.css";
import { getPlanMetadata } from "../../../services/refact/types";

type PlanBannerProps = {
  threadId: string;
};

const MINUTE_MS = 60_000;
const HOUR_MS = 60 * MINUTE_MS;
const DAY_MS = 24 * HOUR_MS;

function humanizedAgeFrom(
  createdAtMs: number | undefined,
  nowMs: number,
): string {
  if (createdAtMs === undefined) return "recently";
  const ageMs = Math.max(0, nowMs - createdAtMs);
  if (!Number.isFinite(ageMs)) return "recently";
  if (ageMs < MINUTE_MS) return "just now";
  if (ageMs < HOUR_MS) return `${Math.floor(ageMs / MINUTE_MS)}m ago`;
  if (ageMs < DAY_MS) return `${Math.floor(ageMs / HOUR_MS)}h ago`;
  if (ageMs < 2 * DAY_MS) return "yesterday";
  return `${Math.floor(ageMs / DAY_MS)} days ago`;
}

function storageKey(threadId: string): string {
  return `plan-banner-collapsed-${threadId}`;
}

function readCollapsed(threadId: string): boolean {
  try {
    if (typeof localStorage === "undefined") return false;
    return localStorage.getItem(storageKey(threadId)) === "true";
  } catch {
    return false;
  }
}

function writeCollapsed(threadId: string, collapsed: boolean): void {
  try {
    if (typeof localStorage === "undefined") return;
    localStorage.setItem(storageKey(threadId), String(collapsed));
  } catch {
    return;
  }
}

export const PlanBanner: React.FC<PlanBannerProps> = ({ threadId }) => {
  const plan = useAppSelector((state) => selectCurrentPlan(state, threadId));
  const history = useAppSelector(
    (state) => selectPlanHistory(state, threadId),
    shallowEqual,
  );
  const [collapsed, setCollapsed] = useState(() => readCollapsed(threadId));
  const [editorOpen, setEditorOpen] = useState(false);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [nowMs, setNowMs] = useState(() => Date.now());
  const metadata = useMemo(
    () => (plan ? getPlanMetadata(plan) : undefined),
    [plan],
  );

  useEffect(() => {
    setCollapsed(readCollapsed(threadId));
  }, [threadId]);

  useEffect(() => {
    setNowMs(Date.now());
  }, [metadata?.created_at_ms]);

  const header = useMemo(() => {
    if (!plan) return "";
    const mode = metadata?.mode ?? "Mode unknown";
    const version =
      metadata?.version !== undefined ? `v${metadata.version}` : "v?";
    return `📋 Plan — ${mode} · ${version} · ${humanizedAgeFrom(
      metadata?.created_at_ms,
      nowMs,
    )}`;
  }, [metadata, nowMs, plan]);

  if (!plan) return null;

  const handleToggle = () => {
    const nextCollapsed = !collapsed;
    setCollapsed(nextCollapsed);
    writeCollapsed(threadId, nextCollapsed);
  };

  return (
    <Box className={styles.sticky} data-testid="plan-banner">
      <Card className={styles.card}>
        <Flex
          align="center"
          justify="between"
          gap="3"
          className={styles.header}
        >
          <Text as="div" size="2" weight="bold" className={styles.title}>
            {header}
          </Text>
          <Flex align="center" gap="2" className={styles.actions}>
            <Button
              type="button"
              size="1"
              variant="soft"
              onClick={() => setEditorOpen(true)}
            >
              Edit
            </Button>
            <Button
              type="button"
              size="1"
              variant="soft"
              onClick={handleToggle}
            >
              {collapsed ? "Expand" : "Collapse"}
            </Button>
            <Button
              type="button"
              size="1"
              variant="soft"
              color="gray"
              onClick={() => setHistoryOpen(true)}
            >
              History
            </Button>
          </Flex>
        </Flex>
        {!collapsed && (
          <Box className={styles.body} data-testid="plan-banner-body">
            <Markdown>{plan.content}</Markdown>
          </Box>
        )}
      </Card>
      <PlanEditor
        open={editorOpen}
        content={plan.content}
        onOpenChange={setEditorOpen}
      />
      <PlanHistoryModal
        open={historyOpen}
        onOpenChange={setHistoryOpen}
        plans={history}
      />
    </Box>
  );
};
