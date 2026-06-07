import type { Meta, StoryObj } from "@storybook/react";
import { ChevronRight, Download, Plus, Trash2 } from "lucide-react";
import { Button, ButtonGroup, IconButton } from "./Button";
import styles from "./Button.stories.module.css";

const variants = ["ghost", "soft", "primary", "danger", "plain"] as const;
const sizes = ["sm", "md", "lg"] as const;

function ButtonMatrix() {
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
              Variants, sizes, disabled, loading, and icon-only states use only
              Refact tokens.
            </p>
          </div>

          <div className={styles.section}>
            <h3 className={styles.sectionTitle}>Variants × sizes</h3>
            <div className={styles.matrix}>
              <span className={styles.axisLabel}>Variant</span>
              {sizes.map((size) => (
                <span className={styles.axisLabel} key={size}>
                  {size}
                </span>
              ))}
              {variants.map((variant) => (
                <div className={styles.rowLabel} key={variant}>
                  {variant}
                </div>
              ))}
              {variants.flatMap((variant) =>
                sizes.map((size) => (
                  <div className={styles.cell} key={`${variant}-${size}`}>
                    <Button
                      leftIcon={Plus}
                      rightIcon={ChevronRight}
                      size={size}
                      variant={variant}
                    >
                      {variant}
                    </Button>
                  </div>
                )),
              )}
            </div>
          </div>

          <div className={styles.section}>
            <h3 className={styles.sectionTitle}>States</h3>
            <div className={styles.stateGrid}>
              {variants.map((variant) => (
                <ButtonGroup key={variant}>
                  <Button leftIcon={Download} variant={variant}>
                    Default
                  </Button>
                  <Button autoFocus rightIcon={ChevronRight} variant={variant}>
                    Focusable
                  </Button>
                  <Button disabled variant={variant}>
                    Disabled
                  </Button>
                  <Button loading variant={variant}>
                    Loading
                  </Button>
                  <IconButton
                    aria-label={`${variant} icon-only button`}
                    icon={variant === "danger" ? Trash2 : Plus}
                    variant={variant}
                  />
                </ButtonGroup>
              ))}
            </div>
          </div>

          <div className={styles.section}>
            <h3 className={styles.sectionTitle}>Icon-only sizes</h3>
            <div className={styles.iconGrid}>
              {variants.flatMap((variant) =>
                sizes.map((size) => (
                  <IconButton
                    aria-label={`${variant} ${size} icon button`}
                    icon={variant === "danger" ? Trash2 : Plus}
                    key={`${variant}-${size}`}
                    size={size}
                    variant={variant}
                  />
                )),
              )}
            </div>
          </div>

          <div className={styles.section}>
            <h3 className={styles.sectionTitle}>Narrow</h3>
            <div className={styles.narrowFrame}>
              <ButtonGroup>
                <Button leftIcon={Plus} variant="primary">
                  Create
                </Button>
                <IconButton aria-label="Delete item" icon={Trash2} variant="danger" />
              </ButtonGroup>
              <Button loading size="sm" variant="soft">
                Saving
              </Button>
            </div>
          </div>
        </section>
      ))}
    </div>
  );
}

const meta = {
  title: "UI/Button",
  component: Button,
  parameters: {
    layout: "fullscreen",
  },
} satisfies Meta<typeof Button>;

export default meta;

type Story = StoryObj<typeof meta>;

export const VariantsSizesStates: Story = {
  render: () => <ButtonMatrix />,
};
