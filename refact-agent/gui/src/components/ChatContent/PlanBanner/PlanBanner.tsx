import React, {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import { CheckIcon, CopyIcon } from "@radix-ui/react-icons";
import { Box, Button, Container, Flex, Text } from "@radix-ui/themes";
import classNames from "classnames";
import { ClipboardList } from "lucide-react";
import { useAppSelector, useCopyToClipboard } from "../../../hooks";
import { selectPlanBannerState } from "../../../features/Chat/Thread/selectors";
import { Markdown } from "../../Markdown";
import { Icon } from "../../ui";
import styles from "./PlanBanner.module.css";
import { getPlanMetadata } from "../../../services/refact/types";
import { PlanHistoryModal } from "./PlanHistoryModal";
import { normalizePlanContent } from "./planContent";

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
  const copyToClipboard = useCopyToClipboard();
  const {
    base: plan,
    synthesizedText,
    history: planHistory,
  } = useAppSelector((state) => selectPlanBannerState(state, threadId));
  const [collapsed, setCollapsed] = useState(() => readCollapsed(threadId));
  const [historyOpen, setHistoryOpen] = useState(false);
  const [copied, setCopied] = useState(false);
  const [nowMs, setNowMs] = useState(() => Date.now());
  const copyTimerRef = useRef<number | null>(null);
  const bodyId = useId();
  const metadata = useMemo(
    () => (plan ? getPlanMetadata(plan) : undefined),
    [plan],
  );
  const planText = normalizePlanContent(synthesizedText ?? plan?.content ?? "");

  useEffect(() => {
    setCollapsed(readCollapsed(threadId));
  }, [threadId]);

  useEffect(() => {
    setNowMs(Date.now());
  }, [metadata?.created_at_ms]);

  useEffect(() => {
    setHistoryOpen(false);
  }, [threadId]);

  useEffect(() => {
    return () => {
      if (copyTimerRef.current !== null) {
        window.clearTimeout(copyTimerRef.current);
      }
    };
  }, []);

  const title = useMemo(() => {
    if (!plan) return "";
    const mode = metadata?.mode ?? "Mode unknown";
    const version =
      metadata?.version !== undefined ? `v${metadata.version}` : "v?";
    return `Plan — ${mode} · ${version} · ${humanizedAgeFrom(
      metadata?.created_at_ms,
      nowMs,
    )}`;
  }, [metadata, nowMs, plan]);

  const handleToggle = () => {
    const nextCollapsed = !collapsed;
    setCollapsed(nextCollapsed);
    writeCollapsed(threadId, nextCollapsed);
  };

  const handleHistoryClick = (event: React.MouseEvent<HTMLButtonElement>) => {
    event.stopPropagation();
    setHistoryOpen(true);
  };

  const handleCopyClick = useCallback(
    (event: React.MouseEvent<HTMLButtonElement>) => {
      event.stopPropagation();
      if (!planText) return;

      copyToClipboard(planText);
      setCopied(true);

      if (copyTimerRef.current !== null) {
        window.clearTimeout(copyTimerRef.current);
      }
      copyTimerRef.current = window.setTimeout(() => {
        setCopied(false);
        copyTimerRef.current = null;
      }, 2000);
    },
    [copyToClipboard, planText],
  );

  if (!plan) return null;

  return (
    <Box className={styles.sticky} data-testid="plan-banner">
      <Container className={styles.container}>
        <Box className={styles.card} data-testid="plan-banner-card">
          <Flex
            align="center"
            gap="2"
            className={styles.header}
            data-testid="plan-banner-header"
          >
            <button
              type="button"
              className={styles.toggleButton}
              onClick={handleToggle}
              aria-expanded={!collapsed}
              aria-controls={bodyId}
            >
              <span className={styles.icon}>
                <Icon icon={ClipboardList} size="sm" />
              </span>
              <Text size="1" className={styles.title}>
                {title}
              </Text>
            </button>
            <span className={styles.actions}>
              <button
                type="button"
                className={classNames(
                  styles.actionButton,
                  copied && styles.copiedButton,
                )}
                onClick={handleCopyClick}
                title="Copy plan"
                aria-label="Copy plan"
              >
                {copied ? <CheckIcon /> : <CopyIcon />}
              </button>
              <Button
                type="button"
                size="1"
                variant="ghost"
                color="gray"
                onClick={handleHistoryClick}
              >
                History
              </Button>
            </span>
          </Flex>
          {!collapsed && (
            <Box
              id={bodyId}
              className={styles.body}
              data-testid="plan-banner-body"
            >
              <Markdown>{planText}</Markdown>
            </Box>
          )}
        </Box>
      </Container>
      <PlanHistoryModal
        open={historyOpen}
        onOpenChange={setHistoryOpen}
        items={planHistory}
      />
    </Box>
  );
};
