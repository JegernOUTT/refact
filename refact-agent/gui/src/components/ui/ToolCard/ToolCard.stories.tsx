import type { Meta, StoryObj } from "@storybook/react";
import { Theme } from "@radix-ui/themes";
import {
  CheckCircle2,
  Clock3,
  Copy,
  FileCode2,
  Loader2,
  PlayCircle,
  Terminal,
  XCircle,
} from "lucide-react";

import { Button } from "../Button";
import { ToolCard } from "./ToolCard";
import type { ToolCardStatus } from "./ToolCard";
import styles from "./ToolCard.stories.module.css";

const states: {
  status: ToolCardStatus;
  title: string;
  icon: typeof Terminal;
  defaultOpen?: boolean;
}[] = [
  { status: "idle", title: "Read file", icon: FileCode2 },
  { status: "running", title: "Run shell command", icon: PlayCircle },
  { status: "success", title: "Patch applied", icon: CheckCircle2 },
  { status: "error", title: "Tool failed", icon: XCircle },
  { status: "streaming", title: "Streaming logs", icon: Loader2 },
  {
    status: "idle",
    title: "Collapsed by default",
    icon: Clock3,
    defaultOpen: false,
  },
];

function ToolBody({ status }: { status: ToolCardStatus }) {
  return (
    <>
      <p className={styles.copy}>
        Presentational shell for {status} tool output. The body owns no vertical
        scroll and wide code or diff previews live in explicit horizontal scroll
        islands.
      </p>
      <div className="scrollX">
        <pre
          className={styles.codeBlock}
        >{`$ refact tool --status ${status}\nstdout: useful preview that can be wider than the card without creating page overflow`}</pre>
      </div>
    </>
  );
}

function StateGallery() {
  return (
    <div className={styles.gallery}>
      {states.map(({ status, title, icon, defaultOpen }) => (
        <ToolCard
          actions={
            <Button leftIcon={Copy} size="sm" variant="ghost">
              Copy
            </Button>
          }
          defaultOpen={defaultOpen}
          icon={icon}
          key={`${status}-${title}`}
          status={status}
          title={title}
        >
          <ToolBody status={status} />
        </ToolCard>
      ))}
    </div>
  );
}

function LightDarkGallery() {
  return (
    <div className={styles.storyShell}>
      <Theme appearance="light" data-appearance="light">
        <section className={styles.panel}>
          <h2 className={styles.title}>Light</h2>
          <StateGallery />
        </section>
      </Theme>
      <Theme appearance="dark" data-appearance="dark">
        <section className={styles.panel}>
          <h2 className={styles.title}>Dark</h2>
          <StateGallery />
        </section>
      </Theme>
    </div>
  );
}

const meta = {
  title: "UI/ToolCard",
  component: ToolCard,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof ToolCard>;

export default meta;

type Story = StoryObj<typeof meta>;

export const States: Story = {
  args: {
    title: "Read file",
    status: "idle",
  },
  render: () => (
    <div className={styles.storyShell}>
      <section className={styles.panel}>
        <h2 className={styles.title}>Tool states</h2>
        <StateGallery />
      </section>
    </div>
  ),
};

export const LightAndDark: Story = {
  args: {
    title: "Read file",
    status: "idle",
  },
  render: () => <LightDarkGallery />,
};
