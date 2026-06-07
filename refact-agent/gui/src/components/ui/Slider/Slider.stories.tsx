import type { Meta, StoryObj } from "@storybook/react";
import { useState } from "react";

import { Slider } from "./Slider";
import storyStyles from "../Control.stories.module.css";

const meta = {
  title: "UI/Slider",
  component: Slider,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Slider>;

export default meta;
type Story = StoryObj<typeof meta>;

function SliderDemo({ reducedMotion = false }: { reducedMotion?: boolean }) {
  const [value, setValue] = useState([48]);

  return (
    <div className={reducedMotion ? storyStyles.reducedMotion : undefined}>
      <div className={storyStyles.storyShell}>
        <section className={storyStyles.panel}>
          <h3 className={storyStyles.title}>Slider</h3>
          <p className={storyStyles.description}>Token styled Radix Slider with hover and focus states.</p>
          <Slider
            label="Temperature"
            max={100}
            step={1}
            value={value}
            valueLabel={`${value[0]}%`}
            onValueChange={setValue}
          />
          <Slider defaultValue={[25, 75]} label="Range" max={100} step={5} />
          <Slider defaultValue={[32]} disabled label="Disabled" max={100} />
        </section>
        <section className={`${storyStyles.panel} ${storyStyles.narrowPanel}`} data-appearance="light">
          <p className={storyStyles.description}>Light + narrow container.</p>
          <Slider defaultValue={[64]} label="Light slider" max={100} />
        </section>
      </div>
    </div>
  );
}

export const States: Story = {
  render: () => <SliderDemo />,
};

export const ReducedMotion: Story = {
  render: () => <SliderDemo reducedMotion />,
};
