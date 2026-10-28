import { useEffect, useRef } from "react";
import { useGetSkillsStatusQuery } from "../services/refact/skillsStatus";
import { selectSkillsStatusRevisionById } from "../features/Chat/Thread/selectors";
import { useAppSelector } from "./useAppSelector";
import { useIsDocumentVisible } from "./useIsDocumentVisible";

const SKILLS_STATUS_POLLING_INTERVAL_MS = 60_000;

export function useSkillsStatus(chatId: string) {
  const revision = useAppSelector((state) =>
    selectSkillsStatusRevisionById(state, chatId),
  );
  const visible = useIsDocumentVisible();
  const { data, refetch, isUninitialized } = useGetSkillsStatusQuery(chatId, {
    pollingInterval: visible ? SKILLS_STATUS_POLLING_INTERVAL_MS : 0,
    skip: !chatId,
  });

  const lastSeenRef = useRef({ chatId, revision });
  useEffect(() => {
    const last = lastSeenRef.current;
    lastSeenRef.current = { chatId, revision };
    if (last.chatId !== chatId || last.revision === revision) return;
    if (!isUninitialized) void refetch();
  }, [chatId, revision, refetch, isUninitialized]);

  return {
    skillsEnabled: data?.skills_enabled ?? false,
    skillsAvailable: data?.skills_available ?? 0,
    skillsIncluded: data?.skills_included ?? [],
    activeSkill: data?.active_skill ?? null,
  };
}
