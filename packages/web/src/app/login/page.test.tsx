import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { AnchorHTMLAttributes, ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import LoginPage from "./page";

type LinkProps = AnchorHTMLAttributes<HTMLAnchorElement> & {
  href: string;
  children: ReactNode;
};

const navigation = vi.hoisted(() => {
  const state = {
    code: null as string | null,
    push: vi.fn(),
    searchParams: {
      get: vi.fn(),
    },
  };
  state.searchParams.get.mockImplementation((key: string) =>
    key === "code" ? state.code : null,
  );
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

function submitButton(name: string | RegExp = "Sign in"): HTMLButtonElement {
  return screen.getByRole("button", { name }) as HTMLButtonElement;
}

function fillLoginForm() {
  fireEvent.change(emailInput(), { target: { value: "user@example.com" } });
  fireEvent.change(passwordInput(), { target: { value: "secret-pass" } });
}

function jsonResponse(body: unknown, ok = true) {
  return {
    ok,
    json: vi.fn().mockResolvedValue(body),
  };
}

function mockLoginSuccess(overrides: Record<string, unknown> = {}) {
  fetchMock.mockResolvedValueOnce(
    jsonResponse({
      token: "token-123",
      user_id: "user-1",
      user_email: "user@example.com",
      ...overrides,
    }),
  );
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

describe("LoginPage", () => {
  beforeEach(() => {
    navigation.code = null;
    navigation.push.mockReset();
    navigation.searchParams.get.mockClear();
    store.login.mockReset();
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

  it("stores returned auth data and redirects after a successful login", async () => {
    mockLoginSuccess();

    render(<LoginPage />);
    fillLoginForm();
    fireEvent.click(submitButton());

    await waitFor(() => {
      expect(store.login).toHaveBeenCalledWith("token-123", "user-1", "user@example.com");
    });
    expect(fetchMock).toHaveBeenCalledWith("http://apas.mpaxos.com:8080/auth/login", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ email: "user@example.com", password: "secret-pass" }),
    });
    expect(navigation.push).toHaveBeenCalledWith("/");
  });

  it("shows loading state and renders login errors from failed responses", async () => {
    const loginAttempt = deferred<ReturnType<typeof jsonResponse>>();
    fetchMock.mockReturnValueOnce(loginAttempt.promise);

    render(<LoginPage />);
    fillLoginForm();
    fireEvent.click(submitButton());

    expect(await screen.findByRole("button", { name: "Signing in..." })).toBeTruthy();

    await act(async () => {
      loginAttempt.resolve(jsonResponse({ message: "Invalid credentials" }, false));
    });

    expect(await screen.findByText("Invalid credentials")).toBeTruthy();
    expect(submitButton().disabled).toBe(false);
    expect(store.login).not.toHaveBeenCalled();
    expect(navigation.push).not.toHaveBeenCalled();
  });

  it("completes CLI device-code login, shows authorization success, and redirects after the timer", async () => {
    const originalSetTimeout = globalThis.setTimeout;
    const setTimeoutSpy = vi
      .spyOn(globalThis, "setTimeout")
      .mockImplementation((callback: TimerHandler, timeout?: number) => {
        if (timeout === 2000 && typeof callback === "function") {
          callback();
          return 0 as unknown as ReturnType<typeof setTimeout>;
        }
        return originalSetTimeout(callback, timeout);
      });
    navigation.code = "CLI123";
    mockLoginSuccess();
    fetchMock.mockResolvedValueOnce(jsonResponse({ success: true }));

    render(<LoginPage />);

    expect(screen.getByText(/authorize your CLI/)).toBeTruthy();
    expect(screen.getByRole("link", { name: "Register" }).getAttribute("href")).toBe(
      "/register?code=CLI123",
    );

    fillLoginForm();
    fireEvent.click(submitButton());

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledTimes(2);
    });
    expect(fetchMock).toHaveBeenLastCalledWith(
      "http://apas.mpaxos.com:8080/auth/device-complete",
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ code: "CLI123", user_id: "user-1" }),
      },
    );
    expect(await screen.findByText("CLI Authorized!")).toBeTruthy();
    expect(setTimeoutSpy.mock.calls.some(([, timeout]) => timeout === 2000)).toBe(true);
    expect(navigation.push).toHaveBeenCalledWith("/");
  });

  it("falls back to the dashboard when device-code completion fails", async () => {
    navigation.code = "CLI123";
    mockLoginSuccess();
    fetchMock.mockResolvedValueOnce(jsonResponse({ message: "Expired device code" }, false));

    render(<LoginPage />);
    fillLoginForm();
    fireEvent.click(submitButton());

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledTimes(2);
    });
    expect(screen.queryByText("CLI Authorized!")).toBeNull();
    expect(navigation.push).toHaveBeenCalledWith("/");
  });

  it("falls back to the dashboard when device-code completion has a network error", async () => {
    navigation.code = "CLI123";
    mockLoginSuccess();
    fetchMock.mockRejectedValueOnce(new Error("network"));

    render(<LoginPage />);
    fillLoginForm();
    fireEvent.click(submitButton());

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledTimes(2);
    });
    expect(screen.queryByText("CLI Authorized!")).toBeNull();
    expect(navigation.push).toHaveBeenCalledWith("/");
  });
});
