import React from "react";
import classNames from "classnames";
import styles from "./LogoAnimation.module.css";

export type LogoAnimationProps = {
  isWaiting: boolean;
  isStreaming: boolean;
  size?: "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9";
};

const STREAM_VARIANTS = [
  "glitch",
  "orbit",
  "bars",
  "pinwheel",
  "comet",
  "typewriter",
  "helix",
  "flip",
  "swap",
  "rain",
  "radar",
  "stairs",
  "tetris",
  "rings",
  "metronome",
  "eight",
] as const;

type StreamVariant = (typeof STREAM_VARIANTS)[number];

const SPAN_COUNT: Record<Exclude<StreamVariant, "glitch">, number> = {
  orbit: 4,
  bars: 4,
  pinwheel: 4,
  comet: 3,
  typewriter: 3,
  helix: 3,
  flip: 2,
  swap: 2,
  rain: 3,
  radar: 2,
  stairs: 4,
  tetris: 3,
  rings: 2,
  metronome: 2,
  eight: 2,
};

const GLITCH_PAIRS = ["_[", "*&", "?;", "\\>", "=/", ",("];

function pickVariant(previous?: StreamVariant): StreamVariant {
  const pool = previous
    ? STREAM_VARIANTS.filter((v) => v !== previous)
    : STREAM_VARIANTS;
  return pool[Math.floor(Math.random() * pool.length)];
}

function variantContent(variant: StreamVariant): React.ReactNode {
  if (variant === "glitch") {
    return GLITCH_PAIRS.map((pair, index) => (
      <span
        key={index}
        className={styles.glitchLayer}
        style={{ animationDelay: `${index * 0.22}s` }}
      >
        {pair}
      </span>
    ));
  }
  return Array.from({ length: SPAN_COUNT[variant] }, (_, index) => (
    <span key={index} className={styles.el} />
  ));
}

export const LogoAnimation: React.FC<LogoAnimationProps> = ({
  isWaiting,
  isStreaming,
  size = "8",
}) => {
  const [variant, setVariant] = React.useState<StreamVariant>(() =>
    pickVariant(),
  );
  const wasStreamingRef = React.useRef(isStreaming);

  React.useEffect(() => {
    if (isStreaming && !wasStreamingRef.current) {
      // New streaming episode: pick a fresh animation so it never gets stale.
      setVariant((previous) => pickVariant(previous));
    }
    wasStreamingRef.current = isStreaming;
  }, [isStreaming]);

  if (!isStreaming && !isWaiting) return false;

  const style = { fontSize: `var(--font-size-${size})` };

  if (isStreaming) {
    return (
      <span
        className={classNames(styles.root, styles.stream, styles[variant])}
        style={style}
        data-testid="logo-animation"
        data-variant={variant}
      >
        {variantContent(variant)}
      </span>
    );
  }

  return (
    <span className={styles.waiting} style={style} data-testid="logo-animation">
      <span className={styles.dot} />
      <span className={styles.dot} />
    </span>
  );
};
