import React, { useEffect, useState } from "react";
import { Flex, Heading, Text } from "@radix-ui/themes";
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
    if (typeof window === "undefined") return;

    const media = window.matchMedia("(prefers-reduced-motion: reduce)");
    setReducedMotion(media.matches);

    const onChange = () => setReducedMotion(media.matches);
    media.addEventListener("change", onChange);
    return () => media.removeEventListener("change", onChange);
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

        <Flex direction="column" align="center" gap="2">
          <Heading as="h1" size="7" className={styles.title}>
            Refact
          </Heading>
          <Text size="2" color="gray" className={styles.caption}>
            {message}
          </Text>
        </Flex>

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
