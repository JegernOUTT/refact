import type { LanguageSupport } from "@codemirror/language";
import { languages } from "@codemirror/language-data";

export const SUPPORTED_LANGUAGE_ALIASES = new Map<string, string | null>([
  ["javascriptreact", "jsx"],
  ["typescriptreact", "tsx"],
  ["shellscript", "shell"],
  ["plaintext", null],
  ["csharp", "C#"],
  ["cpp", "C++"],
  ["makefile", "Makefile"],
  ["dockerfile", "Dockerfile"],
]);

export async function resolveLanguageSupport(
  language: string | null,
): Promise<LanguageSupport | null> {
  if (language === null) return null;

  const normalized = language.toLowerCase();
  const explicitAlias = SUPPORTED_LANGUAGE_ALIASES.get(normalized);
  if (explicitAlias === null) return null;
  const sought = (explicitAlias ?? language).toLowerCase();
  const description = languages.find(
    ({ name, alias }) =>
      name.toLowerCase() === sought ||
      alias.some((candidate) => candidate.toLowerCase() === sought),
  );

  if (!description) return null;

  try {
    return await description.load();
  } catch {
    return null;
  }
}
