import { useEffect, useRef } from "react";

import { useAppearance } from "../../../../hooks";
import { resolveLanguageForPath } from "./diffLanguage";
import {
  createDiffEditors,
  type DiffEditorsController,
  type DiffMode,
} from "./diffEditors";
import styles from "./GitDiffViewer.module.css";

export type DiffEditorsHostProps = {
  mode: DiffMode;
  left: string;
  right: string;
  collapseUnchanged: boolean;
  languagePath: string;
  leftLabel: string;
  rightLabel: string;
  onController: (controller: DiffEditorsController | null) => void;
};

/**
 * Owns the CodeMirror merge editors. They are rebuilt only when the diff
 * inputs (mode, documents, collapse) change; theme and language are
 * reconfigured in place through compartments.
 */
export function DiffEditorsHost({
  mode,
  left,
  right,
  collapseUnchanged,
  languagePath,
  leftLabel,
  rightLabel,
  onController,
}: DiffEditorsHostProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const controllerRef = useRef<DiffEditorsController | null>(null);
  const { appearance } = useAppearance();
  const isDark = appearance === "dark";
  const latest = useRef({ isDark, leftLabel, rightLabel, onController });
  latest.current = { isDark, leftLabel, rightLabel, onController };
  const languageRef = useRef<{
    path: string;
    support: Awaited<ReturnType<typeof resolveLanguageForPath>>;
  } | null>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const cachedLanguage =
      languageRef.current?.path === languagePath
        ? languageRef.current.support
        : null;
    const controller = createDiffEditors({
      parent: host,
      mode,
      left,
      right,
      collapseUnchanged,
      isDark: latest.current.isDark,
      language: cachedLanguage,
      leftLabel: latest.current.leftLabel,
      rightLabel: latest.current.rightLabel,
    });
    controllerRef.current = controller;
    latest.current.onController(controller);
    return () => {
      controllerRef.current = null;
      latest.current.onController(null);
      controller.destroy();
    };
  }, [collapseUnchanged, left, mode, right, languagePath]);

  useEffect(() => {
    controllerRef.current?.setDark(isDark);
  }, [isDark]);

  useEffect(() => {
    let stale = false;
    if (languageRef.current?.path === languagePath) return;
    void resolveLanguageForPath(languagePath).then((support) => {
      if (stale) return;
      languageRef.current = { path: languagePath, support };
      controllerRef.current?.setLanguage(support);
    });
    return () => {
      stale = true;
    };
  }, [languagePath]);

  return (
    <div ref={hostRef} className={styles.host} data-testid="git-diff-editors" />
  );
}
