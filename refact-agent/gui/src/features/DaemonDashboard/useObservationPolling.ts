import { useEffect, useState } from "react";

export function useObservationPolling() {
  const [active, setActive] = useState(document.visibilityState !== "hidden");
  useEffect(() => {
    const update = () =>
      setActive(document.visibilityState !== "hidden" && document.hasFocus());
    const blur = () => setActive(false);
    document.addEventListener("visibilitychange", update);
    window.addEventListener("focus", update);
    window.addEventListener("blur", blur);
    return () => {
      document.removeEventListener("visibilitychange", update);
      window.removeEventListener("focus", update);
      window.removeEventListener("blur", blur);
    };
  }, []);
  return {
    pollingInterval: active ? 5_000 : 0,
    skipPollingIfUnfocused: true,
    refetchOnFocus: true,
  };
}
