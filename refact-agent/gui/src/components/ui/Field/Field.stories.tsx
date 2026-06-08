import type { Meta, StoryObj } from "@storybook/react";
import { Bell, Bot, Database, KeyRound } from "lucide-react";
import { useMemo, useState } from "react";

import { Button } from "../Button";
import {
  Field,
  FieldRow,
  FieldSelect,
  FieldSlider,
  FieldStack,
  FieldSwitch,
  FieldText,
  FieldTextarea,
  SaveStatus,
} from "./Field";
import { SettingItem } from "../SettingItem";
import { SettingsShell } from "../SettingsShell";
import styles from "./Field.stories.module.css";

const meta = {
  title: "UI/Field",
  component: Field,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Field>;

export default meta;
type Story = StoryObj<typeof meta>;

const sections = [
  { id: "general", label: "General", icon: Bot },
  { id: "providers", label: "Providers", icon: KeyRound },
  { id: "notifications", label: "Notifications", icon: Bell },
  { id: "storage", label: "Storage", icon: Database },
];

function useSaveSimulator() {
  const [status, setStatus] = useState<"idle" | "saving" | "saved" | "error">(
    "idle",
  );

  return {
    status,
    commit(nextStatus: "saved" | "error" = "saved") {
      setStatus("saving");
      window.setTimeout(() => setStatus(nextStatus), 450);
      window.setTimeout(() => setStatus("idle"), 1800);
    },
  };
}

function FieldDemo() {
  const [name, setName] = useState("Refact Agent");
  const [prompt, setPrompt] = useState("Be concise, helpful, and careful.");
  const [mode, setMode] = useState("agent");
  const [enabled, setEnabled] = useState(true);
  const [temperature, setTemperature] = useState([32]);
  const save = useSaveSimulator();

  return (
    <div className={styles.storyShell}>
      {(["light", "dark"] as const).map((appearance) => (
        <section
          className={styles.panel}
          data-appearance={appearance}
          key={appearance}
        >
          <div className={styles.sectionStack}>
            <h2 className={styles.title}>{appearance} fields</h2>
            <p className={styles.description}>
              Controlled controls support blur-save through onCommit and
              submit-save by letting the parent gather values.
            </p>
          </div>
          <FieldRow
            helper="Blur the input to commit the value."
            htmlFor={`${appearance}-name`}
            label="Agent name"
            control={
              <FieldText
                id={`${appearance}-name`}
                value={name}
                onChange={setName}
                onCommit={() => save.commit()}
              />
            }
          />
          <FieldStack
            helper="A vertical field for longer prompts."
            htmlFor={`${appearance}-prompt`}
            label="System prompt"
            control={
              <FieldTextarea
                id={`${appearance}-prompt`}
                rows={3}
                value={prompt}
                onChange={setPrompt}
              />
            }
          />
          <FieldRow
            error={
              mode === "planner"
                ? "Planner is unavailable in this mock account."
                : undefined
            }
            label="Default mode"
            control={
              <FieldSelect
                options={[
                  { value: "agent", label: "Agent" },
                  { value: "explore", label: "Explore" },
                  { value: "planner", label: "Planner" },
                ]}
                placeholder="Choose mode"
                value={mode}
                onChange={setMode}
              />
            }
          />
          <FieldRow
            helper="Switches commit immediately."
            label="Enable tools"
            control={
              <FieldSwitch
                checked={enabled}
                onChange={setEnabled}
                onCommit={() => save.commit()}
              />
            }
          />
          <FieldStack
            helper="Sliders change continuously and can commit on release."
            label="Temperature"
            control={
              <FieldSlider
                max={100}
                step={1}
                value={temperature}
                valueLabel={`${temperature[0]}%`}
                onChange={setTemperature}
                onCommit={() => save.commit()}
              />
            }
          />
          <div className={styles.actions}>
            <SaveStatus state={save.status} />
            <SaveStatus state="saving" />
            <SaveStatus state="saved" />
            <SaveStatus state="error" />
          </div>
        </section>
      ))}
      <section
        className={`${styles.panel} ${styles.narrowPanel}`}
        data-appearance="light"
      >
        <h2 className={styles.title}>Narrow field stack</h2>
        <FieldRow
          helper="Rows collapse to one column."
          label="Workspace"
          control={
            <FieldText value="/repo/refact" onChange={() => undefined} />
          }
        />
      </section>
    </div>
  );
}

function SettingsDemo({ narrow = false }: { narrow?: boolean }) {
  const [active, setActive] = useState("general");
  const [model, setModel] = useState("fast");
  const [autoSave, setAutoSave] = useState(true);
  const [draft, setDraft] = useState("Daily summary");
  const [submitted, setSubmitted] = useState("No submit yet");
  const save = useSaveSimulator();
  const content = useMemo(() => {
    if (active === "providers") {
      return (
        <div className={styles.sectionStack}>
          <SettingItem
            description="Blur-save provider name. The control owns local typing, parent owns persistence."
            saveStatus={save.status}
            title="Provider display name"
            control={
              <FieldText
                value={draft}
                onChange={setDraft}
                onCommit={() => save.commit()}
              />
            }
          />
          <SettingItem
            description="Select commits immediately and can still be collected by a form parent."
            title="Routing model"
            control={
              <FieldSelect
                options={[
                  { value: "fast", label: "Fast" },
                  { value: "balanced", label: "Balanced" },
                  { value: "deep", label: "Deep reasoning" },
                ]}
                value={model}
                onChange={setModel}
                onCommit={() => save.commit()}
              />
            }
          />
        </div>
      );
    }

    return (
      <form
        className={styles.sectionStack}
        onSubmit={(event) => {
          event.preventDefault();
          setSubmitted(
            `Submitted: ${draft}, ${model}, auto-save ${
              autoSave ? "on" : "off"
            }`,
          );
        }}
      >
        <SettingItem
          description="Row layout keeps simple controls aligned."
          title="Auto-save settings"
          control={<FieldSwitch checked={autoSave} onChange={setAutoSave} />}
        />
        <SettingItem
          description="Stack layout leaves room for validation copy and larger controls."
          layout="stack"
          title="Workspace label"
        >
          <FieldStack
            error={
              draft.length < 3 ? "Use at least three characters." : undefined
            }
            helper="This submit-save example gathers values from parent state."
            label="Label"
            control={<FieldText value={draft} onChange={setDraft} />}
          />
        </SettingItem>
        <div className={styles.actions}>
          <Button type="submit" variant="primary">
            Save settings
          </Button>
          <p className={styles.statusLine}>{submitted}</p>
        </div>
      </form>
    );
  }, [active, autoSave, draft, model, save, submitted]);

  return (
    <div className={styles.storyShell}>
      <section
        className={`${styles.panel} ${narrow ? styles.narrowPanel : ""}`}
        data-appearance="light"
      >
        <SettingsShell
          active={active}
          description="Two-pane settings shell with a section selector on narrow viewports."
          sections={sections}
          title="Settings"
          onSectionChange={setActive}
        >
          {content}
        </SettingsShell>
      </section>
    </div>
  );
}

export const Controls: Story = {
  render: () => <FieldDemo />,
};

export const SettingsPage: Story = {
  render: () => <SettingsDemo />,
};

export const NarrowSettingsPage: Story = {
  render: () => <SettingsDemo narrow />,
};
