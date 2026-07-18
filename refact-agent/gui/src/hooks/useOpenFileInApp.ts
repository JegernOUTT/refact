import { useCallback, useMemo } from "react";

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
  const { openFile: openFileInIde, queryPathThenOpenFile } =
    useEventsBusForIDE();

  const canOpenInApp = capabilities.openFileInApp;
  const canOpenInIde = capabilities.openFileInIde;
  const canOpen = canOpenInApp || canOpenInIde;

  const openFile = useCallback(
    ({ path, line, resolved }: OpenFileInAppTarget) => {
      if (canOpenInApp) {
        dispatch(openFileInFilesPanel({ path, line }));
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
    ],
  );

  return useMemo(() => ({ canOpen, openFile }), [canOpen, openFile]);
}
