import React, { useState } from "react";
import { Button, Popover, Select } from "../../../../components/ui";
import {
  organizationItemKey,
  useApplyOrganizationMutation,
  type Assignment,
  type OrganizationOperation,
} from "../../../../services/refact/organization";
import { useOrganization } from "../../hooks/useOrganization";
import type { StreamItem } from "../../streamSelectors";
import styles from "../../components/FilterBar/FilterBar.module.css";

export const OrganizationEditor: React.FC<{ item: StreamItem }> = ({
  item,
}) => {
  const organization = useOrganization();
  const key = item.organizationKey ?? { kind: item.kind, id: item.id };
  const assignment = organization.byKey.get(organizationItemKey(key));
  const [apply, mutation] = useApplyOrganizationMutation();
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [createKind, setCreateKind] = useState<"create_topic" | "create_tag">(
    "create_topic",
  );
  const save = async (
    changes: Partial<Assignment>,
    create?: { op: "create_topic" | "create_tag"; name: string },
  ) => {
    const revision = organization.assignments.currentData?.revision;
    if (revision === undefined || !assignment) return;
    setError(null);
    try {
      let expected = revision;
      const next: Assignment = { ...assignment, ...changes, item: key };
      if (create) {
        const created = await apply({
          scope: organization.scope,
          expected_revision: expected,
          request_id: crypto.randomUUID(),
          operations: [create],
        }).unwrap();
        const id = created.results[0]?.id;
        if (!id) throw new Error("Organization did not return the new ID");
        expected = created.revision;
        if (create.op === "create_topic") {
          next.topic_id = id;
          next.topic_locked = true;
        } else {
          next.tag_ids = [...next.tag_ids, id];
          next.tags_locked = true;
        }
      }
      const operation: OrganizationOperation = { op: "assign_item", ...next };
      await apply({
        scope: organization.scope,
        expected_revision: expected,
        request_id: crypto.randomUUID(),
        operations: [operation],
      }).unwrap();
      setName("");
    } catch {
      setError(
        "Organization could not be saved or changed elsewhere. Reload metadata, review your selection, and try again.",
      );
      organization.retry();
    }
  };
  const disabled =
    mutation.isLoading ||
    !assignment ||
    organization.error ||
    organization.assignments.isFetching;
  return (
    <Popover>
      <Popover.Trigger asChild>
        <Button size="sm" variant="soft">
          Topic & tags
        </Button>
      </Popover.Trigger>
      <Popover.Content maxWidth="360px">
        <div className={styles.options}>
          {!assignment && (
            <span role="status">Organization unavailable for this item.</span>
          )}
          <span>Topic</span>
          <Select
            value={assignment?.topic_id ?? "none"}
            onValueChange={(value) =>
              void save({
                topic_id: value === "none" ? null : value,
                topic_locked: true,
              })
            }
            disabled={disabled}
          >
            <Select.Trigger aria-label="Item topic">
              {!assignment ? "Unknown" : undefined}
            </Select.Trigger>
            <Select.Content>
              <Select.Item value="none">No topic</Select.Item>
              {(organization.topics.currentData?.items ?? [])
                .filter((topic) => !topic.archived)
                .map((topic) => (
                  <Select.Item key={topic.id} value={topic.id}>
                    {topic.name}
                  </Select.Item>
                ))}
            </Select.Content>
          </Select>
          <span>Tags</span>
          {(organization.tags.currentData?.items ?? [])
            .filter((tag) => !tag.archived)
            .map((tag) => (
              <label key={tag.id}>
                <input
                  type="checkbox"
                  disabled={disabled}
                  checked={assignment?.tag_ids.includes(tag.id) ?? false}
                  onChange={(event) =>
                    void save({
                      tag_ids: event.target.checked
                        ? [...(assignment?.tag_ids ?? []), tag.id]
                        : assignment?.tag_ids.filter((id) => id !== tag.id),
                      tags_locked: true,
                    })
                  }
                />
                {tag.name}
              </label>
            ))}
          <Button
            size="sm"
            variant="ghost"
            disabled={disabled || !assignment.topic_locked}
            onClick={() => void save({ topic_locked: false })}
          >
            Let Buddy manage topic
          </Button>
          <Button
            size="sm"
            variant="ghost"
            disabled={disabled || !assignment.tags_locked}
            onClick={() => void save({ tags_locked: false })}
          >
            Let Buddy manage tags
          </Button>
          <Select
            value={createKind}
            onValueChange={(value) => {
              if (value === "create_topic" || value === "create_tag")
                setCreateKind(value);
            }}
          >
            <Select.Trigger aria-label="Create organization kind" />
            <Select.Content>
              <Select.Item value="create_topic">New topic</Select.Item>
              <Select.Item value="create_tag">New tag</Select.Item>
            </Select.Content>
          </Select>
          <input
            aria-label="New topic or tag name"
            placeholder="Name"
            value={name}
            onChange={(event) => setName(event.target.value)}
          />
          <Button
            size="sm"
            disabled={disabled || !name.trim()}
            onClick={() => void save({}, { op: createKind, name: name.trim() })}
          >
            Create and assign
          </Button>
          {(error !== null || organization.error) && (
            <span role="alert">{error ?? "Organization unavailable."}</span>
          )}
          <Button size="sm" variant="ghost" onClick={organization.retry}>
            Reload metadata
          </Button>
        </div>
      </Popover.Content>
    </Popover>
  );
};
