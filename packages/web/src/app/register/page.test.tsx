import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { AnchorHTMLAttributes, ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import RegisterPage from "./page";

type LinkProps = AnchorHTMLAttributes<HTMLAnchorElement> & {
  href: string;
  children: ReactNode;
};

const navigation = vi.hoisted(() => {
  const state = {
    code: null as string | null,
    invitation: null as string | null,
    push: vi.fn(),
    searchParams: {
      get: vi.fn(),
    },
  };
  // One implementation for the whole file, reading mutable fields. A test that
  // swapped this out instead would leak its query string into every test after
  // it, because beforeEach clears calls but not implementations.
  state.searchParams.get.mockImplementation((key: string) => {
    if (key === "code") return state.code;
    if (key === "invitation") return state.invitation;
    return null;
  });
  return state;
});

const store = vi.hoisted(() => ({
  login: vi.fn(),
}));

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

vi.mock("@/lib/store", () => ({
  useStore: (selector: (state: { login: typeof store.login }) => unknown) =>
    selector({ login: store.login }),
}));

const originalFetch = globalThis.fetch;
const fetchMock = vi.fn();

function emailInput(): HTMLInputElement {
  return screen.getByLabelText("Email") as HTMLInputElement;
}

function passwordInput(): HTMLInputElement {
  return screen.getByLabelText("Password") as HTMLInputElement;
}

function confirmPasswordInput(): HTMLInputElement {
  return screen.getByLabelText("Confirm Password") as HTMLInputElement;
}

function submitButton(
  name: string | RegExp = "Create account",
): HTMLButtonElement {
  return screen.getByRole("button", { name }) as HTMLButtonElement;
}

function submitForm() {
  const form = submitButton().closest("form");
  if (!form) {
    throw new Error("Register form was not rendered");
  }
  fireEvent.submit(form);
}

function fillRegisterForm({
  email = "user@example.com",
  password = "secret-pass",
  confirmPassword = password,
}: {
  email?: string;
  password?: string;
  confirmPassword?: string;
} = {}) {
  fireEvent.change(emailInput(), { target: { value: email } });
  fireEvent.change(passwordInput(), { target: { value: password } });
  fireEvent.change(confirmPasswordInput(), { target: { value: confirmPassword } });
}

function jsonResponse(body: unknown, ok = true) {
  return {
    ok,
    json: vi.fn().mockResolvedValue(body),
  };
}

function mockRegisterSuccess(overrides: Record<string, unknown> = {}) {
  fetchMock.mockResolvedValueOnce(
    jsonResponse({
      token: "token-123",
      user_id: "user-1",
      user_email: "registered@example.com",
      ...overrides,
    }),
  );
}

/// The page asks the server who may register, on mount, before any submit.
/// Queue that answer first so each test's own queued response still lines up
/// with the register call it is about.
function mockRegistrationPolicy(
  domains: string[] = ["cs.stonybrook.edu", "stonybrook.edu"],
  minPasswordLength = 8,
) {
  fetchMock.mockResolvedValueOnce(
    jsonResponse({
      self_signup_email_domains: domains,
      min_password_length: minPasswordLength,
    }),
  );
}

function registerCalls() {
  return fetchMock.mock.calls.filter(
    (call) => typeof call[0] === "string" && call[0].includes("/auth/register") &&
      !call[0].includes("registration-policy"),
  );
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

describe("RegisterPage", () => {
  beforeEach(() => {
    navigation.code = null;
    navigation.invitation = null;
    navigation.push.mockReset();
    navigation.searchParams.get.mockClear();
    store.login.mockReset();
    fetchMock.mockReset();
    globalThis.fetch = fetchMock as unknown as typeof fetch;
    mockRegistrationPolicy();
    vi.useRealTimers();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.useRealTimers();
    globalThis.fetch = originalFetch;
    document.body.innerHTML = "";
  });

  it("rejects mismatched passwords before calling fetch", async () => {
    render(<RegisterPage />);
    fillRegisterForm({ password: "secret-pass", confirmPassword: "different-pass" });
    submitForm();

    expect(await screen.findByText("Passwords do not match")).toBeTruthy();
    expect(registerCalls()).toHaveLength(0);
    expect(store.login).not.toHaveBeenCalled();
    expect(navigation.push).not.toHaveBeenCalled();
  });

  it("rejects short passwords before calling fetch", async () => {
    render(<RegisterPage />);
    fillRegisterForm({ password: "short", confirmPassword: "short" });
    submitForm();

    expect(
      await screen.findByText("Password must be at least 8 characters"),
    ).toBeTruthy();
    expect(registerCalls()).toHaveLength(0);
    expect(store.login).not.toHaveBeenCalled();
    expect(navigation.push).not.toHaveBeenCalled();
  });

  it("tells a visitor which domains may sign up, reading them from the server", async () => {
    render(<RegisterPage />);

    expect(
      await screen.findByText(/Open registration is currently limited to/),
    ).toBeTruthy();
    expect(
      screen.getByText("@cs.stonybrook.edu or @stonybrook.edu"),
    ).toBeTruthy();
    // The way in for everyone else has to be stated too, or an ineligible
    // visitor is simply told no.
    expect(screen.getByText(/Ask an administrator for an invitation/)).toBeTruthy();
  });

  it("says an invitation is required when the deployment lists no domains", async () => {
    fetchMock.mockReset();
    mockRegistrationPolicy([]);
    render(<RegisterPage />);

    expect(
      await screen.findByText(/Signing up requires an invitation/),
    ).toBeTruthy();
    expect(screen.queryByText(/currently limited to/)).toBeNull();
  });

  it("warns as soon as an ineligible domain is typed, and not before", async () => {
    render(<RegisterPage />);
    await screen.findByText(/Open registration is currently limited to/);

    // Mid-typing: no accusation while the domain is still incomplete.
    fireEvent.change(emailInput(), { target: { value: "someone@gmail" } });
    expect(screen.queryByText(/cannot sign up directly/)).toBeNull();

    fireEvent.change(emailInput(), { target: { value: "someone@gmail.com" } });
    expect(
      await screen.findByText(/@gmail\.com addresses cannot sign up directly/),
    ).toBeTruthy();

    // An accepted domain clears it.
    fireEvent.change(emailInput(), {
      target: { value: "someone@cs.stonybrook.edu" },
    });
    await waitFor(() => {
      expect(screen.queryByText(/cannot sign up directly/)).toBeNull();
    });
  });

  it("does not warn about the domain when an invitation is present", async () => {
    navigation.invitation = "invite-code-abc";
    render(<RegisterPage />);

    expect(
      await screen.findByText(/You are signing up with an invitation/),
    ).toBeTruthy();
    fireEvent.change(emailInput(), { target: { value: "someone@gmail.com" } });
    expect(screen.queryByText(/cannot sign up directly/)).toBeNull();
    expect(screen.queryByText(/currently limited to/)).toBeNull();
  });

  it("shows the server's reason for refusing, not a generic failure", async () => {
    fetchMock.mockResolvedValueOnce(
      jsonResponse({ error: "A cluster invitation is required" }, false),
    );

    render(<RegisterPage />);
    fillRegisterForm({ email: "someone@gmail.com" });
    fireEvent.click(submitButton());

    // The API reports failures as `error`; the page used to read only
    // `message`, so every rejection surfaced as "Registration failed".
    expect(
      await screen.findByText("A cluster invitation is required"),
    ).toBeTruthy();
    expect(screen.queryByText("Registration failed")).toBeNull();
  });

  it("says nothing about domains when the server is too old to report a policy", async () => {
    fetchMock.mockReset();
    fetchMock.mockRejectedValueOnce(new Error("404"));
    render(<RegisterPage />);

    await waitFor(() => {
      expect(screen.getByLabelText("Email")).toBeTruthy();
    });
    // No claim is better than a wrong claim.
    expect(screen.queryByText(/currently limited to/)).toBeNull();
    expect(screen.queryByText(/Signing up requires an invitation/)).toBeNull();
  });

  it("stores returned auth data and redirects after successful registration", async () => {
    mockRegisterSuccess();

    render(<RegisterPage />);
    fillRegisterForm();
    fireEvent.click(submitButton());

    await waitFor(() => {
      expect(store.login).toHaveBeenCalledWith(
        "token-123",
        "user-1",
        "registered@example.com",
      );
    });
    expect(fetchMock).toHaveBeenCalledWith("https://apas.mpaxos.com/auth/register", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ email: "user@example.com", password: "secret-pass" }),
    });
    expect(navigation.push).toHaveBeenCalledWith("/");
  });

  it("shows loading state and renders registration errors from failed responses", async () => {
    const registerAttempt = deferred<ReturnType<typeof jsonResponse>>();
    fetchMock.mockReturnValueOnce(registerAttempt.promise);

    render(<RegisterPage />);
    fillRegisterForm();
    fireEvent.click(submitButton());

    const loadingButton = await screen.findByRole("button", {
      name: "Creating account...",
    });
    expect((loadingButton as HTMLButtonElement).disabled).toBe(true);

    await act(async () => {
      registerAttempt.resolve(jsonResponse({ message: "Email already exists" }, false));
    });

    expect(await screen.findByText("Email already exists")).toBeTruthy();
    expect(submitButton().disabled).toBe(false);
    expect(store.login).not.toHaveBeenCalled();
    expect(navigation.push).not.toHaveBeenCalled();
  });

  it("completes CLI device-code registration, shows authorization success, and redirects after the timer", async () => {
    const originalSetTimeout = globalThis.setTimeout;
    const setTimeoutSpy = vi
      .spyOn(globalThis, "setTimeout")
      .mockImplementation((callback, timeout, ...args) => {
        if (timeout === 2000) {
          callback(...args);
          return 0 as unknown as ReturnType<typeof setTimeout>;
        }
        return originalSetTimeout(callback, timeout, ...args);
      });
    navigation.code = "CLI123";
    mockRegisterSuccess();
    fetchMock.mockResolvedValueOnce(jsonResponse({ success: true }));

    render(<RegisterPage />);

    expect(screen.getByText(/authorize your CLI/)).toBeTruthy();
    expect(screen.getByRole("link", { name: "Sign in" }).getAttribute("href")).toBe(
      "/login?code=CLI123",
    );

    fillRegisterForm();
    fireEvent.click(submitButton());

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledTimes(2);
    });
    expect(fetchMock).toHaveBeenLastCalledWith(
      "https://apas.mpaxos.com/auth/device-complete",
      {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Authorization: "Bearer token-123",
        },
        body: JSON.stringify({ code: "CLI123", user_id: "user-1" }),
      },
    );
    expect(await screen.findByText("Account Created & CLI Authorized!")).toBeTruthy();
    expect(setTimeoutSpy.mock.calls.some(([, timeout]) => timeout === 2000)).toBe(true);
    expect(navigation.push).toHaveBeenCalledWith("/");
  });

  it("falls back to the dashboard when device-code completion fails", async () => {
    navigation.code = "CLI123";
    mockRegisterSuccess();
    fetchMock.mockResolvedValueOnce(jsonResponse({ message: "Expired device code" }, false));

    render(<RegisterPage />);
    fillRegisterForm();
    fireEvent.click(submitButton());

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledTimes(2);
    });
    expect(screen.queryByText("Account Created & CLI Authorized!")).toBeNull();
    expect(navigation.push).toHaveBeenCalledWith("/");
  });

  it("falls back to the dashboard when device-code completion has a network error", async () => {
    navigation.code = "CLI123";
    mockRegisterSuccess();
    fetchMock.mockRejectedValueOnce(new Error("network"));

    render(<RegisterPage />);
    fillRegisterForm();
    fireEvent.click(submitButton());

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledTimes(2);
    });
    expect(screen.queryByText("Account Created & CLI Authorized!")).toBeNull();
    expect(navigation.push).toHaveBeenCalledWith("/");
  });
});
