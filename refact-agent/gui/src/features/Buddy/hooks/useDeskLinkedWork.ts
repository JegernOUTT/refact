import {
  useGetBuddyOperatorWorkItemQuery,
  type OperatorPage,
  type OperatorRecord,
} from "../../../services/refact/buddyOperator";
import type { WorkItem } from "../operatorTypes";

function sameProjectItem(
  page: OperatorPage<OperatorRecord<WorkItem>>,
  workId: string,
  projectId: string,
): WorkItem | undefined {
  const value = page.data.value;
  const matches =
    value.id === workId &&
    page.project.project_id === projectId &&
    value.project.project_id === projectId &&
    value.project.canonical_root === page.project.canonical_root;
  return matches ? value : undefined;
}

export function useDeskLinkedWork({
  endpointIdentity,
  workId,
  projectId,
}: {
  endpointIdentity: string;
  workId?: string;
  projectId?: string;
}) {
  const linkedWork = useGetBuddyOperatorWorkItemQuery(
    {
      id: workId ?? "",
      project_id: projectId,
      endpoint_identity: endpointIdentity,
    },
    { skip: !workId || !projectId, refetchOnMountOrArgChange: true },
  );
  const linkedPage = linkedWork.currentData;
  const linkedValue =
    linkedPage && workId !== undefined && projectId !== undefined
      ? sameProjectItem(linkedPage, workId, projectId)
      : undefined;
  const linkedFailure =
    workId !== undefined &&
    (!projectId ||
      Boolean(linkedWork.error) ||
      (linkedPage !== undefined && linkedValue === undefined));
  return { linkedWork, linkedValue, linkedFailure };
}
