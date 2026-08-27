import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useStore } from "@/lib/store";
import ClusterInvitationPage from "./page";

const replace = vi.hoisted(() => vi.fn());
vi.mock("next/navigation", () => ({
  useParams: () => ({ token: "invite-token" }),
  useRouter: () => ({ replace, push: vi.fn() }),
}));

const initialStore = useStore.getState();

beforeEach(() => {
  replace.mockReset();
});

afterEach(() => {
  vi.unstubAllGlobals();
  localStorage.clear();
  act(() => useStore.setState(initialStore, true));
});

describe("shared cluster invitation", () => {
  it("preserves the invitation route across login", async () => {
    act(() => useStore.setState({ token: null }));
    render(<ClusterInvitationPage />);
    await waitFor(() => expect(replace).toHaveBeenCalledWith(
      "/login?redirect=%2Fcluster-invitations%2Finvite-token",
    ));
  });

  it("requires trust confirmation and accepts as the addressed account", async () => {
    act(() => useStore.setState({ token: "auth-token" }));
    const fetchMock = vi.fn().mockImplementation(async (_input: string | URL, init?: RequestInit) => {
      if (init?.method === "POST") {
        return { ok: true, json: async () => ({ status: "active" }) };
      }
      return {
        ok: true,
        json: async () => ({
          invitation: {
            cluster_owner_email: "owner@example.com",
            invitee_email: "member@example.com",
            expires_at: "2026-09-01T00:00:00Z",
            status: "pending",
          },
          trust_warning: "The owner can access processes and credentials.",
        }),
      };
    });
    vi.stubGlobal("fetch", fetchMock);
    render(<ClusterInvitationPage />);

    expect(await screen.findByText("owner@example.com")).toBeTruthy();
    const join = screen.getByRole("button", { name: "Join cluster" }) as HTMLButtonElement;
    expect(join.disabled).toBe(true);
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: "Join cluster" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith(
      "https://apas.mpaxos.com/cluster/invitation-links/invite-token/accept",
      expect.objectContaining({ method: "POST", body: JSON.stringify({ trust_confirmed: true }) }),
    ));
    expect(replace).toHaveBeenCalledWith("/machines");
  });
});
