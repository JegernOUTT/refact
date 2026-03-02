import { describe, test, expect } from "vitest";
import { render } from "../../utils/test-utils";
import { SkillsIndicator } from "./SkillsIndicator";
import type { RootState } from "../../app/store";

function makeSkillsState(data: {
  skills_available: number;
  skills_included: string[];
  skills_enabled: boolean;
  active_skill: string | null;
}): Partial<RootState> {
  return {
    skillsStatusApi: {
      queries: {
        'getSkillsStatus("test-chat-id")': {
          status: "fulfilled",
          data,
          error: undefined,
          endpointName: "getSkillsStatus",
          requestId: "test",
          startedTimeStamp: Date.now(),
          fulfilledTimeStamp: Date.now(),
          originalArgs: "test-chat-id",
        },
      },
      mutations: {},
      provided: {},
      subscriptions: {},
      config: {
        online: true,
        focused: true,
        middlewareRegistered: true,
        refetchOnFocus: false,
        refetchOnReconnect: false,
        refetchOnMountOrArgChange: false,
        keepUnusedDataFor: 60,
        reducerPath: "skillsStatusApi",
        invalidationBehavior: "delayed",
      },
    },
  } as unknown as Partial<RootState>;
}

describe("SkillsIndicator", () => {
  test("renders correctly with skills data", () => {
    const preloadedState = makeSkillsState({
      skills_available: 5,
      skills_included: ["review", "docs"],
      skills_enabled: true,
      active_skill: null,
    });

    const { getByRole, getByText } = render(
      <SkillsIndicator chatId="test-chat-id" />,
      { preloadedState },
    );

    const indicator = getByRole("button");
    expect(indicator).toBeTruthy();
    expect(getByText(/5 available/)).toBeTruthy();
  });

  test("renders null when no skills available and no active skill", () => {
    const preloadedState = makeSkillsState({
      skills_available: 0,
      skills_included: [],
      skills_enabled: false,
      active_skill: null,
    });

    const { container } = render(<SkillsIndicator chatId="test-chat-id" />, {
      preloadedState,
    });
    expect(container.querySelector('[role="button"]')).toBeNull();
  });

  test("clicking navigates to extensions page", async () => {
    const preloadedState = makeSkillsState({
      skills_available: 3,
      skills_included: [],
      skills_enabled: true,
      active_skill: null,
    });

    const { getByRole, store, user } = render(
      <SkillsIndicator chatId="test-chat-id" />,
      { preloadedState },
    );

    const indicator = getByRole("button");
    await user.click(indicator);

    const pages = store.getState().pages;
    const lastPage = pages[pages.length - 1];
    expect(lastPage).toEqual({ name: "extensions", tab: "skills" });
  });

  test("renders active skill badge when active_skill is set", () => {
    const preloadedState = makeSkillsState({
      skills_available: 3,
      skills_included: [],
      skills_enabled: true,
      active_skill: "review-skill",
    });

    const { getByRole, getByText } = render(
      <SkillsIndicator chatId="test-chat-id" />,
      { preloadedState },
    );

    expect(getByRole("button")).toBeTruthy();
    expect(getByText("review-skill")).toBeTruthy();
    expect(getByText(/Active skill:/)).toBeTruthy();
  });

  test("renders only available count when no active skill", () => {
    const preloadedState = makeSkillsState({
      skills_available: 4,
      skills_included: [],
      skills_enabled: true,
      active_skill: null,
    });

    const { getByRole, getByText, queryByText } = render(
      <SkillsIndicator chatId="test-chat-id" />,
      { preloadedState },
    );

    expect(getByRole("button")).toBeTruthy();
    expect(getByText(/Skills: 4 available/)).toBeTruthy();
    expect(queryByText(/Active skill:/)).toBeNull();
  });

  test("renders nothing when no skills available and no active skill", () => {
    const preloadedState = makeSkillsState({
      skills_available: 0,
      skills_included: [],
      skills_enabled: false,
      active_skill: null,
    });

    const { container } = render(<SkillsIndicator chatId="test-chat-id" />, {
      preloadedState,
    });
    expect(container.querySelector('[role="button"]')).toBeNull();
  });
});
