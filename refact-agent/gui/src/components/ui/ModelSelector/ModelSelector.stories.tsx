import React from "react";
import type { Meta, StoryObj } from "@storybook/react";
import { Theme } from "@radix-ui/themes";

import { ModelSelector } from "./ModelSelector";
import type { ModelOption } from "./ModelSelector";
import styles from "./ModelSelector.stories.module.css";

const groups = [
  { id: "openai", label: "OpenAI" },
  { id: "anthropic", label: "Anthropic" },
  { id: "local", label: "Local" },
];

const capabilities = (...items: string[]) => (
  <span className={styles.capabilities}>
    {items.map((item) => (
      <span key={item} className={styles.capabilityPill}>
        {item}
      </span>
    ))}
  </span>
);

const longModelName =
  "OpenAI GPT 5.5 Ultra Long Model Name For Truncation With Task Agent Badge";

const manyModels: ModelOption[] = Array.from({ length: 18 }, (_, index) => ({
  value: `openai/bulk-${index}`,
  displayName: `Bulk model ${index + 1} with a deliberately descriptive name`,
  group: index % 2 === 0 ? "openai" : "anthropic",
  pricing: { prompt: "$1.00", output: "$4.00" },
  contextWindow: "128K ctx",
  badges: index % 3 === 0 ? ["task-agent", "reasoning"] : ["light"],
  capabilities: capabilities("tools", "vision"),
}));

const models: ModelOption[] = [
  {
    value: "openai/gpt-5.5",
    displayName: longModelName,
    group: "openai",
    pricing: { prompt: "$1.25", output: "$10.00" },
    contextWindow: "400K ctx",
    badges: ["default", "reasoning", "chat2"],
    capabilities: capabilities("tools", "vision", "agent"),
  },
  {
    value: "openai/gpt-5.5-mini",
    displayName: "GPT 5.5 Mini",
    group: "openai",
    pricing: { prompt: "$0.15", output: "$0.60" },
    contextWindow: "128K ctx",
    badges: ["light"],
    capabilities: capabilities("tools"),
  },
  {
    value: "anthropic/claude-sonnet-4.5",
    displayName: "Claude Sonnet 4.5",
    group: "anthropic",
    pricing: { prompt: "$3.00", output: "$15.00" },
    contextWindow: "200K ctx",
    badges: ["task-agent", "reasoning"],
    capabilities: capabilities("tools", "thinking"),
  },
  {
    value: "anthropic/claude-haiku-4.5",
    displayName: "Claude Haiku 4.5",
    group: "anthropic",
    pricing: { prompt: "$0.80", output: "$4.00" },
    contextWindow: "200K ctx",
    badges: ["buddy"],
    capabilities: capabilities("fast", "tools"),
  },
  {
    value: "local/qwen3-coder",
    displayName: "Qwen3 Coder Local",
    group: "local",
    contextWindow: "64K ctx",
    badges: ["buddy", "light"],
    capabilities: capabilities("local", "code"),
  },
  {
    value: "local/disabled-experiment",
    displayName: "Disabled Experiment",
    group: "local",
    disabled: true,
    contextWindow: "32K ctx",
    badges: ["reasoning", "task-agent", "chat2", "default", "light", "buddy"],
    capabilities: capabilities("offline"),
  },
];

const meta = {
  title: "UI/ModelSelector",
  component: ModelSelector,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof ModelSelector>;

export default meta;
type Story = StoryObj<typeof meta>;

const baseArgs = {
  models,
  value: models[0]?.value ?? null,
  onSelect: () => undefined,
};

function StatefulSelector(
  props: Omit<React.ComponentProps<typeof ModelSelector>, "value" | "onSelect">,
) {
  const [value, setValue] = React.useState<string | null>(
    models[0]?.value ?? null,
  );

  return (
    <ModelSelector
      {...props}
      value={value}
      onSelect={(nextValue) => setValue(nextValue || null)}
    />
  );
}

export const PopoverGrouped: Story = {
  args: baseArgs,
  render: () => (
    <div className={styles.storyShell}>
      <section className={styles.panel}>
        <h2 className={styles.title}>Grouped popover</h2>
        <p className={styles.description}>
          Search, grouped rows, selected highlight, pricing, context and
          capability content.
        </p>
        <StatefulSelector
          groups={groups}
          models={models}
          onAddNewModel={() => undefined}
        />
      </section>
    </div>
  ),
};

export const InlineWithUnset: Story = {
  args: baseArgs,
  render: () => (
    <div className={styles.storyShell}>
      <section className={styles.panel}>
        <h2 className={styles.title}>Inline settings list</h2>
        <p className={styles.description}>
          Inline variant with unset and add-new actions for settings surfaces.
        </p>
        <StatefulSelector
          allowUnset
          groups={groups}
          models={models}
          variant="inline"
          onAddNewModel={() => undefined}
        />
      </section>
    </div>
  ),
};

export const CustomUnsetLabel: Story = {
  args: baseArgs,
  render: () => (
    <div className={styles.storyShell}>
      <section className={styles.panel}>
        <h2 className={styles.title}>Custom unset label</h2>
        <p className={styles.description}>
          Callers can rename the empty model row for settings forms.
        </p>
        <StatefulSelector
          allowUnset
          groups={groups}
          models={models}
          unsetLabel="None"
        />
      </section>
    </div>
  ),
};

export const DisabledRowsAndAllBadges: Story = {
  args: baseArgs,
  render: () => (
    <div className={styles.storyShell}>
      <section className={styles.panel}>
        <h2 className={styles.title}>All badge combinations</h2>
        <StatefulSelector groups={groups} models={models} variant="inline" />
      </section>
    </div>
  ),
};

export const PanelLessSingleScroll: Story = {
  args: baseArgs,
  render: () => (
    <div className={styles.storyShell}>
      <section className={styles.panel}>
        <h2 className={styles.title}>Panel-less single-scroll list</h2>
        <p className={styles.description}>
          Long names truncate, badges stay grouped, selected state is tint plus
          check, and only the model list scrolls.
        </p>
        <StatefulSelector
          groups={groups}
          models={[...models, ...manyModels]}
          onAddNewModel={() => undefined}
          variant="inline"
        />
      </section>
    </div>
  ),
};

export const NarrowPopoverSheet: Story = {
  args: baseArgs,
  render: () => (
    <div className={styles.storyShell}>
      <section className={`${styles.panel} ${styles.narrowPanel}`}>
        <h2 className={styles.title}>Narrow popover</h2>
        <p className={styles.description}>
          The kit Popover can become a Sheet on narrow screens.
        </p>
        <StatefulSelector
          allowUnset
          groups={groups}
          models={models}
          onAddNewModel={() => undefined}
        />
      </section>
    </div>
  ),
};

export const LightAndDark: Story = {
  args: baseArgs,
  render: () => (
    <div className={styles.storyShell}>
      <Theme appearance="light" data-appearance="light">
        <section className={styles.panel}>
          <h2 className={styles.title}>Light</h2>
          <StatefulSelector groups={groups} models={models} />
        </section>
      </Theme>
      <Theme appearance="dark" data-appearance="dark">
        <section className={styles.panel}>
          <h2 className={styles.title}>Dark</h2>
          <StatefulSelector groups={groups} models={models} />
        </section>
      </Theme>
    </div>
  ),
};
