import { useState } from "react";
import { FlatList, View } from "react-native";
import { router, useLocalSearchParams } from "expo-router";
import { useInfiniteQuery } from "@tanstack/react-query";
import { ErrorNotice, Screen, SectionTitle } from "@/components/ui";
import { useJobs, purgeJobAccess } from "./runtime";
import { jobsApi } from "./api";
import { ApiError } from "@/api/client";
import { Copy, JobAction, styles } from "./ui";

export default function JobsScreen() {
  const params = useLocalSearchParams<{ attention?: string }>();
  const [attention, setAttention] = useState(params.attention === "1");
  const state = useJobs();
  const online = state.online && state.foreground && state.authorized;
  const query = useInfiniteQuery({
    queryKey: ["jobs", state.account, "inbox", attention], initialPageParam: null as string | null,
    queryFn: async ({ pageParam, signal }) => {
      try { return await jobsApi.list(pageParam, attention, signal); }
      catch (error) { if (error instanceof ApiError && [401, 403].includes(error.status)) await purgeJobAccess(); throw error; }
    },
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    enabled: Boolean(state.account && online), retry: false, maxPages: 5,
  });
  const jobs = query.data?.pages.flatMap((page) => page.jobs) ?? state.inbox.jobs;
  const visible = attention ? jobs.filter((job) => job.attention_count > 0) : jobs;
  return <Screen><FlatList
    contentContainerStyle={styles.page} data={visible} keyExtractor={(job) => job.id}
    refreshing={query.isRefetching} onRefresh={() => { if (online) void query.refetch(); }}
    ListHeaderComponent={<View style={{ gap: 12 }}><SectionTitle>Earlier finite-job results</SectionTitle><Copy>Read-only original instructions, execution history and outcomes. New recurring work requires separate worker approval.</Copy><JobAction onPress={() => router.push("/(code)/workers")}>Workers and Attention</JobAction><JobAction selected={attention} onPress={() => setAttention(!attention)}>{attention ? "Show all earlier results" : "Earlier decisions and outcomes"}</JobAction><JobAction onPress={() => router.push("/(code)/(tabs)")}>Coding sessions</JobAction>{!online && <ErrorNotice message="Cached earlier jobs · offline or synchronizing · read-only" />}{state.unavailable.map((reason) => <ErrorNotice key={reason} message={reason} />)}{query.error && <ErrorNotice message={String(query.error)} />}</View>}
    renderItem={({ item }) => <View style={styles.card}><JobAction onPress={() => router.push({ pathname: "/(code)/jobs/[jobId]", params: { jobId: item.id, project: item.project_id } })}>{item.title}{"\n"}{item.project_name} · {item.state.replaceAll("_", " ")} · {item.attention_count} attention</JobAction>{item.reason && <Copy>{item.reason}</Copy>}<Copy>Last confirmed update: {item.updated_at}</Copy></View>}
    ListEmptyComponent={<Copy>No authorized jobs in this view.</Copy>}
    ListFooterComponent={query.hasNextPage ? <JobAction disabled={!online || query.isFetchingNextPage} onPress={() => { void query.fetchNextPage(); }}>More jobs</JobAction> : null}
  /></Screen>;
}
