import { useEffect, useRef } from "react";
import { push } from "../features/Pages/pagesSlice";
import { useAppDispatch } from "./useAppDispatch";
import { useConfig } from "./useConfig";

export function useBuddyWorkDeepLink(ready: boolean) {
  const dispatch = useAppDispatch();
  const config = useConfig();
  const handled = useRef(false);
  useEffect(() => {
    if (
      handled.current ||
      !ready ||
      config.host !== "web" ||
      !config.engineServed
    )
      return;
    const params = new URLSearchParams(window.location.search);
    const workId = params.get("buddy_work");
    const projectId = params.get("buddy_project");
    if (params.has("chat")) {
      handled.current = true;
      return;
    }
    if (!workId || !projectId) return;
    handled.current = true;
    if (workId.length > 512 || projectId.length > 512) return;
    dispatch(push({ name: "buddy", workId, workProjectId: projectId }));
  }, [config.engineServed, config.host, dispatch, ready]);
}
