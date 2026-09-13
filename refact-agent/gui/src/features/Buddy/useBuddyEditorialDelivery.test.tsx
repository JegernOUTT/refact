import { StrictMode } from "react";
import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useBuddyEditorialDelivery } from "./useBuddyEditorialDelivery";

// The shared setup imports the application store and its hook graph before
// this file. Reload that graph so these boundary mocks apply to the hook.
vi.hoisted(() => vi.resetModules());

const mocks = vi.hoisted(() => ({
  identity: "endpoint",
  visible: true,
  fetching: false,
  error: false,
  editorial: true,
  epoch: 1,
  claim: vi.fn<() => { unwrap: () => Promise<boolean> }>(),
  feedback: vi.fn<() => { unwrap: () => Promise<boolean> }>(),
}));
vi.mock("../../hooks", () => ({ useAppSelector: () => mocks.identity }));
vi.mock("../../hooks/useIsDocumentVisible", () => ({
  useIsDocumentVisible: () => mocks.visible,
}));
vi.mock("../../services/refact/buddyEditorial", () => ({
  useClaimBuddyEditorialMutation: () => [mocks.claim],
  useFeedbackBuddyEditorialMutation: () => [mocks.feedback],
}));
vi.mock("../../services/refact/buddySupervision", () => ({
  useGetBuddySupervisionContextQuery: () => ({
    isFetching: mocks.fetching,
    error: mocks.error,
    currentData: {
      project: { project_id: "p", canonical_root: "/p" },
      worker: { worker_id: "w", engine_instance_id: "e", epoch: mocks.epoch },
      features: { editorial: mocks.editorial },
    },
  }),
}));
const candidate = {
  id: "speech-1",
  text: "Bounded observation",
  editorial_delivery_required: true,
};
const flush = async () => {
  await act(async () => {
    await Promise.resolve();
  });
};
beforeEach(() => {
  mocks.identity = "endpoint";
  mocks.visible = true;
  mocks.fetching = false;
  mocks.error = false;
  mocks.editorial = true;
  mocks.epoch = 1;
  mocks.claim
    .mockReset()
    .mockImplementation(() => ({ unwrap: () => Promise.resolve(true) }));
  mocks.feedback
    .mockReset()
    .mockImplementation(() => ({ unwrap: () => Promise.resolve(true) }));
});
describe("editorial delivery", () => {
  it("does not claim while hidden or disabled", async () => {
    mocks.visible = false;
    const { result, rerender } = renderHook(
      ({ enabled }) => useBuddyEditorialDelivery(candidate, "panel", enabled),
      { initialProps: { enabled: true } },
    );
    await flush();
    expect(mocks.claim).not.toHaveBeenCalled();
    expect(result.current.speech).toBeNull();
    mocks.visible = true;
    rerender({ enabled: false });
    await flush();
    expect(mocks.claim).not.toHaveBeenCalled();
    expect(result.current.speech).toBeNull();
  });
  it("claims once in StrictMode before display and binds feedback", async () => {
    const { result } = renderHook(
      () => useBuddyEditorialDelivery(candidate, "panel"),
      { wrapper: StrictMode },
    );
    expect(result.current.speech).toBeNull();
    await flush();
    expect(mocks.claim).toHaveBeenCalledTimes(1);
    expect(mocks.claim).toHaveBeenCalledWith(
      expect.objectContaining({
        project: { project_id: "p", canonical_root: "/p" },
        worker: { worker_id: "w", engine_instance_id: "e", epoch: 1 },
        surface: "panel",
      }),
    );
    expect(result.current.speech).toEqual(candidate);
    await act(() => result.current.sendFeedback("noisy"));
    expect(mocks.feedback).toHaveBeenCalledWith(
      expect.objectContaining({
        speech_id: candidate.id,
        feedback: "noisy",
        worker: expect.objectContaining({ epoch: 1 }),
      }),
    );
    expect(result.current.feedbackStatus).toBe(
      "Feedback recorded. Permissions unchanged.",
    );
  });
  it("never claims or displays editorial speech on the avatar", async () => {
    const { result } = renderHook(() =>
      useBuddyEditorialDelivery(candidate, "avatar"),
    );
    await flush();
    expect(result.current.speech).toBeNull();
    expect(mocks.claim).not.toHaveBeenCalled();
  });
  it("withholds uncertain delivery without retry on rerender or visibility change", async () => {
    mocks.claim.mockImplementation(() => ({
      unwrap: () => Promise.reject(new Error("offline")),
    }));
    const { result, rerender } = renderHook(() =>
      useBuddyEditorialDelivery(candidate, "panel"),
    );
    await flush();
    mocks.visible = false;
    rerender();
    mocks.visible = true;
    rerender();
    await flush();
    expect(mocks.claim).toHaveBeenCalledTimes(1);
    expect(result.current.speech).toBeNull();
  });
  it("withholds stale acknowledgement and changed content under the same id", async () => {
    let finish: (value: boolean) => void = () => {
      throw new Error("not started");
    };
    const response = new Promise<boolean>((resolve) => {
      finish = resolve;
    });
    mocks.claim.mockImplementation(() => ({ unwrap: () => response }));
    const { result, rerender } = renderHook(
      ({ text }) => useBuddyEditorialDelivery({ ...candidate, text }, "panel"),
      { initialProps: { text: candidate.text } },
    );
    rerender({ text: "Changed observation" });
    await act(async () => {
      finish(true);
      await response;
    });
    expect(result.current.speech).toBeNull();
    expect(mocks.claim).toHaveBeenCalledTimes(1);
  });
  it("hides delivered speech during refresh, errors, disabled rollout and worker changes", async () => {
    const { result, rerender } = renderHook(() =>
      useBuddyEditorialDelivery(candidate, "panel"),
    );
    await flush();
    expect(result.current.speech).toEqual(candidate);
    mocks.fetching = true;
    rerender();
    expect(result.current.speech).toBeNull();
    mocks.fetching = false;
    mocks.error = true;
    rerender();
    expect(result.current.speech).toBeNull();
    mocks.error = false;
    mocks.editorial = false;
    rerender();
    expect(result.current.speech).toBeNull();
    mocks.editorial = true;
    mocks.epoch = 2;
    rerender();
    expect(result.current.speech).toBeNull();
    await flush();
    expect(mocks.claim).toHaveBeenCalledTimes(2);
  });
  it("bounds retained claims without evicting uncertain ids for automatic replay", async () => {
    mocks.claim.mockImplementation(() => ({
      unwrap: () => Promise.resolve(false),
    }));
    const { result, rerender } = renderHook(
      ({ id }) => useBuddyEditorialDelivery({ ...candidate, id }, "panel"),
      { initialProps: { id: "speech-0" } },
    );
    for (let i = 1; i <= 130; i++) rerender({ id: `speech-${i}` });
    await flush();
    expect(mocks.claim).toHaveBeenCalledTimes(128);
    rerender({ id: "speech-0" });
    await flush();
    expect(mocks.claim).toHaveBeenCalledTimes(128);
    expect(result.current.speech).toBeNull();
  });
});
