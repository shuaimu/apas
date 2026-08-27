"use client";

import { useEffect, useState } from "react";
import { useParams, useRouter } from "next/navigation";
import { useStore } from "@/lib/store";

const API_URL = process.env.NEXT_PUBLIC_API_URL || "https://apas.mpaxos.com";

interface InvitationInspection {
  invitation: {
    cluster_owner_email: string;
    invitee_email: string;
    expires_at: string;
    status: string;
  };
  trust_warning: string;
}

export default function ClusterInvitationPage() {
  const params = useParams<{ token: string }>();
  const router = useRouter();
  const storeToken = useStore((state) => state.token);
  const [inspection, setInspection] = useState<InvitationInspection | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [joining, setJoining] = useState(false);
  const token = params.token;

  useEffect(() => {
    const authToken = storeToken || localStorage.getItem("apas_token");
    if (!authToken) {
      router.replace(`/login?redirect=${encodeURIComponent(`/cluster-invitations/${token}`)}`);
      return;
    }
    void fetch(`${API_URL}/cluster-invitations/${encodeURIComponent(token)}`, {
      headers: { Authorization: `Bearer ${authToken}` },
    }).then(async (response) => {
      const body = await response.json().catch(() => null);
      if (!response.ok) throw new Error(body?.error || "This invitation is unavailable");
      setInspection(body as InvitationInspection);
    }).catch((cause) => setError(cause instanceof Error ? cause.message : "Invitation lookup failed"));
  }, [router, storeToken, token]);

  async function accept() {
    const authToken = storeToken || localStorage.getItem("apas_token");
    if (!authToken || !confirmed) return;
    setJoining(true);
    setError(null);
    try {
      const response = await fetch(`${API_URL}/cluster-invitations/${encodeURIComponent(token)}/accept`, {
        method: "POST",
        headers: { "Content-Type": "application/json", Authorization: `Bearer ${authToken}` },
        body: JSON.stringify({ trust_confirmed: true }),
      });
      const body = await response.json().catch(() => null);
      if (!response.ok) throw new Error(body?.error || "Could not join this cluster");
      router.replace("/machines");
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not join this cluster");
    } finally {
      setJoining(false);
    }
  }

  return (
    <main className="flex min-h-screen items-center justify-center bg-gray-50 p-4 dark:bg-gray-950">
      <section className="w-full max-w-lg rounded-xl border border-gray-200 bg-white p-6 shadow-sm dark:border-gray-800 dark:bg-gray-900">
        <h1 className="text-xl font-semibold">Join a shared cluster</h1>
        {error && <div className="mt-4 rounded border border-red-300 bg-red-50 p-3 text-sm text-red-700 dark:border-red-800 dark:bg-red-950/40 dark:text-red-300">{error}</div>}
        {!inspection && !error && <p className="mt-4 text-sm text-gray-500">Checking invitation…</p>}
        {inspection && (
          <div className="mt-4 space-y-4">
            <div className="rounded bg-gray-50 p-3 text-sm dark:bg-gray-800">
              <div>Cluster owner: <strong>{inspection.invitation.cluster_owner_email}</strong></div>
              <div className="mt-1 text-xs text-gray-500">Addressed to {inspection.invitation.invitee_email} · expires {new Date(inspection.invitation.expires_at).toLocaleString()}</div>
            </div>
            <label className="flex items-start gap-2 rounded border border-amber-300 bg-amber-50 p-3 text-sm text-amber-900 dark:border-amber-700 dark:bg-amber-950/30 dark:text-amber-100">
              <input type="checkbox" checked={confirmed} onChange={(event) => setConfirmed(event.target.checked)} className="mt-1" />
              <span>{inspection.trust_warning}</span>
            </label>
            <div className="flex justify-end gap-2">
              <button onClick={() => router.push("/")} className="rounded border border-gray-300 px-3 py-2 text-sm dark:border-gray-700">Cancel</button>
              <button onClick={() => void accept()} disabled={!confirmed || joining || inspection.invitation.status !== "pending"} className="rounded bg-blue-600 px-3 py-2 text-sm text-white disabled:opacity-50">{joining ? "Joining…" : "Join cluster"}</button>
            </div>
          </div>
        )}
      </section>
    </main>
  );
}
