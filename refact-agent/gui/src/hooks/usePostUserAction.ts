import { useCallback } from "react";
import { usePostUserActionMutation } from "../services/refact/buddy";

export function usePostUserAction() {
  const [postUserAction] = usePostUserActionMutation();

  const postFileOpened = useCallback(
    (path: string) => {
      if (!path) return;
      void postUserAction({
        type: "file_opened",
        path,
        ts: new Date().toISOString(),
      })
        .unwrap()
        .catch(() => undefined);
    },
    [postUserAction],
  );

  const postSnippetSelected = useCallback(
    (path: string, line1: number, line2: number) => {
      if (!path) return;
      void postUserAction({
        type: "snippet_selected",
        path,
        lines: [line1, line2],
        ts: new Date().toISOString(),
      })
        .unwrap()
        .catch(() => undefined);
    },
    [postUserAction],
  );

  return { postFileOpened, postSnippetSelected };
}
