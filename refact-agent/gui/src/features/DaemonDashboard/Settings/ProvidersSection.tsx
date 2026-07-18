import { useEffect, useMemo, useState } from "react";
import { ExternalLink } from "lucide-react";

import {
  Button,
  FieldSelect,
  LoadingState,
  SettingItem,
  Surface,
} from "../../../components/ui";
import { useListProjectsQuery } from "../../../services/refact/daemon";
import { SettingsGroup } from "../../Settings/SettingsSection";
import styles from "./SettingsPage.module.css";

export function ProvidersSection() {
  const { data: workers = [], isLoading } = useListProjectsQuery(undefined);
  const defaultProject = useMemo(
    () => workers.find((worker) => worker.pinned) ?? workers.at(0) ?? null,
    [workers],
  );
  const [projectId, setProjectId] = useState("");

  useEffect(() => {
    if (!defaultProject) {
      setProjectId("");
      return;
    }
    setProjectId((current) =>
      workers.some((worker) => worker.project_id === current)
        ? current
        : defaultProject.project_id,
    );
  }, [defaultProject, workers]);

  const selectedProject =
    workers.find((worker) => worker.project_id === projectId) ?? defaultProject;
  const projectHref = selectedProject
    ? `/p/${encodeURIComponent(selectedProject.project_id)}/`
    : "";

  return (
    <SettingsGroup title="Providers & Models">
      <Surface className={styles.sectionSurface} variant="glass">
        <p className={styles.sectionCopy}>
          Providers, models and API keys are configured per project.
        </p>
        {isLoading ? (
          <LoadingState kind="skeleton" label="Loading projects" />
        ) : workers.length === 0 ? (
          <p className={styles.muted}>
            Open a project before configuring its providers.
          </p>
        ) : (
          <SettingItem
            layout="stack"
            title="Project"
            description="The workspace opens at its home page. Choose Providers from its navigation."
            control={
              <div className={styles.projectControl}>
                <FieldSelect
                  aria-label="Provider settings project"
                  value={selectedProject?.project_id ?? ""}
                  onChange={setProjectId}
                  options={workers.map((worker) => ({
                    value: worker.project_id,
                    label: worker.pinned
                      ? `${worker.slug} · pinned`
                      : worker.slug,
                  }))}
                />
                {selectedProject ? (
                  <Button
                    asChild
                    leftIcon={ExternalLink}
                    size="sm"
                    variant="soft"
                  >
                    <a href={projectHref}>
                      Open provider settings for {selectedProject.slug}
                    </a>
                  </Button>
                ) : null}
              </div>
            }
          />
        )}
      </Surface>
    </SettingsGroup>
  );
}
