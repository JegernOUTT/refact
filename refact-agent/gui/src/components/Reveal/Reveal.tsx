import React, { useCallback, useEffect, useState } from "react";
import { useCollapsibleStore } from "../ChatContent/useStoredOpen";
import styles from "./reveal.module.css";
import classNames from "classnames";

export type RevealProps = {
  children: React.ReactNode;
  defaultOpen: boolean;
  isRevealingCode?: boolean;
  onClose?: () => void;
  storeKey?: string;
};

const RevealButton: React.FC<{
  onClick: () => void;
  isInline: boolean;
  children: React.ReactNode;
}> = ({ onClick, isInline, children }) => (
  <button
    className={classNames(styles.reveal_button, {
      [styles.reveal_button_inline]: isInline,
    })}
    onClick={onClick}
    type="button"
  >
    {children}
  </button>
);

const RevealText: React.FC<{
  isRevealingCode: boolean;
  text: string;
}> = ({ isRevealingCode, text }) => (
  <div className={styles.reveal_text_wrap}>
    {isRevealingCode ? text : <div className={styles.reveal_text}>{text}</div>}
  </div>
);

export const Reveal: React.FC<RevealProps> = ({
  children,
  defaultOpen,
  isRevealingCode = false,
  onClose,
  storeKey,
}) => {
  const store = useCollapsibleStore();
  const [open, setOpen] = useState(() => {
    if (storeKey && store) {
      const stored = store.get(storeKey);
      if (stored !== undefined) return stored;
    }
    return defaultOpen;
  });

  useEffect(() => {
    if (storeKey && store) store.set(storeKey, open);
  }, [storeKey, store, open]);

  const handleClick = useCallback(() => {
    if (defaultOpen) return;
    setOpen((v) => !v);
  }, [defaultOpen]);

  const handleClose = useCallback(() => {
    if (defaultOpen) return;
    setOpen(false);
    onClose && onClose();
  }, [defaultOpen, onClose]);

  if (open) {
    return (
      <div className={styles.reveal_open}>
        {children}
        <RevealButton onClick={handleClose} isInline={!isRevealingCode}>
          {!defaultOpen && (
            <div
              className={classNames(
                styles.reveal_hidden,
                styles.reveal_hidden_exposed,
              )}
            >
              <RevealText
                isRevealingCode={isRevealingCode}
                text="Hide details"
              />
            </div>
          )}
        </RevealButton>
      </div>
    );
  }

  return (
    <RevealButton onClick={handleClick} isInline={!isRevealingCode}>
      <div className={styles.reveal_closed}>
        <div
          className={classNames({
            [styles.reveal_hidden]: !open,
          })}
        >
          {children}
        </div>
        {!defaultOpen && (
          <div
            className={classNames({
              [styles.reveal_button_box]: open,
            })}
          >
            <RevealText
              isRevealingCode={isRevealingCode}
              text="Click for more"
            />
          </div>
        )}
      </div>
    </RevealButton>
  );
};
