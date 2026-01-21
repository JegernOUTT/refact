import { useEffect } from "react";
import { useAppDispatch } from "./useAppDispatch";
import { setBrowserOnline } from "../features/Connection";

export function useBrowserOnlineStatus() {
  const dispatch = useAppDispatch();

  useEffect(() => {
    const handleOnline = () => {
      dispatch(setBrowserOnline(true));
    };

    const handleOffline = () => {
      dispatch(setBrowserOnline(false));
    };

    window.addEventListener("online", handleOnline);
    window.addEventListener("offline", handleOffline);

    dispatch(setBrowserOnline(navigator.onLine));

    return () => {
      window.removeEventListener("online", handleOnline);
      window.removeEventListener("offline", handleOffline);
    };
  }, [dispatch]);
}
