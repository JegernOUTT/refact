import { ShikiCodeBlock } from "../../Markdown";
import { TranscriptClamp } from "../TranscriptItem";
import styles from "./OpenAIResponsesTool.module.css";

function str(record: Record<string, unknown>, ...keys: string[]) {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string" && value.length > 0) return value;
  }
  return undefined;
}

function isSafeHttpUrl(url: string): boolean {
  try {
    const parsed = new URL(url);
    return parsed.protocol === "http:" || parsed.protocol === "https:";
  } catch {
    return false;
  }
}

export function WebResults({
  results,
}: {
  results: readonly Record<string, unknown>[];
}) {
  if (results.length === 0) return null;
  return (
    <TranscriptClamp>
      <ul className={styles.resultList}>
        {results.map((result, index) => {
          const title = str(result, "title") ?? "(no title)";
          const url = str(result, "url");
          const safeUrl = url && isSafeHttpUrl(url) ? url : undefined;
          const snippet = str(result, "snippet", "description");
          return (
            <li className={styles.resultItem} key={index}>
              {safeUrl ? (
                <a href={safeUrl} rel="noopener noreferrer" target="_blank">
                  {title}
                </a>
              ) : (
                <span>{title}</span>
              )}
              {safeUrl ? (
                <span className={styles.inlineCode}>{safeUrl}</span>
              ) : null}
              {snippet ? <span className={styles.muted}>{snippet}</span> : null}
            </li>
          );
        })}
      </ul>
    </TranscriptClamp>
  );
}

export function FileResults({
  results,
}: {
  results: readonly Record<string, unknown>[];
}) {
  if (results.length === 0) return null;
  return (
    <TranscriptClamp>
      <ul className={styles.resultList}>
        {results.map((result, index) => {
          const filename = str(result, "filename", "file_name") ?? "(file)";
          const text = str(result, "text", "content");
          return (
            <li className={styles.resultItem} key={index}>
              <span className={styles.inlineCode}>{filename}</span>
              {text ? (
                <ShikiCodeBlock showLineNumbers={false}>{text}</ShikiCodeBlock>
              ) : null}
            </li>
          );
        })}
      </ul>
    </TranscriptClamp>
  );
}
