import { beforeEach, describe, expect, test, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { fireEvent, render, screen, waitFor } from "../utils/test-utils";
import { setUpStore } from "../app/store";
import { server } from "../utils/mockServer";
import { BuddyChatCompanion } from "../features/Buddy/BuddyChatCompanion";
import {
  defaultBuddySettings,
  setBuddySnapshot,
} from "../features/Buddy/buddySlice";
import type {
  BuddySnapshot,
  BuddyState,
  ConductorGoal,
} from "../features/Buddy/types";

function makeState(): BuddyState {
  return {
    identity: {
      name: "Pixel",
      created_at: "2024-01-01T00:00:00Z",
      palette_index: 0,
    },
    progression: { stage: 0, stage_name: "Egg", level: 1, xp: 0, xp_next: 30 },
    skills: { unlocked: [], locked: [] },
    workflow_summaries: [],
    semantic: {
      mood: "idle",
      focus: "none",
      headline: "",
      last_active: "2024-01-01T00:00:00Z",
    },
    recent_activities: [],
    suggestion_state: [],
    pet: {
      needs: {
        hunger: 80,
        energy: 85,
        hygiene: 80,
        boredom: 15,
        affection: 75,
      },
      condition: {
        sleeping: false,
        hungry: false,
        sleepy: false,
        dirty: false,
        bored: false,
        lonely: false,
      },
      evolution: {
        care_score: 0,
        neglect_score: 0,
        open_seconds: 0,
        last_evolved_at: null,
      },
    },
    personality: {
      archetype_id: "helper_sprite",
      archetype_label: "Helper Sprite",
      vibe: "Playful",
      summary: "Helpful gremlin",
      prompt: "Helpful gremlin",
      traits: {
        playfulness: 70,
        chaos: 35,
        sociability: 72,
        curiosity: 78,
        resilience: 66,
      },
    },
    active_quest: null,
    opportunities: [],
  };
}

function makeSnapshot(overrides?: Partial<BuddySnapshot>): BuddySnapshot {
  return {
    state: makeState(),
    settings: defaultBuddySettings(),
    enabled: true,
    recent_diagnostics: [],
    ...overrides,
  };
}

function makeGoal(overrides?: Partial<ConductorGoal>): ConductorGoal {
  return {
    id: "goal-1",
    title: "Tiny captain goal",
    plan_doc_slug: "master-plan",
    plan_markdown: "# Tiny captain goal",
    done_when: { summary: "Done", checklist: [] },
    status: "active",
    autonomy: "governed",
    budget: { total_tokens: 10000, usd: null },
    spent: {
      elapsed_secs: 125,
      prompt_tokens: 1200,
      completion_tokens: 300,
      total_tokens: 1500,
      cache_read_tokens: 250,
      usd: null,
      no_progress_wakes: 0,
    },
    summary: {
      task_count: 2,
      chat_count: 2,
      memo_count: 0,
      learning_record_count: 0,
      pending_question_count: 0,
      open_question_count: 0,
      ghost_message_count: 0,
      no_progress_wakes: 0,
      turn_failures: 0,
      has_planner_task: true,
      has_conductor_chat: true,
    },
    ledger: {
      status: "active",
      autonomy: "governed",
      planner_task_id: "planner-1",
      task_ids: ["card-1", "card-2"],
      chat_ids: ["chat-a", "agent-chat"],
      memos: [],
      learning_records: [],
      pending_questions: [],
      ghost_messages: [],
      no_progress_wakes: 0,
      turn_failures: 0,
    },
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:10Z",
    completed_at: null,
    ...overrides,
  };
}

const noopContext = {
  clearRect: vi.fn(),
  fillRect: vi.fn(),
  fillText: vi.fn(),
  save: vi.fn(),
  restore: vi.fn(),
  scale: vi.fn(),
  translate: vi.fn(),
  beginPath: vi.fn(),
  arc: vi.fn(),
  ellipse: vi.fn(),
  fill: vi.fn(),
  moveTo: vi.fn(),
  lineTo: vi.fn(),
  stroke: vi.fn(),
  getImageData: vi.fn(() => ({ data: new Uint8ClampedArray(4) }) as ImageData),
  putImageData: vi.fn(),
} as unknown as CanvasRenderingContext2D;

describe("BuddyChatCompanion conductor cockpit", () => {
  beforeEach(() => {
    localStorage.clear();
    vi.restoreAllMocks();
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      window.setTimeout(() => callback(0), 0);
      return 1;
    });
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation(
      () => undefined,
    );
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(
      noopContext,
    );
    server.use(
      http.get("*/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
    );
  });

  test("renders owned chat cockpit with tokens-only stamina budget", async () => {
    const store = setUpStore();
    store.dispatch(
      setBuddySnapshot(makeSnapshot({ conductor_goals: [makeGoal()] })),
    );

    render(<BuddyChatCompanion chatId="chat-a" />, { store });

    expect(
      await screen.findByLabelText("Buddy conductor cockpit"),
    ).toBeInTheDocument();
    expect(screen.getByText("Tiny captain goal")).toBeInTheDocument();
    expect(screen.getByText("Buddy stamina")).toBeInTheDocument();
    expect(screen.getByText("Tokens 1.5K / 10.0K")).toBeInTheDocument();
    expect(screen.getByText("USD —")).toBeInTheDocument();
    expect(screen.getByText("Planner 1")).toBeInTheDocument();
    expect(screen.getByText("Cards 2")).toBeInTheDocument();
    expect(screen.getByText("Agents 2")).toBeInTheDocument();
  });

  test("shows human-yield state clearly", async () => {
    const store = setUpStore();
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot({
          conductor_goals: [
            makeGoal({
              status: "paused",
              summary: {
      task_count: 2,
      chat_count: 2,
      memo_count: 0,
      learning_record_count: 0,
      pending_question_count: 0,
      open_question_count: 1,
      ghost_message_count: 0,
      no_progress_wakes: 0,
      turn_failures: 0,
      has_planner_task: true,
      has_conductor_chat: true,
    },
    ledger: {
                ...makeGoal().ledger,
                status: "paused",
                pending_questions: [
                  {
                    id: "q-1",
                    question: "Need a human",
                    asked_at: "2024-01-01T00:00:00Z",
                    blocking: true,
                  },
                ],
              },
            }),
          ],
        }),
      ),
    );

    render(<BuddyChatCompanion chatId="chat-a" />, { store });

    expect(await screen.findByText("Human yield · 1")).toBeInTheDocument();
  });

  test("pause resume autonomy and manual wake buttons call conductor endpoints", async () => {
    const calls: string[] = [];
    server.use(
      http.post("*/v1/buddy/conductor/goals/:goalId/pause", ({ params }) => {
        calls.push(`pause:${String(params.goalId)}`);
        return HttpResponse.json(
          makeGoal({ id: String(params.goalId), status: "paused" }),
        );
      }),
      http.post("*/v1/buddy/conductor/goals/:goalId/resume", ({ params }) => {
        calls.push(`resume:${String(params.goalId)}`);
        return HttpResponse.json(
          makeGoal({ id: String(params.goalId), status: "active" }),
        );
      }),
      http.post(
        "*/v1/buddy/conductor/goals/:goalId/manual_wake",
        ({ params }) => {
          calls.push(`wake:${String(params.goalId)}`);
          return HttpResponse.json({
            goal_id: String(params.goalId),
            enqueued: true,
          });
        },
      ),
      http.post(
        "*/v1/buddy/conductor/goals/:goalId/autonomy",
        async ({ params, request }) => {
          const body = (await request.json()) as { autonomy: string };
          calls.push(`autonomy:${String(params.goalId)}:${body.autonomy}`);
          return HttpResponse.json(
            makeGoal({
              id: String(params.goalId),
              autonomy: body.autonomy as ConductorGoal["autonomy"],
            }),
          );
        },
      ),
    );
    const store = setUpStore();
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot({
          conductor_goals: [
            makeGoal({ id: "goal-controls", status: "proposed" }),
          ],
        }),
      ),
    );

    render(<BuddyChatCompanion chatId="chat-a" />, { store });
    fireEvent.click(await screen.findByRole("button", { name: "Pause" }));
    await waitFor(() => expect(calls).toContain("pause:goal-controls"));
    fireEvent.click(screen.getByRole("button", { name: "Resume" }));
    await waitFor(() => expect(calls).toContain("resume:goal-controls"));
    fireEvent.click(screen.getByRole("button", { name: "Manual wake" }));
    await waitFor(() => expect(calls).toContain("wake:goal-controls"));
    fireEvent.change(screen.getByLabelText("Set conductor autonomy"), {
      target: { value: "read_only" },
    });

    await waitFor(() => {
      expect(calls).toEqual([
        "pause:goal-controls",
        "resume:goal-controls",
        "wake:goal-controls",
        "autonomy:goal-controls:read_only",
      ]);
    });
  });

  test("open conductor log navigates to owned chat", async () => {
    const store = setUpStore();
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot({
          conductor_goals: [
            makeGoal({
              summary: {
      task_count: 2,
      chat_count: 2,
      memo_count: 0,
      learning_record_count: 0,
      pending_question_count: 0,
      open_question_count: 0,
      ghost_message_count: 0,
      no_progress_wakes: 0,
      turn_failures: 0,
      has_planner_task: true,
      has_conductor_chat: true,
    },
    ledger: {
                ...makeGoal().ledger,
                chat_ids: ["conductor-log-chat"],
              },
            }),
          ],
        }),
      ),
    );

    render(<BuddyChatCompanion chatId="conductor-log-chat" />, { store });
    fireEvent.click(
      await screen.findByRole("button", { name: "Open conductor log" }),
    );

    await waitFor(() => {
      expect(store.getState().pages.at(-1)).toEqual({ name: "conductor" });
    });
  });
});
