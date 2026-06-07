import React, { useEffect, useState } from "react";
import { RefactIcon } from "../../images";
import { LogoAnimation } from "../../components/LogoAnimation";
import styles from "./SplashScreen.module.css";

type SplashScreenProps = {
  message?: string;
};

export const SplashScreen: React.FC<SplashScreenProps> = ({
  message = "Starting local Refact engine…",
}) => {
  const [reducedMotion, setReducedMotion] = useState(false);

  useEffect(() => {
    try {
      if (typeof window === "undefined") return;
      if (typeof window.matchMedia !== "function") return;

      const media = window.matchMedia("(prefers-reduced-motion: reduce)");
      setReducedMotion(media.matches);

      const onChange = () => setReducedMotion(media.matches);
      if (typeof media.addEventListener === "function") {
        media.addEventListener("change", onChange);
        return () => {
          try {
            media.removeEventListener("change", onChange);
          } catch {
            // Ignore media-query cleanup failures.
          }
        };
      }
      if (typeof media.addListener === "function") {
        media.addListener(onChange);
        return () => {
          try {
            media.removeListener(onChange);
          } catch {
            // Ignore legacy media-query cleanup failures.
          }
        };
      }
    } catch {
      return;
    }
  }, []);

  return (
    <div
      className={styles.root}
      data-testid="startup-splash"
      role="status"
      aria-live="polite"
    >
      <div className={styles.card}>
        <div className={styles.logoWrap}>
          <RefactIcon className={styles.logo} aria-hidden="true" />
        </div>

        <div className={styles.copy}>
          <h1 className={styles.title}>Refact</h1>
          <p className={styles.caption}>{message}</p>
        </div>

        {!reducedMotion && (
          <div className={styles.animation} aria-hidden="true">
            <LogoAnimation isWaiting={false} isStreaming size="8" />
          </div>
        )}
      </div>
    </div>
  );
};

SplashScreen.displayName = "SplashScreen";
