import {
  LanguageDescription,
  type LanguageSupport,
} from "@codemirror/language";
import { languages } from "@codemirror/language-data";

import { pathBasename } from "./gitDiffDescriptor";

/** Resolve CodeMirror language support from a repository path's file name. */
export async function resolveLanguageForPath(
  path: string,
): Promise<LanguageSupport | null> {
  const description = LanguageDescription.matchFilename(
    languages,
    pathBasename(path),
  );
  if (!description) return null;
  try {
    return await description.load();
  } catch {
    return null;
  }
}
