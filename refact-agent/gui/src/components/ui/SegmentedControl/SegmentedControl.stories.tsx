import type { Meta, StoryObj } from "@storybook/react";
import { useState } from "react";

import { SegmentedControl } from "./SegmentedControl";
import storyStyles from "../Control.stories.module.css";

const meta = {
  title: "UI/SegmentedControl",
  component: SegmentedControl,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof SegmentedControl>;

export default meta;
type Story = StoryObj<typeof meta>;

const densityOptions = [
  { value: "compact", label: "Compact" },
  { value: "regular", label: "Regular" },
  { value: "roomy", label: "Roomy" },
];

function SegmentedDemo({ reducedMotion = false }: { reducedMotion?: boolean }) {
  const [value, setValue] = useState("regular");

  return (
    <div className={reducedMotion ? storyStyles.reducedMotion : undefined}>
      <div className={storyStyles.storyShell}>
        <section className={storyStyles.panel}>
          <h3 className={storyStyles.title}>SegmentedControl</h3>
          <p className={storyStyles.description}>
            Compact selector with a transform-only sliding indicator.
          </p>
          <SegmentedControl
            options={densityOptions}
            value={value}
            onValueChange={setValue}
          />
          <SegmentedControl
            size="sm"
            options={[
              ...densityOptions,
              { value: "disabled", label: "Disabled", disabled: true },
            ]}
            value="compact"
            onValueChange={() => undefined}
          />
        </section>
        <section
          className={`${storyStyles.panel} ${storyStyles.narrowPanel}`}
          data-appearance="light"
        >
          <p className={storyStyles.description}>Light + narrow container.</p>
          <SegmentedControl
            options={densityOptions}
            value="compact"
            onValueChange={() => undefined}
          />
        </section>
      </div>
    </div>
  );
}

export const States: Story = {
  args: {
    options: densityOptions,
    value: "regular",
    onValueChange: () => undefined,
  },
  render: () => <SegmentedDemo />,
};

export const ReducedMotion: Story = {
  args: {
    options: densityOptions,
    value: "regular",
    onValueChange: () => undefined,
  },
  render: () => <SegmentedDemo reducedMotion />,
};
