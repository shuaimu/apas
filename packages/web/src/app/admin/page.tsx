"use client";

import { useStore } from "@/lib/store";
import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { ArrowLeft, Users, FolderOpen, Monitor, Share2, Activity } from "lucide-react";

const API_URL = process.env.NEXT_PUBLIC_API_URL || "http://apas.mpaxos.com:8080";
const ADMIN_USER_ID = "88b6016d-a8b4-400c-bdc9-f0120504a4fc";

interface SystemStats {
  total_users: number;
  recent_users_7d: number;
  total_sessions: number;
  active_sessions_24h: number;
  total_cli_clients: number;
  online_cli_clients: number;
  total_shares: number;
  recent_users: { email: string; created_at: string | null }[];
  sessions_per_day: { date: string; count: number }[];
}

function StatCard({ icon: Icon, label, value, subValue }: {
  icon: React.ElementType;
  label: string;
  value: number | string;
  subValue?: string;
}) {
  return (
    <div className="bg-white dark:bg-gray-800 rounded-lg p-4 shadow-sm border border-gray-200 dark:border-gray-700">
      <div className="flex items-center gap-3">
        <div className="p-2 bg-blue-100 dark:bg-blue-900/30 rounded-lg">
          <Icon className="w-5 h-5 text-blue-600 dark:text-blue-400" />
        </div>
        <div>
          <div className="text-2xl font-bold">{value}</div>
          <div className="text-sm text-gray-500 dark:text-gray-400">{label}</div>
          {subValue && (
            <div className="text-xs text-gray-400 dark:text-gray-500">{subValue}</div>
          )}
        </div>
      </div>
    </div>
  );
}

export default function AdminPage() {
  const { token, userId, isAuthenticated } = useStore();
  const router = useRouter();
  const [stats, setStats] = useState<SystemStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    // Check if user is admin
    if (!isAuthenticated) {
      router.push("/login");
      return;
    }

    if (userId !== ADMIN_USER_ID) {
      router.push("/");
      return;
    }

    // Fetch stats
    const fetchStats = async () => {
      try {
        const response = await fetch(`${API_URL}/admin/stats`, {
          headers: {
            Authorization: `Bearer ${token}`,
          },
        });

        if (!response.ok) {
          if (response.status === 401 || response.status === 403) {
            setError("Access denied");
            return;
          }
          throw new Error("Failed to fetch stats");
        }

        const data = await response.json();
        setStats(data);
      } catch (err) {
        setError(err instanceof Error ? err.message : "Failed to load stats");
      } finally {
        setLoading(false);
      }
    };

    fetchStats();
  }, [token, userId, isAuthenticated, router]);

  if (!isAuthenticated || userId !== ADMIN_USER_ID) {
    return null;
  }

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900">
      <div className="max-w-6xl mx-auto p-6">
        {/* Header */}
        <div className="flex items-center gap-4 mb-6">
          <button
            onClick={() => router.push("/")}
            className="p-2 hover:bg-gray-200 dark:hover:bg-gray-800 rounded-lg"
          >
            <ArrowLeft className="w-5 h-5" />
          </button>
          <h1 className="text-2xl font-bold">System Dashboard</h1>
        </div>

        {loading ? (
          <div className="flex items-center justify-center py-12">
            <div className="animate-spin w-8 h-8 border-4 border-blue-500 border-t-transparent rounded-full" />
          </div>
        ) : error ? (
          <div className="text-center py-12 text-red-500">{error}</div>
        ) : stats ? (
          <div className="space-y-6">
            {/* Stats Grid */}
            <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
              <StatCard
                icon={Users}
                label="Total Users"
                value={stats.total_users}
                subValue={`+${stats.recent_users_7d} this week`}
              />
              <StatCard
                icon={FolderOpen}
                label="Total Sessions"
                value={stats.total_sessions}
                subValue={`${stats.active_sessions_24h} active (24h)`}
              />
              <StatCard
                icon={Monitor}
                label="CLI Clients"
                value={stats.total_cli_clients}
                subValue={`${stats.online_cli_clients} online now`}
              />
              <StatCard
                icon={Share2}
                label="Session Shares"
                value={stats.total_shares}
              />
            </div>

            {/* Recent Activity */}
            <div className="grid md:grid-cols-2 gap-6">
              {/* Recent Users */}
              <div className="bg-white dark:bg-gray-800 rounded-lg p-4 shadow-sm border border-gray-200 dark:border-gray-700">
                <h2 className="font-semibold mb-3 flex items-center gap-2">
                  <Users className="w-4 h-4" />
                  Recent Users
                </h2>
                <div className="space-y-2">
                  {stats.recent_users.map((user, i) => (
                    <div
                      key={i}
                      className="flex items-center justify-between py-2 border-b border-gray-100 dark:border-gray-700 last:border-0"
                    >
                      <span className="text-sm truncate">{user.email}</span>
                      <span className="text-xs text-gray-400">
                        {user.created_at
                          ? new Date(user.created_at).toLocaleDateString()
                          : "N/A"}
                      </span>
                    </div>
                  ))}
                </div>
              </div>

              {/* Sessions per Day */}
              <div className="bg-white dark:bg-gray-800 rounded-lg p-4 shadow-sm border border-gray-200 dark:border-gray-700">
                <h2 className="font-semibold mb-3 flex items-center gap-2">
                  <Activity className="w-4 h-4" />
                  Sessions (Last 14 Days)
                </h2>
                <div className="space-y-2">
                  {stats.sessions_per_day.length === 0 ? (
                    <div className="text-sm text-gray-400">No data</div>
                  ) : (
                    stats.sessions_per_day.map((day, i) => (
                      <div
                        key={i}
                        className="flex items-center gap-3 py-1"
                      >
                        <span className="text-xs text-gray-400 w-20">{day.date}</span>
                        <div className="flex-1 bg-gray-100 dark:bg-gray-700 rounded-full h-4 overflow-hidden">
                          <div
                            className="bg-blue-500 h-full rounded-full"
                            style={{
                              width: `${Math.min(100, (day.count / Math.max(...stats.sessions_per_day.map(d => d.count))) * 100)}%`
                            }}
                          />
                        </div>
                        <span className="text-sm font-medium w-8 text-right">{day.count}</span>
                      </div>
                    ))
                  )}
                </div>
              </div>
            </div>
          </div>
        ) : null}
      </div>
    </div>
  );
}
