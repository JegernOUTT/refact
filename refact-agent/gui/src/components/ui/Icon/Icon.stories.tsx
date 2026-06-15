import type { Meta, StoryObj } from "@storybook/react";
import { AlertTriangle, CheckCircle, Info, Sparkles } from "lucide-react";
import { Icon } from "./Icon";
import styles from "./Icon.stories.module.css";

const sizes = ["sm", "md", "lg"] as const;
const tones = [
  "default",
  "muted",
  "faint",
  "accent",
  "success",
  "warning",
  "danger",
] as const;

function IconMatrix() {
  return (
    <div className={styles.storyShell}>
      {(["light", "dark"] as const).map((appearance) => (
        <section
          className={styles.panel}
          data-appearance={appearance}
          key={appearance}
        >
          <div className={styles.header}>
            <h2 className={styles.title}>{appearance}</h2>
            <p className={styles.description}>
              Lucide strokes inherit currentColor from the Icon tone class.
            </p>
          </div>
          <div className={styles.grid}>
            <span className={styles.axisLabel}>Tone</span>
            {sizes.map((size) => (
              <span className={styles.axisLabel} key={size}>
                {size}
              </span>
            ))}
            {tones.map((tone) => (
              <div className={styles.rowLabel} key={tone}>
                <Icon icon={Sparkles} size="sm" tone={tone} />
                <span>{tone}</span>
              </div>
            ))}
            {tones.flatMap((tone) =>
              sizes.map((size) => (
                <div className={styles.cell} key={`${tone}-${size}`}>
                  <Icon
                    aria-label={`${tone} ${size} icon`}
                    icon={
                      tone === "danger"
                        ? AlertTriangle
                        : tone === "success"
                          ? CheckCircle
                          : Info
                    }
                    size={size}
                    tone={tone}
                  />
                </div>
              )),
            )}
          </div>
        </section>
      ))}
    </div>
  );
}

const meta = {
  title: "UI/Icon",
  component: Icon,
  parameters: {
    layout: "fullscreen",
  },
} satisfies Meta<typeof Icon>;

export default meta;

type Story = StoryObj<typeof meta>;

export const SizesAndTones: Story = {
  args: {
    icon: Info,
  },
  render: () => <IconMatrix />,
};
