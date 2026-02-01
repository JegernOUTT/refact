import { useEffect, useState, useCallback } from "react";
import {
  createHighlighter,
  type Highlighter,
  type BundledLanguage,
  type BundledTheme,
} from "shiki";

let highlighterInstance: Highlighter | null = null;
let highlighterPromise: Promise<Highlighter> | null = null;

const INITIAL_LANGUAGES: BundledLanguage[] = [
  "javascript",
  "typescript",
  "python",
  "rust",
  "go",
  "java",
  "c",
  "cpp",
  "csharp",
  "html",
  "css",
  "json",
  "yaml",
  "markdown",
  "bash",
  "shell",
  "sql",
  "dockerfile",
  "tsx",
  "jsx",
];

const LIGHT_THEME: BundledTheme = "github-light";
const DARK_THEME: BundledTheme = "github-dark";

async function getHighlighter(): Promise<Highlighter> {
  if (highlighterInstance) {
    return highlighterInstance;
  }

  if (highlighterPromise) {
    return highlighterPromise;
  }

  highlighterPromise = createHighlighter({
    themes: [LIGHT_THEME, DARK_THEME],
    langs: INITIAL_LANGUAGES,
  })
    .then((h: Highlighter) => {
      highlighterInstance = h;
      return h;
    })
    .catch((err: unknown) => {
      highlighterPromise = null;
      throw err;
    });

  return highlighterPromise;
}

const LANGUAGE_ALIASES: Record<string, string> = {
  js: "javascript",
  ts: "typescript",
  py: "python",
  rb: "ruby",
  sh: "bash",
  zsh: "bash",
  yml: "yaml",
  md: "markdown",
  rs: "rust",
  cs: "csharp",
  "c++": "cpp",
  "c#": "csharp",
  plaintext: "plaintext",
  plain: "plaintext",
  text: "plaintext",
};

function normalizeLanguage(lang: string): string {
  const lower = lang.toLowerCase();
  const alias = LANGUAGE_ALIASES[lower] as string | undefined;
  return alias ?? lower;
}

export type ShikiHighlightResult = {
  html: string;
  language: string;
};

export function useShiki() {
  const [highlighter, setHighlighter] = useState<Highlighter | null>(
    highlighterInstance,
  );
  const [isLoading, setIsLoading] = useState(!highlighterInstance);
  const [error, setError] = useState<Error | null>(null);

  useEffect(() => {
    if (highlighterInstance) {
      setHighlighter(highlighterInstance);
      setIsLoading(false);
      return;
    }

    let mounted = true;

    void getHighlighter()
      .then((h: Highlighter) => {
        if (mounted) {
          setHighlighter(h);
          setIsLoading(false);
        }
      })
      .catch((err: unknown) => {
        if (mounted) {
          setError(err instanceof Error ? err : new Error(String(err)));
          setIsLoading(false);
        }
      });

    return () => {
      mounted = false;
    };
  }, []);

  const highlight = useCallback(
    async (
      code: string,
      language: string,
      isDark: boolean,
    ): Promise<ShikiHighlightResult> => {
      const h = highlighter ?? (await getHighlighter());
      const normalizedLang = normalizeLanguage(language);
      const theme = isDark ? DARK_THEME : LIGHT_THEME;

      const loadedLangs = (
        h as unknown as { getLoadedLanguages(): BundledLanguage[] }
      ).getLoadedLanguages();
      let finalLang = normalizedLang;

      if (!loadedLangs.includes(normalizedLang as BundledLanguage)) {
        try {
          await (
            h as unknown as {
              loadLanguage(lang: BundledLanguage): Promise<void>;
            }
          ).loadLanguage(normalizedLang as BundledLanguage);
        } catch {
          finalLang = "plaintext";
        }
      }

      const html = (
        h as unknown as {
          codeToHtml(
            code: string,
            options: { lang: string; theme: BundledTheme },
          ): string;
        }
      ).codeToHtml(code, {
        lang: finalLang,
        theme,
      });

      return { html, language: finalLang };
    },
    [highlighter],
  );

  const highlightSync = useCallback(
    (
      code: string,
      language: string,
      isDark: boolean,
    ): ShikiHighlightResult | null => {
      if (!highlighter) return null;

      const normalizedLang = normalizeLanguage(language);
      const theme = isDark ? DARK_THEME : LIGHT_THEME;
      const loadedLangs = (
        highlighter as unknown as { getLoadedLanguages(): BundledLanguage[] }
      ).getLoadedLanguages();

      const finalLang = loadedLangs.includes(normalizedLang as BundledLanguage)
        ? normalizedLang
        : "plaintext";

      const html = (
        highlighter as unknown as {
          codeToHtml(
            code: string,
            options: { lang: string; theme: BundledTheme },
          ): string;
        }
      ).codeToHtml(code, {
        lang: finalLang,
        theme,
      });

      return { html, language: finalLang };
    },
    [highlighter],
  );

  return {
    highlighter,
    isLoading,
    error,
    highlight,
    highlightSync,
    isReady: !!highlighter && !isLoading,
  };
}

export { LIGHT_THEME, DARK_THEME };
