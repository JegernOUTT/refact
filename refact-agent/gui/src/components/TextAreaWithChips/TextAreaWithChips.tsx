import React, { useRef, useEffect, useMemo, useCallback } from "react";
import { TextArea, type TextAreaProps } from "../TextArea/TextArea";
import { AtCommandChip } from "../AtCommands";
import {
  parseLines,
  tokenToChipInfo,
  getAtCommandTokens,
} from "../../utils/atCommands";
import styles from "./TextAreaWithChips.module.css";

type TextAreaWithChipsProps = TextAreaProps & {
  host: string;
  onOpenFile?: (file: { file_path: string; line?: number }) => Promise<void>;
};

export const TextAreaWithChips = React.forwardRef<
  HTMLTextAreaElement,
  TextAreaWithChipsProps
>(({ host, onOpenFile, value, ...props }, ref) => {
  const overlayRef = useRef<HTMLDivElement>(null);

  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // eslint-disable-next-line @typescript-eslint/no-non-null-assertion
  React.useImperativeHandle(ref, () => textareaRef.current!, []);

  const textValue = typeof value === "string" ? value : String(value);

  const parsedLines = useMemo(() => parseLines(textValue), [textValue]);
  const allAtTokens = useMemo(
    () => getAtCommandTokens(parsedLines),
    [parsedLines],
  );

  const handleChipClick = useCallback(
    (type: string, arg?: string, lineRange?: { line1: number }) => {
      if (type === "file" && arg && onOpenFile) {
        void onOpenFile({ file_path: arg, line: lineRange?.line1 });
      } else if (type === "web" && arg) {
        window.open(
          arg.startsWith("http") ? arg : `https://${arg}`,
          "_blank",
          "noopener,noreferrer",
        );
      }
    },
    [onOpenFile],
  );

  useEffect(() => {
    const textarea = textareaRef.current;
    const overlay = overlayRef.current;
    if (!textarea || !overlay) return;

    const syncScroll = () => {
      overlay.scrollTop = textarea.scrollTop;
      overlay.scrollLeft = textarea.scrollLeft;
    };

    textarea.addEventListener("scroll", syncScroll);
    return () => textarea.removeEventListener("scroll", syncScroll);
  }, []);

  const renderOverlay = () => {
    return parsedLines.map((line, lineIdx) => {
      const elements = line.tokens.map((token, tokenIdx) => {
        if (token.kind === "text") {
          return (
            <span key={`${lineIdx}-${tokenIdx}`} className={styles.text}>
              {token.text}
            </span>
          );
        }

        const chip = tokenToChipInfo(token, host === "web", allAtTokens);

        return (
          <AtCommandChip
            key={`${lineIdx}-${tokenIdx}`}
            chip={chip}
            onClick={() =>
              handleChipClick(token.type, token.arg, token.lineRange)
            }
          />
        );
      });

      return (
        <div key={lineIdx} className={styles.line}>
          {elements.length > 0 ? elements : "\u200B"}
        </div>
      );
    });
  };

  return (
    <div className={styles.container}>
      <div ref={overlayRef} className={styles.overlay} aria-hidden="true">
        {renderOverlay()}
      </div>
      <TextArea {...props} value={value} ref={textareaRef} />
    </div>
  );
});

TextAreaWithChips.displayName = "TextAreaWithChips";
