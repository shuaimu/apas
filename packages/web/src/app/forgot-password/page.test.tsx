import { act, fireEvent, render, screen } from "@testing-library/react";
import type { AnchorHTMLAttributes, ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ForgotPasswordPage from "./page";

type LinkProps = AnchorHTMLAttributes<HTMLAnchorElement> & {
  href: string;
  children: ReactNode;
};

vi.mock("next/link", () => ({
  default: ({ href, children, ...props }: LinkProps) => (
    <a href={href} {...props}>
      {children}
    </a>
  ),
}));

const originalFetch = globalThis.fetch;
const fetchMock = vi.fn();

function emailInput(): HTMLInputElement {
  return screen.getByLabelText("Email") as HTMLInputElement;
}

function submitButton(
  name: string | RegExp = "Send reset link",
): HTMLButtonElement {
  return screen.getByRole("button", { name }) as HTMLButtonElement;
}

function fillForgotPasswordForm(email = "user@example.com") {
  fireEvent.change(emailInput(), { target: { value: email } });
}

function jsonResponse(body: unknown, ok = true) {
  return {
    ok,
    json: vi.fn().mockResolvedValue(body),
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

describe("ForgotPasswordPage", () => {
  beforeEach(() => {
    fetchMock.mockReset();
    globalThis.fetch = fetchMock as unknown as typeof fetch;
  });

  afterEach(() => {
    vi.restoreAllMocks();
    globalThis.fetch = originalFetch;
    document.body.innerHTML = "";
  });

  it("posts the reset request and renders the success email state", async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse({ success: true }));

    render(<ForgotPasswordPage />);
    fillForgotPasswordForm();
    fireEvent.click(submitButton());

    expect(await screen.findByText("Check your email")).toBeTruthy();
    expect(screen.getByText(/sent you a password reset link/)).toBeTruthy();
    expect(screen.getByRole("link", { name: "Back to login" }).getAttribute("href")).toBe(
      "/login",
    );
    expect(fetchMock).toHaveBeenCalledWith(
      "https://apas.mpaxos.com/auth/forgot-password",
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ email: "user@example.com" }),
      },
    );
  });

  it("shows loading state and renders forgot-password server errors", async () => {
    const resetRequest = deferred<ReturnType<typeof jsonResponse>>();
    fetchMock.mockReturnValueOnce(resetRequest.promise);

    render(<ForgotPasswordPage />);
    fillForgotPasswordForm();
    fireEvent.click(submitButton());

    const loadingButton = await screen.findByRole("button", { name: "Sending..." });
    expect((loadingButton as HTMLButtonElement).disabled).toBe(true);

    await act(async () => {
      resetRequest.resolve(jsonResponse({ message: "No account found" }, false));
    });

    expect(await screen.findByText("No account found")).toBeTruthy();
    expect(submitButton().disabled).toBe(false);
  });

  it("renders fallback copy for forgot-password network errors", async () => {
    fetchMock.mockRejectedValueOnce("network");

    render(<ForgotPasswordPage />);
    fillForgotPasswordForm();
    fireEvent.click(submitButton());

    expect(await screen.findByText("Failed to send reset email")).toBeTruthy();
  });
});
