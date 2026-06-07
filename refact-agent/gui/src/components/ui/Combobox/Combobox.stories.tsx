import type { Meta, StoryObj } from "@storybook/react";
import { useState } from "react";

import { Combobox } from "./Combobox";
import storyStyles from "../Control.stories.module.css";

const meta = {
  title: "UI/Combobox",
  component: Combobox,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Combobox>;

export default meta;
type Story = StoryObj<typeof meta>;

const items = [
  { value: "agent", label: "Agent mode" },
  { value: "explore", label: "Explore mode" },
  { value: "planner", label: "Planner mode" },
  { value: "review", label: "Review helper" },
  { value: "disabled", label: "Disabled item", disabled: true },
];

function ComboboxDemo({ reducedMotion = false }: { reducedMotion?: boolean }) {
  const [value, setValue] = useState("ag");

  return (
    <div className={reducedMotion ? storyStyles.reducedMotion : undefined}>
      <div className={storyStyles.storyShell}>
        <section className={storyStyles.panel}>
          <h3 className={storyStyles.title}>Combobox</h3>
          <p className={storyStyles.description}>
            Generic Ariakit combobox with token styling and clamped popover surface.
          </p>
          <Combobox
            items={items}
            maxHeight="240px"
            placeholder="Search modes"
            value={value}
            onSelect={(item) => setValue(item.value)}
            onValueChange={setValue}
          />
        </section>
        <section className={`${storyStyles.panel} ${storyStyles.narrowPanel}`} data-appearance="light">
          <p className={storyStyles.description}>Light + narrow container.</p>
          <Combobox items={items} placeholder="Filter" value="" onValueChange={() => undefined} />
        </section>
      </div>
    </div>
  );
}

export const States: Story = {
  args: {
    items,
    value: "",
    onValueChange: () => undefined,
  },
  render: () => <ComboboxDemo />,
};

export const ReducedMotion: Story = {
  args: {
    items,
    value: "",
    onValueChange: () => undefined,
  },
  render: () => <ComboboxDemo reducedMotion />,
};
