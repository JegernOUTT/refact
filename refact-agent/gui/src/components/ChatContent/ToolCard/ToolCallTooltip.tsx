import React, {
  useMemo,
  useState,
  useRef,
  useCallback,
  useEffect,
} from "react";
import { ToolCall } from "../../../services/refact/types";
import { Portal } from "../../Portal";
import styles from "./ToolCallTooltip.module.css";

const DELAY_MS = 10000;

function parseArgs(toolCall: ToolCall): [string, string][] {
  try {
    const parsed = JSON.parse(toolCall.function.arguments) as Record<
      string,
      unknown
    >;
    return Object.entries(parsed).map(([k, v]) => [
      k,
      typeof v === "string" ? v : JSON.stringify(v, null, 2),
    ]);
  } catch {
    if (toolCall.function.arguments) {
      return [["(raw)", toolCall.function.arguments]];
    }
    return [];
  }
}

interface ToolCallTooltipProps {
  toolCall: ToolCall;
  children: React.ReactNode;
}

export const ToolCallTooltip: React.FC<ToolCallTooltipProps> = ({
  toolCall,
  children,
}) => {
  const [visible, setVisible] = useState(false);
  const [pos, setPos] = useState({ x: 0, y: 0 });
  const openTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const closeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const wrapperRef = useRef<HTMLDivElement>(null);

  const toolName = toolCall.function.name ?? "unknown";
  const entries = useMemo(
    () => (visible ? parseArgs(toolCall) : []),
    [toolCall, visible],
  );

  const clearOpenTimer = useCallback(() => {
    if (openTimerRef.current) {
      clearTimeout(openTimerRef.current);
      openTimerRef.current = null;
    }
  }, []);

  const clearCloseTimer = useCallback(() => {
    if (closeTimerRef.current) {
      clearTimeout(closeTimerRef.current);
      closeTimerRef.current = null;
    }
  }, []);

  const scheduleClose = useCallback(() => {
    clearCloseTimer();
    closeTimerRef.current = setTimeout(() => {
      setVisible(false);
    }, 100);
  }, [clearCloseTimer]);

  const cancelClose = useCallback(() => {
    clearCloseTimer();
  }, [clearCloseTimer]);

  const handleWrapperEnter = useCallback(() => {
    cancelClose();
    openTimerRef.current = setTimeout(() => {
      if (wrapperRef.current) {
        const rect = wrapperRef.current.getBoundingClientRect();
        setPos({ x: rect.left, y: rect.top - 8 });
      }
      setVisible(true);
    }, DELAY_MS);
  }, [cancelClose]);

  const handleWrapperLeave = useCallback(() => {
    clearOpenTimer();
    scheduleClose();
  }, [clearOpenTimer, scheduleClose]);

  const handlePopupEnter = useCallback(() => {
    cancelClose();
  }, [cancelClose]);

  const handlePopupLeave = useCallback(() => {
    scheduleClose();
  }, [scheduleClose]);

  useEffect(() => {
    return () => {
      clearOpenTimer();
      clearCloseTimer();
    };
  }, [clearOpenTimer, clearCloseTimer]);

  return (
    <div
      ref={wrapperRef}
      onMouseEnter={handleWrapperEnter}
      onMouseLeave={handleWrapperLeave}
    >
      {children}
      {visible && (
        <Portal>
          <div
            className={styles.tooltipPopup}
            style={{ left: pos.x, top: pos.y, transform: "translateY(-100%)" }}
            onMouseEnter={handlePopupEnter}
            onMouseLeave={handlePopupLeave}
          >
            <div className={styles.toolName}>{toolName}</div>
            {entries.length > 0 && (
              <>
                <div className={styles.separator} />
                <div className={styles.args}>
                  {entries.map(([key, value]) => (
                    <div key={key} className={styles.argRow}>
                      <span className={styles.argKey}>{key}</span>
                      <span className={styles.argValue}>{value}</span>
                    </div>
                  ))}
                </div>
              </>
            )}
          </div>
        </Portal>
      )}
    </div>
  );
};
