import type { Meta, StoryObj } from "@storybook/react";
import styles from "./DesignSystemOverview.stories.module.css";

interface TokenItem {
  label: string;
  token: string;
  className: string;
}

interface ScaleItem {
  label: string;
  token: string;
  className: string;
}

const colorTokens: TokenItem[] = [
  { label: "Foreground", token: "--rf-color-fg", className: styles.bgColorFg },
  { label: "Muted", token: "--rf-color-muted", className: styles.bgColorMuted },
  { label: "Faint", token: "--rf-color-faint", className: styles.bgColorFaint },
  {
    label: "Accent",
    token: "--rf-color-accent",
    className: styles.bgColorAccent,
  },
  {
    label: "Accent soft",
    token: "--rf-color-accent-soft",
    className: styles.bgColorAccentSoft,
  },
  {
    label: "Success",
    token: "--rf-color-success",
    className: styles.bgColorSuccess,
  },
  {
    label: "Warning",
    token: "--rf-color-warning",
    className: styles.bgColorWarning,
  },
  {
    label: "Danger",
    token: "--rf-color-danger",
    className: styles.bgColorDanger,
  },
  { label: "Info", token: "--rf-color-info", className: styles.bgColorInfo },
];

const surfaceTokens: ScaleItem[] = [
  { label: "Surface 1", token: "--rf-surface-1", className: styles.surface1 },
  { label: "Surface 2", token: "--rf-surface-2", className: styles.surface2 },
  { label: "Surface 3", token: "--rf-surface-3", className: styles.surface3 },
  {
    label: "Overlay",
    token: "--rf-surface-overlay",
    className: styles.surfaceOverlay,
  },
];

const spacingTokens: ScaleItem[] = [
  { label: "Space 1", token: "--rf-space-1", className: styles.space1 },
  { label: "Space 2", token: "--rf-space-2", className: styles.space2 },
  { label: "Space 3", token: "--rf-space-3", className: styles.space3 },
  { label: "Space 4", token: "--rf-space-4", className: styles.space4 },
  { label: "Space 5", token: "--rf-space-5", className: styles.space5 },
  { label: "Space 6", token: "--rf-space-6", className: styles.space6 },
];

const radiusTokens: ScaleItem[] = [
  { label: "Chip", token: "--rf-radius-chip", className: styles.radiusChip },
  { label: "Control", token: "--rf-radius-ctl", className: styles.radiusCtl },
  { label: "Card", token: "--rf-radius-card", className: styles.radiusCard },
  { label: "Pill", token: "--rf-radius-pill", className: styles.radiusPill },
];

const typeTokens: ScaleItem[] = [
  { label: "Text 1", token: "--rf-text-1", className: styles.type1 },
  { label: "Text 2", token: "--rf-text-2", className: styles.type2 },
  { label: "Text 3", token: "--rf-text-3", className: styles.type3 },
  { label: "Text 4", token: "--rf-text-4", className: styles.type4 },
  { label: "Text 5", token: "--rf-text-5", className: styles.type5 },
];

function DesignSystemOverview() {
  return (
    <main className={styles.overview}>
      <header className={`${styles.hero} rf-enter-rise`}>
        <h1 className={styles.title}>Refact Design System</h1>
        <p className={styles.description}>
          Token gallery and motion workbench for the Storybook toolbar modes:
          appearance, container width, and reduced-motion visual aid.
        </p>
      </header>

      <section className={styles.section}>
        <h2 className={styles.sectionTitle}>Semantic colors</h2>
        <div className={styles.grid}>
          {colorTokens.map((item) => (
            <div className={styles.card} key={item.token}>
              <div className={`${styles.swatch} ${item.className}`} />
              <span className={styles.label}>{item.label}</span>
              <code className={styles.value}>{item.token}</code>
            </div>
          ))}
        </div>
      </section>

      <section className={styles.section}>
        <h2 className={styles.sectionTitle}>Surface ladder</h2>
        <div className={styles.surfaceLadder}>
          {surfaceTokens.map((item) => (
            <div
              className={`${styles.surface} ${item.className}`}
              key={item.token}
            >
              <div>
                <div className={styles.label}>{item.label}</div>
                <code className={styles.value}>{item.token}</code>
              </div>
            </div>
          ))}
        </div>
      </section>

      <section className={styles.section}>
        <h2 className={styles.sectionTitle}>Spacing scale</h2>
        <div className={styles.grid}>
          {spacingTokens.map((item) => (
            <div className={styles.card} key={item.token}>
              <div className={styles.spacingRow}>
                <span className={`${styles.spacingSample} ${item.className}`} />
                <span className={styles.label}>{item.label}</span>
              </div>
              <code className={styles.value}>{item.token}</code>
            </div>
          ))}
        </div>
      </section>

      <section className={styles.section}>
        <h2 className={styles.sectionTitle}>Radii</h2>
        <div className={styles.grid}>
          {radiusTokens.map((item) => (
            <div className={styles.card} key={item.token}>
              <div className={styles.radiusRow}>
                <span className={`${styles.radiusSample} ${item.className}`} />
                <span className={styles.label}>{item.label}</span>
              </div>
              <code className={styles.value}>{item.token}</code>
            </div>
          ))}
        </div>
      </section>

      <section className={styles.section}>
        <h2 className={styles.sectionTitle}>Type scale</h2>
        <div className={styles.typeScale}>
          {typeTokens.map((item) => (
            <div
              className={`${styles.card} ${item.className}`}
              key={item.token}
            >
              <span className={styles.label}>{item.label}</span>
              <code className={styles.value}>{item.token}</code>
            </div>
          ))}
        </div>
      </section>

      <section className={styles.section}>
        <h2 className={styles.sectionTitle}>Motion utilities</h2>
        <p className={styles.note}>
          The Storybook reduced-motion toggle applies a helper class for visual
          review. Real product behavior remains governed by the
          prefers-reduced-motion media query.
        </p>
        <div className={styles.motionGrid}>
          <div className={`${styles.motionDemo} rf-enter`}>
            <span className={styles.motionBox}>.rf-enter</span>
          </div>
          <div className={`${styles.motionDemo} rf-enter-rise`}>
            <span className={styles.motionBox}>.rf-enter-rise</span>
          </div>
          <div className={styles.motionDemo}>
            <div className={`${styles.staggerList} rf-stagger`}>
              <span className={`${styles.staggerDot} rf-enter`} />
              <span className={`${styles.staggerDot} rf-enter`} />
              <span className={`${styles.staggerDot} rf-enter`} />
              <span className={`${styles.staggerDot} rf-enter`} />
            </div>
          </div>
          <div className={styles.motionDemo}>
            <button
              className={`${styles.motionBox} rf-pressable`}
              type="button"
            >
              .rf-pressable
            </button>
          </div>
          <div className={styles.motionDemo}>
            <span className={`${styles.motionBox} rf-status-pulse`}>
              .rf-status-pulse
            </span>
          </div>
          <div className={styles.motionDemo}>
            <div
              className={`${styles.shimmerBox} rf-shimmer`}
              aria-label="rf-shimmer"
            />
          </div>
        </div>
      </section>
    </main>
  );
}

const meta = {
  title: "Design System/Overview",
  component: DesignSystemOverview,
  parameters: {
    layout: "fullscreen",
  },
} satisfies Meta<typeof DesignSystemOverview>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Overview: Story = {};
