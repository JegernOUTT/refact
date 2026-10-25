import React from "react";
import { GoalPlanSection } from "../../components/TaskProgressWidget/TaskProgressWidget";
import { Markdown } from "../../components/Markdown";
import { Button, Flex, Spinner, Text } from "../../components/ui";
import { useGetTaskGoalQuery } from "../../services/refact/tasks";

/** The task owns this state; planner documents are reference material only. */
export const TaskGoalPanel: React.FC<{ taskId: string }> = ({ taskId }) => {
  const {
    currentData: goal,
    isFetching,
    isError,
    refetch,
  } = useGetTaskGoalQuery(taskId, {
    pollingInterval: 5000,
    refetchOnMountOrArgChange: true,
  });

  return (
    <Flex direction="column" gap="3">
      <Flex justify="between" align="center" gap="2">
        <Text size="2" color="gray">
          Task-owned authority, retained when a new planner starts.
        </Text>
        <Button
          size="sm"
          variant="soft"
          disabled={isFetching}
          onClick={() => void refetch()}
        >
          Reload goal
        </Button>
      </Flex>
      <Text size="2" color="gray">
        Ask the current owner planner to update the goal or its plan. Documents,
        including legacy master plans, do not change this state.
      </Text>
      {isError ? (
        <Text role="alert" color="red">
          Failed to load the task goal. Reload to obtain current authority
          before making changes.
        </Text>
      ) : goal === undefined ? (
        <Spinner />
      ) : goal === null ? (
        <Text>
          No task goal installed. Ask the owner planner to set a goal.
        </Text>
      ) : (
        <>
          <Text size="2" color="gray">
            Goal v{goal.version} · revision {goal.revision ?? goal.version} ·{" "}
            {goal.status}
          </Text>
          <Markdown canHaveInteractiveElements={false}>{goal.content}</Markdown>
          {goal.plan ? (
            <GoalPlanSection plan={goal.plan} />
          ) : (
            <Text color="gray">No plan attached to this goal.</Text>
          )}
        </>
      )}
    </Flex>
  );
};
