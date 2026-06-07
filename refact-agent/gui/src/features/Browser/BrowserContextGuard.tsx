import React, { useCallback, useMemo, useState } from "react";
import { TriangleAlert } from "lucide-react";
import { Button, Icon, Surface } from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  selectBrowserContextOversize,
  clearBrowserContextOversize,
} from "./browserSlice";
import { selectChatId } from "../Chat/Thread";
import {
  abortGeneration,
  sendBrowserContextDecision,
} from "../../services/refact/chatCommands";
import { formatKB, estimateSize } from "./BrowserContextGuard.utils";
import styles from "./BrowserContextGuard.module.css";

type BrowserContextGuardProps = {
  chatId: string;
};

export const BrowserContextGuard: React.FC<BrowserContextGuardProps> = ({
  chatId,
}) => {
  const dispatch = useAppDispatch();
  const oversizeInfo = useAppSelector((state) =>
    selectBrowserContextOversize(state, chatId),
  );

  const currentChatId = useAppSelector(selectChatId);
  const config = useAppSelector((state) => state.config);
  const apiKey = useAppSelector((state) => state.config.apiKey);

  const [includeActions, setIncludeActions] = useState(true);
  const [includeConsole, setIncludeConsole] = useState(true);
  const [includeNetwork, setIncludeNetwork] = useState(true);
  const [includeMutations, setIncludeMutations] = useState(true);
  const [includeScreenshot, setIncludeScreenshot] = useState(false);
  const [lastNActions, setLastNActions] = useState(
    oversizeInfo?.action_count ?? 50,
  );
  const [lastNConsole] = useState(oversizeInfo?.console_count ?? 100);
  const [lastNNetwork] = useState(oversizeInfo?.network_count ?? 100);

  const info = oversizeInfo;

  const estimated = useMemo(() => {
    if (!info) return 0;
    return estimateSize(info, {
      includeActions,
      includeConsole,
      includeNetwork,
      includeMutations,
      includeScreenshot,
      lastNActions,
      lastNConsole,
      lastNNetwork,
    });
  }, [
    info,
    includeActions,
    includeConsole,
    includeNetwork,
    includeMutations,
    includeScreenshot,
    lastNActions,
    lastNConsole,
    lastNNetwork,
  ]);

  const handleIncludeAll = useCallback(async () => {
    if (!info) return;
    await sendBrowserContextDecision(
      chatId,
      config,
      {
        pending_message_id: info.pending_message_id,
        include_actions: true,
        include_console: true,
        include_network: true,
        include_mutations: true,
        include_screenshot: false,
        last_n_actions: info.action_count,
        last_n_console: info.console_count,
        last_n_network: info.network_count,
      },
      apiKey ?? undefined,
    );
    dispatch(clearBrowserContextOversize({ chatId }));
  }, [chatId, config, apiKey, info, dispatch]);

  const handleIncludeSelected = useCallback(async () => {
    if (!info) return;
    await sendBrowserContextDecision(
      chatId,
      config,
      {
        pending_message_id: info.pending_message_id,
        include_actions: includeActions,
        include_console: includeConsole,
        include_network: includeNetwork,
        include_mutations: includeMutations,
        include_screenshot: includeScreenshot,
        last_n_actions: lastNActions,
        last_n_console: lastNConsole,
        last_n_network: lastNNetwork,
      },
      apiKey ?? undefined,
    );
    dispatch(clearBrowserContextOversize({ chatId }));
  }, [
    chatId,
    config,
    apiKey,
    info,
    includeActions,
    includeConsole,
    includeNetwork,
    includeMutations,
    includeScreenshot,
    lastNActions,
    lastNConsole,
    lastNNetwork,
    dispatch,
  ]);

  const handleSkipContext = useCallback(async () => {
    if (!info) return;
    await sendBrowserContextDecision(
      chatId,
      config,
      {
        pending_message_id: info.pending_message_id,
        include_actions: false,
        include_console: false,
        include_network: false,
        include_mutations: false,
        include_screenshot: false,
        last_n_actions: 0,
        last_n_console: 0,
        last_n_network: 0,
      },
      apiKey ?? undefined,
    );
    dispatch(clearBrowserContextOversize({ chatId }));
  }, [chatId, config, apiKey, info, dispatch]);

  const handleCancelSend = useCallback(async () => {
    await abortGeneration(chatId, config, apiKey ?? undefined);
    dispatch(clearBrowserContextOversize({ chatId }));
  }, [chatId, config, apiKey, dispatch]);

  if (!info || chatId !== currentChatId) return null;

  return (
    <Surface className={styles.guardCard} variant="surface-1">
      <div className={styles.heading}>
        <Icon icon={TriangleAlert} tone="warning" />
        <span>Browser context is large ({formatKB(info.total_bytes)})</span>
      </div>

      <div className={styles.breakdownGrid}>
        <span className={styles.breakdownLabel}>Actions:</span>
        <span className={styles.breakdownCount}>{info.action_count}</span>
        <span className={styles.breakdownSize}>{formatKB(info.action_bytes)}</span>

        <span className={styles.breakdownLabel}>Console:</span>
        <span className={styles.breakdownCount}>{info.console_count}</span>
        <span className={styles.breakdownSize}>{formatKB(info.console_bytes)}</span>

        <span className={styles.breakdownLabel}>Network:</span>
        <span className={styles.breakdownCount}>{info.network_count}</span>
        <span className={styles.breakdownSize}>{formatKB(info.network_bytes)}</span>

        <span className={styles.breakdownLabel}>Mutations:</span>
        <span className={styles.breakdownCount}>—</span>
        <span className={styles.breakdownSize}>{formatKB(info.mutation_bytes)}</span>
      </div>

      <div className={styles.sliderContainer}>
        <label className={styles.sliderLabel}>
          Include last {lastNActions} actions
        </label>
        <input
          type="range"
          className={styles.slider}
          min={0}
          max={info.action_count}
          value={lastNActions}
          onChange={(e) => setLastNActions(Number(e.target.value))}
        />
      </div>

      <div className={styles.checkboxGroup}>
        <label className={styles.checkboxItem}>
          <input
            type="checkbox"
            checked={includeActions}
            onChange={(e) => setIncludeActions(e.target.checked)}
          />
          Actions
        </label>
        <label className={styles.checkboxItem}>
          <input
            type="checkbox"
            checked={includeConsole}
            onChange={(e) => setIncludeConsole(e.target.checked)}
          />
          Console
        </label>
        <label className={styles.checkboxItem}>
          <input
            type="checkbox"
            checked={includeNetwork}
            onChange={(e) => setIncludeNetwork(e.target.checked)}
          />
          Network
        </label>
        <label className={styles.checkboxItem}>
          <input
            type="checkbox"
            checked={includeMutations}
            onChange={(e) => setIncludeMutations(e.target.checked)}
          />
          Mutations
        </label>
        <label className={styles.checkboxItem}>
          <input
            type="checkbox"
            checked={includeScreenshot}
            onChange={(e) => setIncludeScreenshot(e.target.checked)}
          />
          Screenshot
        </label>
      </div>

      <p className={styles.liveTotal}>Estimated: {formatKB(estimated)}</p>

      <div className={styles.actions}>
        <Button size="sm" variant="soft" onClick={() => void handleIncludeAll()}>
          Include All
        </Button>
        <Button
          size="sm"
          variant="primary"
          onClick={() => void handleIncludeSelected()}
        >
          Include Selected
        </Button>
        <Button size="sm" variant="soft" onClick={() => void handleSkipContext()}>
          Skip Context
        </Button>
        <Button size="sm" variant="danger" onClick={() => void handleCancelSend()}>
          Cancel Send
        </Button>
      </div>
    </Surface>
  );
};
