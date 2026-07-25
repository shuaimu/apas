import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { AnchorHTMLAttributes, ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ResetPasswordPage from "./page";

type LinkProps = AnchorHTMLAttributes<HTMLAnchorElement> & {
  href: string;
  children: ReactNode;
};

const navigation = vi.hoisted(() => {
  const state = {
    push: vi.fn(),
    token: null as string | null,
    searchParams: {
      get: vi.fn(),
    },
  };
  state.searchParams.get.mockImplementation((key: string) =>
    key === "token" ? state.token : null,
  );
  return state;
});

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: navigation.push }),
  useSearchParams: () => navigation.searchParams,
}));

vi.mock("next/link", () => ({
  default: ({ href, children, ...props }: LinkProps) => (
    <a href={href} {...props}>
      {children}
    </a>
  ),
}));

const originalFetch = globalThis.fetch;
const fetchMock = vi.fn();

function passwordInput(): HTMLInputElement {
  return screen.getByLabelText("New Password") as HTMLInputElement;
}

function confirmPasswordInput(): HTMLInputElement {
  return screen.getByLabelText("Confirm New Password") as HTMLInputElement;
}

function submitButton(
  name: string | RegExp = "Reset password",
): HTMLButtonElement {
  return screen.getByRole("button", { name }) as HTMLButtonElement;
}

function submitForm() {
  const form = submitButton().closest("form");
  if (!form) {
    throw new Error("Reset password form was not rendered");
  }
  fireEvent.submit(form);
}

function fillResetPasswordForm({
  password = "secret-pass",
  confirmPassword = password,
}: {
  password?: string;
  confirmPassword?: string;
} = {}) {
  fireEvent.change(passwordInput(), { target: { value: password } });
  fireEvent.change(confirmPasswordInput(), { target: { value: confirmPassword } });
}

async function renderResetPasswordPage(token = "reset-token") {
  navigation.token = token;
  render(<ResetPasswordPage />);
  await screen.findByText("Set your new password");
}

function jsonResponse(body: unknown, ok = true) {
  return {
    ok,
    json: vi.fn().mockResolvedValue(body),
  };
}

describe("ResetPasswordPage", () => {
  beforeEach(() => {
    navigation.push.mockReset();
    navigation.token = null;
    navigation.searchParams.get.mockClear();
    fetchMock.mockReset();
    globalThis.fetch = fetchMock as unknown as typeof fetch;
    vi.useRealTimers();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.useRealTimers();
    globalThis.fetch = originalFetch;
    document.body.innerHTML = "";
  });

  it("renders an invalid-link state when the token query parameter is missing", () => {
    render(<ResetPasswordPage />);

    expect(screen.getByText("Invalid Reset Link")).toBeTruthy();
    expect(screen.getByText("This password reset link is invalid or has expired.")).toBeTruthy();
    expect(
      screen.getByRole("link", { name: "Request a new reset link" }).getAttribute("href"),
    ).toBe("/forgot-password");
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("rejects mismatched passwords before calling fetch", async () => {
    await renderResetPasswordPage();
    fillResetPasswordForm({
      password: "secret-pass",
      confirmPassword: "different-pass",
    });
    submitForm();

    expect(await screen.findByText("Passwords do not match")).toBeTruthy();
    expect(fetchMock).not.toHaveBeenCalled();
    expect(navigation.push).not.toHaveBeenCalled();
  });

  it("rejects short passwords before calling fetch", async () => {
    await renderResetPasswordPage();
    fillResetPasswordForm({ password: "short", confirmPassword: "short" });
    submitForm();

    expect(await screen.findByText("Password must be at least 6 characters")).toBeTruthy();
    expect(fetchMock).not.toHaveBeenCalled();
    expect(navigation.push).not.toHaveBeenCalled();
  });

  it("posts the reset request, renders success, and redirects after the timer", async () => {
    const originalSetTimeout = globalThis.setTimeout;
    const setTimeoutSpy = vi
      .spyOn(globalThis, "setTimeout")
      .mockImplementation((callback: TimerHandler, timeout?: number) => {
        if (timeout === 3000 && typeof callback === "function") {
          callback();
          return 0 as unknown as ReturnType<typeof setTimeout>;
        }
        return originalSetTimeout(callback, timeout);
      });
    fetchMock.mockResolvedValueOnce(jsonResponse({ success: true }));

    await renderResetPasswordPage();
    fillResetPasswordForm();
    fireEvent.click(submitButton());

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        "http://apas.mpaxos.com/auth/reset-password",
        {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ token: "reset-token", password: "secret-pass" }),
        },
      );
    });
    expect(await screen.findByText("Password Reset!")).toBeTruthy();
    expect(screen.getByText("Your password has been successfully reset.")).toBeTruthy();
    expect(setTimeoutSpy.mock.calls.some(([, timeout]) => timeout === 3000)).toBe(true);
    expect(navigation.push).toHaveBeenCalledWith("/login");
  });

  it("renders reset-password server errors", async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse({ message: "Reset token expired" }, false));

    await renderResetPasswordPage();
    fillResetPasswordForm();
    fireEvent.click(submitButton());

    expect(await screen.findByText("Reset token expired")).toBeTruthy();
    expect(navigation.push).not.toHaveBeenCalled();
  });
});
