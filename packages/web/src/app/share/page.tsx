"use client";

import { useState, useEffect, Suspense } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import Link from "next/link";

const API_URL = process.env.NEXT_PUBLIC_API_URL || "http://apas.mpaxos.com:8080";

function ShareRedeemForm() {
  const router = useRouter();
  const searchParams = useSearchParams();

  const [code, setCode] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [success, setSuccess] = useState(false);
  const [sessionId, setSessionId] = useState<string | null>(null);

  useEffect(() => {
    // Check for code in URL
    const urlCode = searchParams.get("code");
    if (urlCode) {
      setCode(urlCode);
      // Auto-submit if logged in
      const token = localStorage.getItem("apas_token");
      if (token) {
        handleRedeem(urlCode, token);
      }
    }
  }, [searchParams]);

  const handleRedeem = async (redeemCode: string, token?: string) => {
    setError(null);
    setLoading(true);

    const authToken = token || localStorage.getItem("apas_token");
    if (!authToken) {
      // Redirect to login with return URL
      router.push(`/login?redirect=/share?code=${redeemCode}`);
      return;
    }

    try {
      const res = await fetch(`${API_URL}/share/redeem`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Authorization: `Bearer ${authToken}`,
        },
        body: JSON.stringify({ code: redeemCode }),
      });

      const data = await res.json();

      if (!res.ok) {
        throw new Error(data.message || "Failed to redeem code");
      }

      if (data.success) {
        setSuccess(true);
        setSessionId(data.session_id);
        // Redirect to session after a moment
        setTimeout(() => {
          router.push("/");
        }, 2000);
      } else {
        setError(data.message || "Failed to redeem code");
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to redeem code");
    } finally {
      setLoading(false);
    }
  };

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    handleRedeem(code);
  };

  if (success) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-gray-50 dark:bg-gray-900">
        <div className="max-w-md w-full p-8 bg-white dark:bg-gray-800 rounded-lg shadow-lg text-center">
          <div className="text-6xl mb-4">&#9989;</div>
          <h1 className="text-2xl font-bold text-green-600 dark:text-green-400 mb-2">
            Session Shared!
          </h1>
          <p className="text-gray-600 dark:text-gray-400">
            The session has been added to your project list.
          </p>
          <p className="text-sm text-gray-500 dark:text-gray-500 mt-4">
            Redirecting to dashboard...
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen flex items-center justify-center bg-gray-50 dark:bg-gray-900">
      <div className="max-w-md w-full p-8 bg-white dark:bg-gray-800 rounded-lg shadow-lg">
        <div className="text-center mb-8">
          <h1 className="text-3xl font-bold text-gray-900 dark:text-white">
            APAS
          </h1>
          <p className="text-gray-600 dark:text-gray-400 mt-2">
            Enter invitation code to access a shared session
          </p>
        </div>

        <form onSubmit={handleSubmit} className="space-y-6">
          {error && (
            <div className="p-3 bg-red-50 dark:bg-red-900/30 border border-red-200 dark:border-red-800 rounded-lg">
              <p className="text-sm text-red-600 dark:text-red-400">{error}</p>
            </div>
          )}

          <div>
            <label
              htmlFor="code"
              className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1"
            >
              Invitation Code
            </label>
            <input
              id="code"
              type="text"
              value={code}
              onChange={(e) => setCode(e.target.value.toUpperCase())}
              required
              maxLength={8}
              className="w-full px-4 py-3 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-2 focus:ring-cyan-500 focus:border-transparent font-mono text-2xl tracking-wider text-center uppercase"
              placeholder="XXXXXXXX"
            />
          </div>

          <button
            type="submit"
            disabled={loading || code.length < 8}
            className="w-full py-2 px-4 bg-cyan-600 hover:bg-cyan-700 disabled:bg-cyan-400 text-white font-medium rounded-lg transition-colors"
          >
            {loading ? "Redeeming..." : "Redeem Code"}
          </button>
        </form>

        <p className="mt-6 text-center text-sm text-gray-600 dark:text-gray-400">
          <Link href="/" className="text-cyan-600 hover:text-cyan-500 font-medium">
            Back to Dashboard
          </Link>
        </p>
      </div>
    </div>
  );
}

export default function SharePage() {
  return (
    <Suspense
      fallback={
        <div className="min-h-screen flex items-center justify-center bg-gray-50 dark:bg-gray-900">
          <div className="text-gray-500">Loading...</div>
        </div>
      }
    >
      <ShareRedeemForm />
    </Suspense>
  );
}
