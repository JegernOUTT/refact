import type { Meta, StoryObj } from "@storybook/react";
import { fn } from "@storybook/test";

import { DocumentationSettings } from ".";
import styles from "./DocumentationSettings.stories.module.css";

const meta: Meta<typeof DocumentationSettings> = {
  title: "Documentation settings",
  component: DocumentationSettings,
  args: {
    sources: [
      {
        url: "https://docs.rs/url/latest/url/index.html",
        pages: 20,
        maxDepth: 2,
        maxPages: 50,
      },
      {
        url: "https://en.cppreference.com/w/cpp/string",
        pages: 1,
        maxDepth: 2,
        maxPages: 50,
      },
    ],
    editDocumentation: fn(),
    addDocumentation: fn(),
    deleteDocumentation: fn(),
    refetchDocumentation: fn(),
  },
  decorators: [
    (Children) => (
      <div className={styles.frame}>
        <Children />
      </div>
    ),
  ],
} satisfies Meta<typeof DocumentationSettings>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Primary: Story = {
  args: {},
};
