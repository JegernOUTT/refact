import { useAppSelector } from "../../hooks";
import { useIsDocumentVisible } from "../../hooks/useIsDocumentVisible";
import {
  browserApi,
  projectBrowserErrorMessage,
} from "../../services/refact/browser";
import { selectProjectBrowserStatus } from "./browserViews";

export { selectProjectBrowserStatus } from "./browserViews";

export const PROJECT_BROWSER_STATUS_POLL_MS = 2000;

export function useProjectBrowserStatus() {
  const visible = useIsDocumentVisible();
  const query = browserApi.useProjectBrowserStatusQuery(undefined, {
    pollingInterval: visible ? PROJECT_BROWSER_STATUS_POLL_MS : 0,
  });
  const [start, startState] = browserApi.useProjectBrowserStartMutation();
  const status = useAppSelector(selectProjectBrowserStatus);

  return {
    status,
    loading: query.isLoading,
    error: query.error ? projectBrowserErrorMessage(query.error) : undefined,
    refetch: query.refetch,
    start: () => start(undefined),
    starting: startState.isLoading,
    startError: startState.error
      ? projectBrowserErrorMessage(startState.error)
      : undefined,
  };
}
