import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, screen, waitFor } from "@testing-library/react";
import { render } from "../../utils/test-utils";
import { AutomationCredentials } from "./AutomationCredentials";
import * as api from "../../services/refact/automationCredentialsApi";

vi.hoisted(() => {
  vi.resetModules();
});

vi.mock(
  "../../services/refact/automationCredentialsApi",
  async (importOriginal) => {
    const original = await importOriginal<typeof api>();
    return {
      ...original,
      credentialHostAvailable: vi.fn(() => true),
      listCredentials: vi.fn(),
      mutateCredential: vi.fn(),
    };
  },
);
const item: api.CredentialMetadata = {
  id: "fixture",
  origin: "https://example.com",
  header: "authorization",
  tenant: "tenant",
  principal: "reader",
  scope: ["read"],
  identity_revision: "identity-r1",
  secret_revision: "secret-r1",
};
beforeEach(() => {
  vi.mocked(api.credentialHostAvailable).mockReturnValue(true);
  vi.mocked(api.listCredentials).mockResolvedValue({
    supported: true,
    project_root: "/project",
    credentials: [item],
  });
  vi.mocked(api.mutateCredential).mockResolvedValue();
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("operator credentials", () => {
  it("clears password before a pending rotation settles, pins exact revision, and never renders its value", async () => {
    let finish: (() => void) | undefined;
    vi.mocked(api.mutateCredential).mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    );
    render(<AutomationCredentials />);
    fireEvent.click(
      await screen.findByRole("button", { name: "Rotate fixture" }),
    );
    const input = screen.getByLabelText<HTMLInputElement>("Secret value");
    expect(input.type).toBe("password");
    fireEvent.change(input, { target: { value: "fixture-secret-only" } });
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: "Confirm rotation" }));
    await waitFor(() => expect(input.value).toBe(""));
    expect(api.mutateCredential).toHaveBeenCalledWith(
      {
        operation: "rotate",
        project_root: "/project",
        id: "fixture",
        expected_secret_revision: "secret-r1",
        secret: "fixture-secret-only",
      },
      null,
    );
    expect(document.body.textContent).not.toContain("fixture-secret-only");
    finish?.();
    await screen.findByText(/Credential operation completed/);
  });
  it("revokes only after confirmation using expected secret revision", async () => {
    render(<AutomationCredentials />);
    fireEvent.click(
      await screen.findByRole("button", { name: "Revoke fixture" }),
    );
    expect(screen.queryByLabelText("Secret value")).toBeNull();
    expect(
      screen.getByRole<HTMLButtonElement>("button", {
        name: "Confirm revocation",
      }).disabled,
    ).toBe(true);
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: "Confirm revocation" }));
    await waitFor(() =>
      expect(api.mutateCredential).toHaveBeenCalledWith(
        {
          operation: "remove",
          project_root: "/project",
          id: "fixture",
          expected_secret_revision: "secret-r1",
        },
        null,
      ),
    );
  });
  it("renders explicit unavailable and unsupported states without a secret control", async () => {
    vi.mocked(api.credentialHostAvailable).mockReturnValue(false);
    const view = render(<AutomationCredentials />);
    expect(screen.getByText(/top-level browser UI/)).toBeTruthy();
    expect(api.listCredentials).not.toHaveBeenCalled();
    expect(screen.queryByLabelText("Secret value")).toBeNull();
    view.unmount();
    vi.mocked(api.credentialHostAvailable).mockReturnValue(true);
    vi.mocked(api.listCredentials).mockResolvedValue({
      supported: false,
      project_root: "/project",
      credentials: [],
    });
    render(<AutomationCredentials />);
    await screen.findByText(/unsupported on this platform/);
    expect(screen.queryByLabelText("Secret value")).toBeNull();
  });
  it("clears secret on failed mutation and does not echo raw error contents", async () => {
    vi.mocked(api.mutateCredential).mockRejectedValue(
      new Error("fixture-secret-in-error"),
    );
    render(<AutomationCredentials />);
    fireEvent.click(
      await screen.findByRole("button", { name: "Rotate fixture" }),
    );
    fireEvent.change(screen.getByLabelText<HTMLInputElement>("Secret value"), {
      target: { value: "fixture-secret" },
    });
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: "Confirm rotation" }));
    await screen.findByText(/Credential operation refused/);
    expect(screen.getByLabelText<HTMLInputElement>("Secret value").value).toBe(
      "",
    );
    expect(document.body.textContent).not.toContain("fixture-secret-in-error");
  });
});
