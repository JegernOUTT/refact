import { describe, test, expect, vi } from "vitest";
import { runPickerPolling } from "./pickerPolling";
import type { BrowserElementPickResultResponse } from "../../services/refact/browser";

const noWait = () => Promise.resolve();

function pickedResponse(): BrowserElementPickResultResponse {
  return {
    selector: "#submit",
    innerText: "Submit",
    bbox: { x: 1, y: 2, width: 3, height: 4 },
  };
}

describe("runPickerPolling", () => {
  test("returns the picked element and never cancels", async () => {
    const cancel = vi.fn(() => Promise.resolve());
    const poll = vi
      .fn<() => Promise<BrowserElementPickResultResponse>>()
      .mockResolvedValueOnce({ status: "waiting" })
      .mockResolvedValueOnce(pickedResponse());

    const picked = await runPickerPolling({
      poll,
      cancel,
      wait: noWait,
      maxAttempts: 5,
    });

    expect(picked).toEqual({
      selector: "#submit",
      innerText: "Submit",
      bbox: { x: 1, y: 2, width: 3, height: 4 },
    });
    expect(poll).toHaveBeenCalledTimes(2);
    expect(cancel).not.toHaveBeenCalled();
  });

  test("cancels the page-side picker when polling gives up", async () => {
    const cancel = vi.fn(() => Promise.resolve());
    const poll = vi.fn(() =>
      Promise.resolve<BrowserElementPickResultResponse>({ status: "waiting" }),
    );

    const picked = await runPickerPolling({
      poll,
      cancel,
      wait: noWait,
      maxAttempts: 3,
    });

    expect(picked).toBeNull();
    expect(poll).toHaveBeenCalledTimes(3);
    expect(cancel).toHaveBeenCalledTimes(1);
  });

  test("cancels the page-side picker when polling throws", async () => {
    const cancel = vi.fn(() => Promise.resolve());
    const poll = vi.fn(() => Promise.reject(new Error("runtime gone")));

    await expect(
      runPickerPolling({ poll, cancel, wait: noWait, maxAttempts: 3 }),
    ).rejects.toThrow("runtime gone");
    expect(cancel).toHaveBeenCalledTimes(1);
  });
});
