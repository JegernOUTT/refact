import { useCallback, useContext, useMemo } from "react";
import { ChatThreadContext } from "../features/Chat/Thread/ChatThreadContext";
import { selectThreadWorktreeById } from "../features/Chat/Thread/selectors";
import { setError } from "../features/Errors/errorsSlice";
import { resolveWorkspaceFilePath } from "../utils/resolveWorkspaceFilePath";

const EMPTY_ROOTS: string[] = [];

import { selectCapabilities } from "../features/Config/configSlice";
import { openFileInFilesPanel } from "../features/Workspace/FilesPanel/filesPanelSlice";
import { useAppDispatch } from "./useAppDispatch";
import { useAppSelector } from "./useAppSelector";
import { useEventsBusForIDE } from "./useEventBusForIDE";

export type OpenFileInAppTarget = {
  path: string;
  line?: number;
  resolved?: boolean;
};

export function useOpenFileInApp() {
  const dispatch = useAppDispatch();
  const capabilities = useAppSelector(selectCapabilities);
  const sourceChatId = useContext(ChatThreadContext);
  const worktree = useAppSelector((state) =>
    sourceChatId ? selectThreadWorktreeById(state, sourceChatId) : null,
  );
  const workspaceRoots = useAppSelector(
    (state) => state.current_project.workspaceRoots ?? EMPTY_ROOTS,
  );
  const { openFile: openFileInIde, queryPathThenOpenFile } =
    useEventsBusForIDE();

  const canOpenInApp = capabilities.openFileInApp;
  const canOpenInIde = capabilities.openFileInIde;
  const canOpen = canOpenInApp || canOpenInIde;

  const openFile = useCallback(
    ({ path, line, resolved }: OpenFileInAppTarget) => {
      if (canOpenInApp) {
        const result = resolveWorkspaceFilePath(
          path,
          workspaceRoots,
          worktree?.root,
        );
        if ("error" in result) {
          dispatch(setError(result.error));
          return;
        }
        dispatch(openFileInFilesPanel({ path: result.path, line }));
        return;
      }
      if (!canOpenInIde) return;
      if (resolved) {
        openFileInIde({ file_path: path, line });
        return;
      }
      void queryPathThenOpenFile({ file_path: path, line });
    },
    [
      canOpenInApp,
      canOpenInIde,
      dispatch,
      openFileInIde,
      queryPathThenOpenFile,
      workspaceRoots,
      worktree?.root,
    ],
  );

  return useMemo(() => ({ canOpen, openFile }), [canOpen, openFile]);
}
