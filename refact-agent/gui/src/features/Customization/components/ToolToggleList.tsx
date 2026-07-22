import React, { useMemo, useState } from "react";

import { Chip, Field, FieldSwitch, FieldText } from "../../../components/ui";
import type { ToolGroup } from "../../../services/refact/tools";
import { useGetToolGroupsQuery } from "../../../hooks/useGetToolGroupsQuery";
import styles from "./editors.module.css";

type ToolToggleListProps = {
  enabledTools: string[];
  onChange: (tools: string[]) => void;
};

export const ToolToggleList: React.FC<ToolToggleListProps> = ({
  enabledTools,
  onChange,
}) => {
  const [search, setSearch] = useState("");
  const { data: toolGroups = [], isLoading } = useGetToolGroupsQuery();

  const builtinGroups = useMemo<ToolGroup[]>(
    () => toolGroups.filter((group) => group.category === "builtin"),
    [toolGroups],
  );

  const builtinNames = useMemo(
    () =>
      new Set(
        builtinGroups.flatMap((group) =>
          group.tools.map((tool) => tool.spec.name),
        ),
      ),
    [builtinGroups],
  );

  const filteredGroups = useMemo(() => {
    const query = search.trim().toLowerCase();
    if (!query) return builtinGroups;

    return builtinGroups
      .map((group) => ({
        ...group,
        tools: group.tools.filter((tool) =>
          [
            group.name,
            group.description,
            tool.spec.name,
            tool.spec.display_name,
            tool.spec.description,
          ].some((value) => value.toLowerCase().includes(query)),
        ),
      }))
      .filter((group) => group.tools.length > 0);
  }, [builtinGroups, search]);

  const customTools = enabledTools.filter((name) => !builtinNames.has(name));

  const toggleTool = (name: string, checked: boolean) => {
    if (checked) {
      if (!enabledTools.includes(name)) onChange([...enabledTools, name]);
      return;
    }

    onChange(enabledTools.filter((tool) => tool !== name));
  };

  if (isLoading) {
    return <p className={styles.emptyText}>Loading tools...</p>;
  }

  return (
    <div className={styles.toolToggleList}>
      <Field label="Available Tools" className={styles.toolToggleSearch}>
        <FieldText
          value={search}
          onChange={setSearch}
          placeholder="Search tools..."
        />
      </Field>

      {filteredGroups.map((group) => (
        <section className={styles.toolToggleGroup} key={group.name}>
          <h3 className={styles.toolToggleGroupTitle}>{group.name}</h3>
          {group.tools.map((tool) => {
            const name = tool.spec.name;
            return (
              <div className={styles.toolToggleRow} key={name}>
                <FieldSwitch
                  checked={enabledTools.includes(name)}
                  onChange={(checked) => toggleTool(name, checked)}
                />
                <div className={styles.toolToggleInfo}>
                  <span className={styles.toolToggleName}>{name}</span>
                  <span className={styles.toolToggleDesc}>
                    {tool.spec.description}
                  </span>
                </div>
              </div>
            );
          })}
        </section>
      ))}

      {filteredGroups.length === 0 && (
        <p className={styles.emptyText}>No tools match your search.</p>
      )}

      {customTools.length > 0 && (
        <section className={styles.toolToggleCustom}>
          <h3 className={styles.toolToggleGroupTitle}>Custom tools</h3>
          <div className={styles.chipList}>
            {customTools.map((name) => (
              <Chip
                key={name}
                removable
                radius="chip"
                onRemove={() => toggleTool(name, false)}
              >
                {name}
              </Chip>
            ))}
          </div>
        </section>
      )}
    </div>
  );
};
