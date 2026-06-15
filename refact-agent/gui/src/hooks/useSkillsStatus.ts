import { useEffect, useState } from "react";
import { useGetSkillsStatusQuery } from "../services/refact/skillsStatus";

export function useSkillsStatus(chatId: string) {
  const [notFound, setNotFound] = useState(false);

  useEffect(() => {
    setNotFound(false);
  }, [chatId]);

  const { data, error } = useGetSkillsStatusQuery(chatId, {
    pollingInterval: 5000,
    skip: !chatId || notFound,
  });

  useEffect(() => {
    if (
      error &&
      "status" in error &&
      typeof error.status === "number" &&
      error.status === 404
    ) {
      setNotFound(true);
    }
  }, [error]);

  return {
    skillsEnabled: data?.skills_enabled ?? false,
    skillsAvailable: data?.skills_available ?? 0,
    skillsIncluded: data?.skills_included ?? [],
    activeSkill: data?.active_skill ?? null,
  };
}
