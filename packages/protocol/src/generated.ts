/* Generated from Rust JSON Schema. Do not edit by hand. */

export type JobState =
  | "queued"
  | "preparing"
  | "running"
  | "needs_input"
  | "verifying"
  | "publishing"
  | "review_ready"
  | "blocked"
  | "recovery_required"
  | "cancelling"
  | "cancelled"
  | "failed";
/**
 * How a pane hosts its agent process. Orthogonal to [`Provider`] (which
 * binary) and [`PaneMode`] (how autonomous).
 *
 * * [`PaneKind::Agent`] — the legacy/managed-team path: the CLI runs the provider
 *   headlessly (`claude --print --output-format stream-json`,
 *   `codex exec --json`) and parses structured events into
 *   `CliToServer::StreamMessage`. Everything team-mode depends on —
 *   usage counters, pane status, diffs, plan review, scratchpad
 *   publishing, Tech Lead delegation — is built on those events. New
 *   unmanaged panes are no longer created with this kind, but existing panes
 *   remain compatible and managed team panes continue to use it.
 * * [`PaneKind::Terminal`] — the pane instead hosts the provider's real
 *   interactive TUI on a pty. Raw bytes flow over the dedicated
 *   `Terminal*` messages and are rendered by xterm.js in the browser.
 *   Nothing is parsed, so a terminal pane has none of the structured
 *   integrations above and is never a delegation target. This is the normal
 *   kind for new user-created Claude, Codex, and OpenCode panes.
 *
 * `#[serde(default)]` on `PaneConfig::kind` keeps `.apas` files written
 * before this existed deserializing as `Agent`.
 */
export type PaneKind = "agent" | "terminal";
/**
 * Mode for a pane
 */
export type PaneMode = "deadloop" | "interactive";
/**
 * Provider for a pane
 */
export type Provider =
  ("codex" | "deepseek" | "opencode" | "cursor-agent") | "claude" | "minimax" | "glm" | "pi" | "omp";
export type CodeEventKind =
  | "instruction"
  | "agent_status"
  | "tool"
  | "question"
  | "approval"
  | "plan"
  | "todo"
  | "test"
  | "diff"
  | "pull_request"
  | "terminal"
  | "completed"
  | "interrupted"
  | "error";
export type MobilePlatform = "ios" | "android";
export type JobArtifactKind = "candidate" | "diff" | "check_log" | "worker_log" | "verification" | "publication";
export type JobQuestionState =
  "pending" | "answer_accepted" | "answer_delivered" | "confirmed" | "uncertain" | "cancelled";
export type JobEventKind =
  | "admitted"
  | "phase_changed"
  | "progress"
  | "subagent"
  | "question"
  | "answer_confirmed"
  | "usage"
  | "verification"
  | "publication"
  | "cancellation_requested"
  | "stopped"
  | "failed"
  | "recovery_required";
export type JobPolicyDocument = JobPolicy | JobPolicyOverride;
export type JobMutationKind = "admit" | "answer" | "cancel" | "retry" | "retry_publication";
export type JobReceiptStatus = "pending" | "accepted" | "completed" | "rejected";
export type JobCheckStatus = "passed" | "failed" | "timed_out" | "missing" | "cancelled";
export type JobModelApi = "openai_responses" | "anthropic_messages";
export type ServerToWeb =
  | {
      detail: JobDetail;
      type: "job_snapshot";
      [k: string]: unknown;
    }
  | {
      event: JobEvent;
      type: "job_event";
      [k: string]: unknown;
    }
  | {
      job_id: string;
      last_event_seq: number;
      project_id: string;
      state_version: number;
      type: "job_changed";
      [k: string]: unknown;
    }
  | {
      code: string;
      job_id: string;
      message: string;
      project_id: string;
      resync_required: boolean;
      type: "job_subscription_rejected";
      [k: string]: unknown;
    }
  | {
      account_status?: string;
      cluster_role?: string;
      mutations_allowed?: boolean;
      negotiated_capabilities?: string[];
      protocol_version?: number | null;
      server_version?: string | null;
      type: "authenticated";
      user_email?: string | null;
      user_id: string;
      [k: string]: unknown;
    }
  | {
      maximum_version: number;
      message: string;
      minimum_version: number;
      read_only: boolean;
      type: "protocol_incompatible";
      [k: string]: unknown;
    }
  | {
      reason: string;
      type: "authentication_failed";
      [k: string]: unknown;
    }
  | {
      accepted: boolean;
      error?: string | null;
      mutation: MutationKind;
      pane_id?: number | null;
      /**
       * A fresh native bridge check permits an explicit corrected submission
       * after rejection. Missing means the question's state is uncertain.
       */
      question_pending?: boolean | null;
      request_id: string;
      session_id: string;
      type: "mutation_ack";
      [k: string]: unknown;
    }
  | {
      inventory?: CliLifecycleInventory;
      session_id: string;
      type: "cli_lifecycle_inventory";
      [k: string]: unknown;
    }
  | {
      inventory?: CliLifecycleInventory1 | null;
      message?: string | null;
      operation: CliLifecycleOperation;
      phase: CliLifecyclePhase;
      request_id: string;
      session_id: string;
      type: "cli_lifecycle_status";
      [k: string]: unknown;
    }
  | {
      pane_id?: number | null;
      pane_type?: PaneType | null;
      session_id: string;
      type: "session_started";
      [k: string]: unknown;
    }
  | {
      status: SessionStatus;
      type: "session_status";
      [k: string]: unknown;
    }
  | {
      has_active_cli: boolean;
      session_id: string;
      type: "session_attached";
      [k: string]: unknown;
    }
  | {
      message: string;
      reason: SessionAttachmentRejectionReason;
      session_id: string;
      type: "session_attachment_rejected";
      [k: string]: unknown;
    }
  | {
      content: string;
      /**
       * Type of output content
       */
      output_type?:
        | ("text" | "system" | "error")
        | {
            code: {
              language?: string | null;
              [k: string]: unknown;
            };
          }
        | {
            tool_use: {
              input: unknown;
              tool: string;
              [k: string]: unknown;
            };
          }
        | {
            tool_result: {
              success: boolean;
              tool: string;
              [k: string]: unknown;
            };
          }
        | {
            approval_request: {
              description: string;
              tool: string;
              tool_call_id: string;
              [k: string]: unknown;
            };
          };
      pane_id?: number | null;
      pane_type?: PaneType | null;
      session_id?: string | null;
      type: "output";
      [k: string]: unknown;
    }
  | {
      message: string;
      type: "error";
      [k: string]: unknown;
    }
  | {
      clients: CliClientInfo[];
      type: "cli_clients";
      [k: string]: unknown;
    }
  | {
      machines: MachineWithProjects[];
      type: "machines";
      [k: string]: unknown;
    }
  | {
      created_at?: string | null;
      message: ClaudeStreamMessage;
      pane_id?: number | null;
      pane_type?: PaneType | null;
      session_id: string;
      type: "stream_message";
      [k: string]: unknown;
    }
  | {
      sessions: SessionInfo[];
      type: "sessions";
      [k: string]: unknown;
    }
  | {
      change: ProjectAccessChange;
      project_id: string;
      role?: string | null;
      type: "project_access_changed";
      [k: string]: unknown;
    }
  | {
      catchup?: boolean;
      has_more?: boolean;
      messages: MessageInfo[];
      session_id: string;
      type: "session_messages";
      [k: string]: unknown;
    }
  | {
      client_msg_id?: string | null;
      created_at?: string | null;
      pane_id?: number | null;
      pane_type?: PaneType | null;
      session_id: string;
      text: string;
      type: "user_input";
      [k: string]: unknown;
    }
  | {
      client_msg_id?: string | null;
      pane_id: number;
      session_id: string;
      type: "terminal_conversation_recorded";
      [k: string]: unknown;
    }
  | {
      is_paused: boolean;
      session_id: string;
      type: "deadloop_status";
      [k: string]: unknown;
    }
  | {
      is_paused: boolean;
      pane_id: number;
      session_id: string;
      type: "pane_paused";
      [k: string]: unknown;
    }
  | {
      pane_id?: number | null;
      /**
       * Pane type for dual-pane mode (legacy - kept for backward compatibility)
       */
      pane_type?: "deadloop" | "interactive";
      session_id: string;
      status?: string | null;
      type: "pane_status";
      [k: string]: unknown;
    }
  | {
      panes: PaneConfig[];
      session_id: string;
      type: "pane_list";
      [k: string]: unknown;
    }
  | {
      cli_client_id: string;
      limits: UsageLimits;
      /**
       * Provider for a pane
       */
      provider?: ("codex" | "deepseek" | "opencode" | "cursor-agent") | "claude" | "minimax" | "glm" | "pi" | "omp";
      type: "usage_limits";
      [k: string]: unknown;
    }
  | {
      input: unknown;
      pane_id: number;
      session_id: string;
      tool_name: string;
      tool_use_id: string;
      type: "plan_review_request";
      [k: string]: unknown;
    }
  | {
      base?: string | null;
      branch?: string | null;
      diff?: string | null;
      error?: string | null;
      pane_id: number;
      session_id: string;
      type: "pane_diff";
      [k: string]: unknown;
    }
  | {
      error?: string | null;
      pane_id: number;
      session_id: string;
      type: "pr_created";
      url?: string | null;
      [k: string]: unknown;
    }
  | {
      session_id: string;
      stats: ProjectUsageStats;
      type: "project_usage_stats";
      [k: string]: unknown;
    }
  | {
      error?: string | null;
      machine_id: string;
      project_id?: string | null;
      request_id?: string | null;
      type: "project_instance_created";
      [k: string]: unknown;
    }
  | {
      machine_id: string;
      request_id: string;
      result: LocalProjectRegistrationResult;
      type: "local_project_registered";
      [k: string]: unknown;
    }
  | {
      auto_approve_todos: boolean;
      auto_merge_prs: boolean;
      /**
       * Tab types this project refuses to create, as `<kind>:<provider>`
       * keys (see `tab_type_key`). A *deny* list so that absent means
       * "everything allowed" — see `tab_type_allowed`. Owner/admin only,
       * same gate as the flags above.
       */
      disallowed_tab_types?: string[];
      session_id: string;
      /**
       * Drives whether the web shows any team surface at all. The CLI
       * re-broadcasts this every 5s from `.apas`, so a web client that
       * attaches mid-session hydrates without asking.
       */
      team_enabled?: boolean;
      type: "project_flags_changed";
      [k: string]: unknown;
    }
  | {
      noncompliant_pane_ids?: number[];
      policy: EffectiveProjectPolicy;
      session_id: string;
      type: "project_policy_changed";
      [k: string]: unknown;
    }
  | {
      data_b64: string;
      instance_id?: string | null;
      pane_id: number;
      seq: number;
      session_id: string;
      type: "terminal_output";
      [k: string]: unknown;
    }
  | {
      data_b64: string;
      instance_id?: string | null;
      /**
       * Last server-observable state of a pty-hosted terminal pane.
       *
       * `Disconnected` means the CLI transport went away while the terminal was
       * last known to be running; it does not claim the provider process exited.
       * `Unknown` is the rollout-safe default for peers predating lifecycle
       * reconciliation and after a server restart before the CLI reports state.
       */
      lifecycle?: "unknown" | "running" | "disconnected" | "exited";
      pane_id: number;
      runtime?: TerminalRuntimeReconciliation | null;
      seq: number;
      session_id: string;
      status?: string | null;
      truncated?: boolean;
      type: "terminal_snapshot";
      [k: string]: unknown;
    }
  | {
      instance_id?: string | null;
      pane_id: number;
      session_id: string;
      status?: string | null;
      type: "terminal_exited";
      [k: string]: unknown;
    }
  | {
      instance_id?: string | null;
      /**
       * Last server-observable state of a pty-hosted terminal pane.
       *
       * `Disconnected` means the CLI transport went away while the terminal was
       * last known to be running; it does not claim the provider process exited.
       * `Unknown` is the rollout-safe default for peers predating lifecycle
       * reconciliation and after a server restart before the CLI reports state.
       */
      lifecycle?: "unknown" | "running" | "disconnected" | "exited";
      pane_id: number;
      runtime?: TerminalRuntimeReconciliation | null;
      session_id: string;
      status?: string | null;
      type: "terminal_state";
      [k: string]: unknown;
    }
  | {
      /**
       * Generation support reported alongside cached records.
       */
      availability?: "available" | "cli_update_required" | "summarizer_disabled" | "summarizer_unavailable" | "unknown";
      pane_id: number;
      session_id: string;
      summaries?: PaneWorkSummary[];
      type: "pane_work_summaries";
      [k: string]: unknown;
    }
  | {
      /**
       * Generation support reported alongside cached records.
       */
      availability?: "available" | "cli_update_required" | "summarizer_disabled" | "summarizer_unavailable" | "unknown";
      pane_id: number;
      session_id: string;
      summary: PaneWorkSummary;
      type: "pane_work_summary_updated";
      [k: string]: unknown;
    }
  | {
      type: "heartbeat";
      [k: string]: unknown;
    };
export type MutationKind = "approval" | "question" | "plan_review" | "interrupt";
/**
 * Whether one configured pane can retain its exact live process during a
 * full CLI replacement.
 */
export type PanePreservationMode = "live_adoptable" | "restart_required_on_cli_reboot" | "structured_pane_may_resume";
/**
 * A project-level lifecycle operation.
 *
 * Only reboot is a user decision. Transport recovery used to be a second
 * variant here, driven by a `Reconnect Server` button, and was withdrawn: the
 * CLI already re-dials a lost transport on its own with bounded backoff, and a
 * request to do so has to travel over the very transport that is down.
 */
export type CliLifecycleOperation = "reboot_cli";
/**
 * Authoritative progress for a correlated lifecycle request.
 */
export type CliLifecyclePhase =
  "accepted" | "preparing" | "reconnecting" | "handoff" | "reconciling" | "succeeded" | "failed" | "timed_out";
/**
 * Pane type for dual-pane mode (legacy - kept for backward compatibility)
 */
export type PaneType = "deadloop" | "interactive";
/**
 * Session status
 */
export type SessionStatus = "pending" | "connected" | "disconnected" | "ended";
/**
 * Why a requested session attachment was rejected. The session id carried
 * alongside this value lets clients reconcile concurrent foreground and
 * background attachment attempts without treating a global error as the
 * result of whichever navigation happens to be visible.
 */
export type SessionAttachmentRejectionReason =
  "project_access_required" | "host_machine_access_required" | "project_unavailable" | "session_not_found";
/**
 * CLI client status
 */
export type CliClientStatus = "online" | "offline" | "busy";
/**
 * Top-level message from Claude CLI stream-json output
 */
export type ClaudeStreamMessage =
  | {
      cwd?: string | null;
      model?: string;
      session_id: string;
      subtype: string;
      tools?: string[];
      type: "system";
      [k: string]: unknown;
    }
  | {
      message: ClaudeAssistantMessage;
      session_id: string;
      type: "assistant";
      [k: string]: unknown;
    }
  | {
      message: ClaudeUserMessage;
      session_id: string;
      tool_use_result?: {
        [k: string]: unknown;
      };
      type: "user";
      [k: string]: unknown;
    }
  | {
      duration_ms?: number;
      is_error?: boolean;
      result?: string;
      session_id: string;
      subtype: string;
      total_cost_usd?: number;
      type: "result";
      [k: string]: unknown;
    };
/**
 * Content block types in Claude messages
 */
export type ClaudeContentBlock =
  | {
      text: string;
      type: "text";
      [k: string]: unknown;
    }
  | {
      id: string;
      input: unknown;
      name: string;
      type: "tool_use";
      [k: string]: unknown;
    }
  | {
      content: string;
      is_error?: boolean;
      tool_use_id: string;
      type: "tool_result";
      [k: string]: unknown;
    };
/**
 * Messages sent from server to web client
 */
export type ProjectAccessChange = "transferred" | "revoked" | "deleted";
/**
 * A registration cannot simultaneously succeed and carry an error.
 */
export type LocalProjectRegistrationResult =
  | {
      project: MachineProjectInfo;
      status: "registered";
      [k: string]: unknown;
    }
  | {
      error: string;
      status: "failed";
      [k: string]: unknown;
    };
/**
 * Messages sent from web client to server
 */
export type WebToServer =
  | {
      after_seq: number;
      job_id: string;
      project_id: string;
      type: "subscribe_job";
      [k: string]: unknown;
    }
  | {
      job_id: string;
      type: "unsubscribe_job";
      [k: string]: unknown;
    }
  | {
      app_version?: string | null;
      capabilities?: string[];
      client_kind?: ClientKind | null;
      protocol_version?: number | null;
      token: string;
      type: "authenticate";
      [k: string]: unknown;
    }
  | {
      type: "list_cli_clients";
      [k: string]: unknown;
    }
  | {
      type: "list_machines";
      [k: string]: unknown;
    }
  | {
      cli_client_id?: string | null;
      type: "start_session";
      [k: string]: unknown;
    }
  | {
      session_id: string;
      type: "resume_session";
      [k: string]: unknown;
    }
  | {
      session_id: string;
      type: "attach_session";
      [k: string]: unknown;
    }
  | {
      client_msg_id?: string | null;
      pane_id?: number | null;
      pane_type?: PaneType | null;
      session_id?: string | null;
      text: string;
      type: "input";
      [k: string]: unknown;
    }
  | {
      client_msg_id?: string | null;
      pane_id: number;
      session_id: string;
      text: string;
      type: "terminal_conversation_input";
      [k: string]: unknown;
    }
  | {
      pane_id?: number | null;
      request_id?: string | null;
      session_id?: string | null;
      tool_call_id: string;
      type: "approve";
      [k: string]: unknown;
    }
  | {
      pane_id?: number | null;
      request_id?: string | null;
      session_id?: string | null;
      tool_call_id: string;
      type: "reject";
      [k: string]: unknown;
    }
  | {
      session_id?: string | null;
      signal: string;
      type: "signal";
      [k: string]: unknown;
    }
  | {
      type: "list_sessions";
      [k: string]: unknown;
    }
  | {
      after_created_at?: string | null;
      before_id?: string | null;
      limit?: number | null;
      pane_id?: number | null;
      pane_type?: PaneType | null;
      pane_watermarks?: {
        [k: string]: string;
      } | null;
      session_id: string;
      type: "get_session_messages";
      [k: string]: unknown;
    }
  | {
      session_id?: string | null;
      type: "pause_deadloop";
      [k: string]: unknown;
    }
  | {
      session_id?: string | null;
      type: "resume_deadloop";
      [k: string]: unknown;
    }
  | {
      pane_id: number;
      session_id?: string | null;
      type: "pause_pane";
      [k: string]: unknown;
    }
  | {
      pane_id: number;
      session_id?: string | null;
      type: "resume_pane";
      [k: string]: unknown;
    }
  | {
      pane_id: number;
      session_id?: string | null;
      type: "reboot_pane";
      [k: string]: unknown;
    }
  | {
      backstory?: string | null;
      goal?: string | null;
      /**
       * When true, the CLI creates an isolated git worktree under
       * `<project>/.apas-worktrees/pane-<id>` (branch `apas-pane-<id>`)
       * before spawning, and persists the resulting absolute path on
       * PaneConfig.worktree_path. Phase 1.1e.
       */
      isolated_worktree?: boolean;
      /**
       * How a pane hosts its agent process. Orthogonal to [`Provider`] (which
       * binary) and [`PaneMode`] (how autonomous).
       *
       * * [`PaneKind::Agent`] — the legacy/managed-team path: the CLI runs the provider
       *   headlessly (`claude --print --output-format stream-json`,
       *   `codex exec --json`) and parses structured events into
       *   `CliToServer::StreamMessage`. Everything team-mode depends on —
       *   usage counters, pane status, diffs, plan review, scratchpad
       *   publishing, Tech Lead delegation — is built on those events. New
       *   unmanaged panes are no longer created with this kind, but existing panes
       *   remain compatible and managed team panes continue to use it.
       * * [`PaneKind::Terminal`] — the pane instead hosts the provider's real
       *   interactive TUI on a pty. Raw bytes flow over the dedicated
       *   `Terminal*` messages and are rendered by xterm.js in the browser.
       *   Nothing is parsed, so a terminal pane has none of the structured
       *   integrations above and is never a delegation target. This is the normal
       *   kind for new user-created Claude, Codex, and OpenCode panes.
       *
       * `#[serde(default)]` on `PaneConfig::kind` keeps `.apas` files written
       * before this existed deserializing as `Agent`.
       */
      kind?: "agent" | "terminal";
      label?: string | null;
      /**
       * v3.5 — true when this pane is being added as part of the
       * project team, false for ordinary user-created terminal work. See
       * PaneConfig::managed.
       */
      managed?: boolean;
      mode: PaneMode;
      model?: string | null;
      /**
       * Phase 3.2a: per-pane policy for the "editable plan checkpoint"
       * feature. The streaming worker reads this at every turn to decide
       * whether to gate the first tool_use behind a user-approval card.
       */
      plan_review_mode?: "always" | "risky_only" | "never";
      prompt?: string | null;
      provider: Provider;
      /**
       * Initial role/goal/backstory/plan_review_mode applied to the new
       * pane BEFORE the first spawn — so a templated worker (Add Worker
       * modal on Overview) uses the right system prompt immediately
       * instead of needing a close+reopen. All optional; missing fields
       * keep the legacy "set via Role modal later" path.
       */
      role?: string | null;
      session_id?: string | null;
      type: "add_pane";
      [k: string]: unknown;
    }
  | {
      cleanup_action?: PaneCleanupAction | null;
      pane_id: number;
      session_id?: string | null;
      type: "remove_pane";
      [k: string]: unknown;
    }
  | {
      label: string;
      pane_id: number;
      session_id?: string | null;
      type: "update_pane_label";
      [k: string]: unknown;
    }
  | {
      effort?: string | null;
      pane_id: number;
      session_id?: string | null;
      type: "update_pane_effort";
      [k: string]: unknown;
    }
  | {
      model?: string | null;
      pane_id: number;
      provider?: Provider | null;
      session_id?: string | null;
      type: "update_pane_model";
      [k: string]: unknown;
    }
  | {
      pane_id: number;
      request_id?: string | null;
      session_id?: string | null;
      type: "interrupt_pane";
      [k: string]: unknown;
    }
  | {
      pane_ids: number[];
      session_id?: string | null;
      type: "reorder_panes";
      [k: string]: unknown;
    }
  | {
      effort?: string | null;
      min_iteration_interval_minutes?: number | null;
      pane_id: number;
      prompt?: string | null;
      session_id?: string | null;
      type: "start_bot";
      [k: string]: unknown;
    }
  | {
      pane_id: number;
      session_id?: string | null;
      type: "stop_bot";
      [k: string]: unknown;
    }
  | {
      session_id?: string | null;
      type: "reboot_cli";
      [k: string]: unknown;
    }
  | {
      /**
       * `None` when the client asked for an operation this build retired.
       */
      operation?: CliLifecycleOperation | null;
      request_id: string;
      session_id: string;
      type: "cli_lifecycle_request";
      [k: string]: unknown;
    }
  | {
      machine_id: string;
      project_id: string;
      type: "start_machine_project_cli";
      [k: string]: unknown;
    }
  | {
      machine_id: string;
      project_id: string;
      type: "stop_machine_project_cli";
      [k: string]: unknown;
    }
  | {
      machine_id: string;
      type: "reboot_daemon";
      [k: string]: unknown;
    }
  | {
      base_path?: string | null;
      branch: string;
      clone_url?: string | null;
      /**
       * Explicit cluster context for shared targets. Omitted by legacy
       * clients and for compatibility owner flows.
       */
      cluster_owner_user_id?: string | null;
      git_remote: string;
      instance_name: string;
      machine_id: string;
      request_id?: string | null;
      type: "create_project_instance";
      [k: string]: unknown;
    }
  | {
      cluster_owner_user_id?: string | null;
      machine_id: string;
      path: string;
      request_id: string;
      type: "register_local_project";
      [k: string]: unknown;
    }
  | {
      api_base_url?: string | null;
      api_key?: string | null;
      clear_api_key?: boolean;
      machine_id: string;
      type: "set_machine_mini_max_config";
      [k: string]: unknown;
    }
  | {
      api_base_url?: string | null;
      api_key?: string | null;
      clear_api_key?: boolean;
      machine_id: string;
      type: "set_machine_glm_config";
      [k: string]: unknown;
    }
  | {
      api_base_url?: string | null;
      api_key?: string | null;
      clear_api_key?: boolean;
      machine_id: string;
      type: "set_machine_deepseek_config";
      [k: string]: unknown;
    }
  | {
      /**
       * Question text → selected option label(s) joined with ", " for
       * multi-select.
       */
      answers: {
        [k: string]: string;
      };
      pane_id?: number | null;
      request_id?: string | null;
      /**
       * The session the answered AskUserQuestion belongs to. Optional for
       * backward compat with older web clients. When present the server
       * routes the answer deterministically via `resolve_target_session`
       * instead of falling back to the connection's last-attached session
       * — the multi-session fan-out drifts that, which misrouted answers
       * to a different project and left the asking pane stuck.
       */
      session_id?: string | null;
      tool_use_id: string;
      type: "answer_question";
      [k: string]: unknown;
    }
  | {
      answers?: OmpQuestionAnswer[];
      cancel?: boolean;
      dialog_id: string;
      pane_id: number;
      request_id: string;
      session_id: string;
      tool_use_id: string;
      type: "answer_omp_question";
      [k: string]: unknown;
    }
  | {
      pane_id: number;
      session_id?: string | null;
      type: "request_pane_diff";
      [k: string]: unknown;
    }
  | {
      pane_id: number;
      session_id?: string | null;
      type: "create_pr";
      [k: string]: unknown;
    }
  | {
      auto_approve_todos: boolean;
      auto_merge_prs: boolean;
      /**
       * Tab types this project refuses to create, as `<kind>:<provider>`
       * keys (see `tab_type_key`). A *deny* list so that absent means
       * "everything allowed" — see `tab_type_allowed`. Owner/admin only,
       * same gate as the flags above.
       */
      disallowed_tab_types?: string[];
      session_id?: string | null;
      /**
       * Managed team mode on/off for this project. **Owner/admin only** --
       * the server drops the whole message from a plain `user`, because
       * these are project-level policy, not per-seat preferences
       * (`auto_merge_prs` alone lets the Tech Lead merge PRs unattended).
       */
      team_enabled?: boolean;
      type: "update_project_flags";
      [k: string]: unknown;
    }
  | {
      auto_approve_todos: boolean;
      auto_merge_prs: boolean;
      session_id?: string | null;
      type: "update_project_operations";
      [k: string]: unknown;
    }
  | {
      backstory?: string | null;
      goal?: string | null;
      pane_id: number;
      role?: string | null;
      session_id?: string | null;
      type: "update_pane_role";
      [k: string]: unknown;
    }
  | {
      approve: boolean;
      pane_id?: number | null;
      request_id?: string | null;
      session_id?: string | null;
      tool_use_id: string;
      type: "plan_review_answer";
      [k: string]: unknown;
    }
  | {
      mode: PlanReviewMode;
      pane_id: number;
      session_id?: string | null;
      type: "update_pane_review_mode";
      [k: string]: unknown;
    }
  | {
      manual_mode: boolean;
      pane_id: number;
      session_id?: string | null;
      type: "update_pane_manual_mode";
      [k: string]: unknown;
    }
  | {
      data_b64: string;
      pane_id: number;
      session_id: string;
      type: "terminal_input";
      [k: string]: unknown;
    }
  | {
      cols: number;
      pane_id: number;
      rows: number;
      session_id: string;
      type: "terminal_resize";
      [k: string]: unknown;
    }
  | {
      pane_id: number;
      session_id: string;
      type: "terminal_attach";
      [k: string]: unknown;
    }
  | {
      include_current?: boolean;
      pane_id: number;
      session_id: string;
      type: "list_pane_work_summaries";
      [k: string]: unknown;
    }
  | {
      pane_id: number;
      session_id: string;
      type: "refresh_pane_work_summary";
      window_start?: string | null;
      [k: string]: unknown;
    }
  | {
      event: MobileTelemetryEvent;
      type: "mobile_telemetry";
      [k: string]: unknown;
    }
  | {
      type: "heartbeat";
      [k: string]: unknown;
    };
export type ClientKind = "web" | "mobile";
/**
 * What to do with an isolated git worktree (and its branch) when the pane
 * that owns it is closed. Selected by the web UI before sending
 * `WebToServer::RemovePane` so the CLI knows which git commands to run.
 * Phase 1.1d of the swarm plan.
 */
export type PaneCleanupAction = "discard" | "merge_and_remove" | "leave_as_branch";
/**
 * Phase 3.2a: per-pane policy for the "editable plan checkpoint"
 * feature. The streaming worker reads this at every turn to decide
 * whether to gate the first tool_use behind a user-approval card.
 */
export type PlanReviewMode = "always" | "risky_only" | "never";
/**
 * Strictly redacted mobile renderer health signals. These events contain no
 * project/session/pane identifiers or arbitrary strings by design.
 */
export type MobileTelemetryEvent =
  "terminal_bridge_ready" | "terminal_bridge_rejected_message" | "terminal_bridge_crash";

export interface MobileProtocolContract {
  auth_response: MobileAuthResponse;
  bootstrap_response: MobileBootstrapResponse;
  code_event: CodeEvent;
  device_session: MobileDeviceSession;
  jobs: JobProtocolContract;
  login_request: MobileLoginRequest;
  logout_request: MobileLogoutRequest;
  notification_preferences: MobileNotificationPreferences;
  push_token_request: MobilePushTokenRequest;
  refresh_request: MobileRefreshRequest;
  server_to_web: ServerToWeb;
  task_launch_request: MobileTaskLaunchRequest;
  task_launch_response: MobileTaskLaunchResponse;
  web_to_server: WebToServer;
  [k: string]: unknown;
}
export interface MobileAuthResponse {
  access_expires_at: string;
  access_token: string;
  cluster_role: string;
  device_session_id: string;
  refresh_expires_at: string;
  refresh_token: string;
  user_email: string;
  user_id: string;
  [k: string]: unknown;
}
export interface MobileBootstrapResponse {
  account_status: string;
  autonomous_jobs?: MobileJobsBootstrap;
  cluster_role: string;
  features: MobileFeatureFlags;
  launch_targets: MobileLaunchTarget[];
  machines: MachineWithProjects[];
  protocol_max_version: number;
  protocol_min_version: number;
  sessions: MobileSessionSummary[];
  user_email: string;
  user_id: string;
  [k: string]: unknown;
}
export interface MobileJobsBootstrap {
  available: boolean;
  jobs: JobSummary[];
  next_cursor?: string | null;
  unavailable_reasons: string[];
  [k: string]: unknown;
}
export interface JobSummary {
  allowed_actions: JobAllowedActions;
  attention_count: number;
  cluster_owner_user_id: string;
  created_at: string;
  current_run_id: string;
  hostname: string;
  id: string;
  limits: JobLimits;
  machine_id: string;
  project_id: string;
  project_name: string;
  reason?: string | null;
  state: JobState;
  state_version: number;
  title: string;
  updated_at: string;
  usage: JobUsage;
  [k: string]: unknown;
}
export interface JobAllowedActions {
  admit: boolean;
  answer: boolean;
  cancel: boolean;
  manage_policy: boolean;
  retry: boolean;
  retry_publication: boolean;
  [k: string]: unknown;
}
export interface JobLimits {
  attempts: number;
  cpu_millis: number;
  elapsed_seconds: number;
  memory_bytes: number;
  model_concurrency: number;
  model_output_tokens: number;
  output_bytes: number;
  pids: number;
  /**
   * Reported-spend threshold, not a promise about the provider's final invoice.
   */
  spend_micro_usd: number;
  workspace_bytes: number;
}
export interface JobUsage {
  elapsed_seconds: number;
  estimated: boolean;
  input_tokens: number;
  output_tokens: number;
  /**
   * None is unknown, not zero.
   */
  spend_micro_usd?: number | null;
  [k: string]: unknown;
}
export interface MobileFeatureFlags {
  bootstrap?: boolean;
  coding_mutations?: boolean;
  deep_links?: boolean;
  notifications?: boolean;
  terminal?: boolean;
  [k: string]: unknown;
}
export interface MobileLaunchTarget {
  hostname: string;
  instance_path: string;
  machine_id: string;
  online: boolean;
  profiles: MobileLaunchProfile[];
  project_id: string;
  project_name: string;
  [k: string]: unknown;
}
export interface MobileLaunchProfile {
  key: string;
  kind: PaneKind;
  label: string;
  mode: PaneMode;
  model?: string | null;
  provider: Provider;
  [k: string]: unknown;
}
/**
 * Machine with its project list
 */
export interface MachineWithProjects {
  cluster_access?: "owner" | "member";
  cluster_owner_user_id?: string | null;
  local_project_registration_available?: boolean;
  machine: MachineInfo;
  projects: MachineProjectInfo[];
  shared_provisioning_available?: boolean;
  [k: string]: unknown;
}
/**
 * Information about a machine reported by a daemon
 */
export interface MachineInfo {
  arch: string;
  daemon_version?: string | null;
  deepseek_backend?: DeepseekBackendInfo | null;
  hostname: string;
  last_seen?: string | null;
  machine_id: string;
  os: string;
  [k: string]: unknown;
}
/**
 * Machine-level DeepSeek backend status safe to expose to web UI.
 */
export interface DeepseekBackendInfo {
  api_base_url?: string | null;
  api_key?: string | null;
  api_key_configured?: boolean;
  [k: string]: unknown;
}
/**
 * APAS project discovered on a machine
 */
export interface MachineProjectInfo {
  is_running?: boolean;
  last_error?: string | null;
  /**
   * Resident set size of the headless CLI process, in KiB. Reported by the
   * daemon from /proc/<pid>/status so the UI can spot runaway memory usage
   * before the kernel does.
   */
  memory_kb?: number | null;
  name?: string | null;
  path: string;
  pid?: number | null;
  /**
   * Server-persisted navigation preference within this machine's cluster.
   * This is neither a runtime placement lock nor an access grant.
   */
  preferred_machine_id?: string | null;
  project_id: string;
  [k: string]: unknown;
}
/**
 * Information about a persisted session
 */
export interface MobileSessionSummary {
  attention_count?: number;
  cli_client_id?: string | null;
  created_at?: string | null;
  /**
   * Canonical `host/owner/repo` of the project's git `origin` remote. The
   * web sidebar groups projects with the same value under one repo header.
   * `None`/absent means "no remote" (its own sidebar group).
   */
  git_remote?: string | null;
  /**
   * Raw `origin` URL for this project's repo (the cloneable URL). Surfaced
   * so the web can prefill the clone URL when creating a new instance.
   */
  git_remote_url?: string | null;
  hostname?: string | null;
  id: string;
  /**
   * True if this session has an active CLI client connected
   */
  is_active?: boolean;
  /**
   * True if this session is shared with the user (not owned)
   */
  is_shared?: boolean;
  /**
   * True when at least one pane is currently reporting work. This is
   * meaningful only while `is_active` is true.
   */
  is_working?: boolean;
  last_user_input_at?: string | null;
  latest_summary?: string | null;
  latest_update_at?: string | null;
  /**
   * Email of the session owner (only set if is_shared is true)
   */
  owner_email?: string | null;
  /**
   * The panes in this session and whether each is working.
   *
   * `is_working` above answers "is anything happening here", which cannot
   * answer "which agent is waiting for me": a project with one busy pane
   * reads as working, hiding every idle pane in it. Both are derived from
   * the same pane statuses, so they cannot disagree.
   *
   * Defaulted: an older server omits it, and a client must read that as "no
   * pane detail" rather than as a session with no panes.
   */
  panes?: MobilePaneSummary[];
  /**
   * Stable project identity from `.apas`. Web UI groups by this.
   * Falls back to `id` for legacy rows that pre-date the column.
   */
  project_id?: string | null;
  project_name?: string | null;
  /**
   * Share role for this user on the session ("owner", "admin", or "user")
   */
  share_role?: string | null;
  status: string;
  working_dir?: string | null;
  [k: string]: unknown;
}
/**
 * One agent pane, as mobile needs to list it.
 */
export interface MobilePaneSummary {
  /**
   * True while this pane is blocked on an unresolved agent question. This
   * is neither active work nor idle time and takes presentation precedence
   * over provider availability.
   */
  awaiting_answer?: boolean;
  /**
   * Most recent working-to-idle transition observed by the server. Older
   * servers omit it; clients keep those panes visible after timestamped ones.
   */
  idle_since?: string | null;
  /**
   * True while this pane is reporting work, from the same pane statuses the
   * session-level flag is derived from.
   */
  is_working?: boolean;
  kind: PaneKind;
  /**
   * The pane's own label when it has one; the client falls back to its kind
   * and id, exactly as the session screen does.
   */
  label?: string | null;
  /**
   * Model/backend override used to associate provider-scoped availability
   * with the same usage account the pane actually consumes.
   */
  model?: string | null;
  pane_id: number;
  provider: Provider;
  /**
   * Provider availability is not pane activity. Carry the latest explicit
   * block separately so shared/mobile snapshots do not turn it into idle.
   */
  usage_limited?: UsageLimited | null;
  [k: string]: unknown;
}
/**
 * A provider-confirmed usage limit that is currently preventing work.
 *
 * This is deliberately separate from utilization: included usage may reach
 * 100% while paid extra usage remains available, in which case the provider
 * is not blocking requests and this field stays absent.
 */
export interface UsageLimited {
  /**
   * Model family this limit applies to. Absent means the provider account
   * is blocked generally. Anthropic currently uses display names such as
   * "Fable" for model-scoped weekly limits.
   */
  model?: string | null;
  /**
   * When the provider expects work to become available again.
   */
  resets_at?: string | null;
  /**
   * Human-readable limiting window, such as "weekly" or "5-hour".
   */
  window: string;
  [k: string]: unknown;
}
export interface CodeEvent {
  created_at: string;
  detail?: unknown;
  id: string;
  kind: CodeEventKind;
  ordering_key: string;
  pane_id?: number | null;
  requires_attention?: boolean;
  session_id: string;
  summary: string;
  [k: string]: unknown;
}
export interface MobileDeviceSession {
  app_version: string;
  created_at: string;
  device_name?: string | null;
  expires_at: string;
  id: string;
  installation_id: string;
  last_used_at: string;
  platform: MobilePlatform;
  revoked_at?: string | null;
  [k: string]: unknown;
}
export interface JobProtocolContract {
  admission_request: JobAdmissionRequest;
  answer_request: JobAnswerRequest;
  artifact: JobArtifactContent;
  bootstrap: MobileJobsBootstrap1;
  deployment_policy: JobPolicy;
  detail: JobDetail;
  error: JobApiError;
  events: JobEventsResponse;
  list: JobListResponse;
  mutation_request: JobMutationRequest;
  policy: EffectiveJobPolicy;
  policy_management: JobPolicyManagement;
  policy_override: JobPolicyOverride;
  preview: JobPreview;
  preview_request: JobPreviewRequest;
  project_list: ProjectJobListResponse;
  receipt: JobMutationReceipt;
  review: JobReview;
  targets: JobTargetsResponse;
  [k: string]: unknown;
}
export interface JobAdmissionRequest {
  fingerprint: string;
  preview_id: string;
  request_id: string;
}
export interface JobAnswerRequest {
  answers: JobQuestionAnswer[];
  epoch: number;
  expected_version: number;
  request_id: string;
  run_id: string;
}
export interface JobQuestionAnswer {
  custom_input?: string | null;
  id: string;
  selected_option_ids: string[];
}
export interface JobArtifactContent {
  artifact: JobArtifact;
  expired: boolean;
  next_cursor?: string | null;
  text: string;
  [k: string]: unknown;
}
export interface JobArtifact {
  content_type: string;
  expires_at?: string | null;
  id: string;
  kind: JobArtifactKind;
  run_id: string;
  sha256: string;
  size_bytes: number;
  truncated: boolean;
  [k: string]: unknown;
}
export interface MobileJobsBootstrap1 {
  available: boolean;
  jobs: JobSummary[];
  next_cursor?: string | null;
  unavailable_reasons: string[];
  [k: string]: unknown;
}
export interface JobPolicy {
  enabled: boolean;
  limits: JobLimits;
  max_active_cluster: number;
  max_active_machine: number;
  max_active_project: number;
  max_queued_project: number;
  model_profiles: string[];
  publish_enabled: boolean;
  verification_profiles: string[];
  worker_profiles: string[];
}
export interface JobDetail {
  high_watermark: number;
  job: JobSummary;
  policy_digest: string;
  publication?: JobPublication | null;
  questions: JobQuestion[];
  runs: JobRun[];
  task: JobTaskSpec;
  [k: string]: unknown;
}
export interface JobPublication {
  base_sha: string;
  branch: string;
  draft: boolean;
  head_sha: string;
  number: number;
  observed_at: string;
  repository: string;
  state: string;
  url: string;
  [k: string]: unknown;
}
export interface JobQuestion {
  answers?: JobQuestionAnswer[] | null;
  confirmed_at?: string | null;
  created_at: string;
  dialog_id: string;
  epoch: number;
  id: string;
  questions: JobQuestionItem[];
  run_id: string;
  state: JobQuestionState;
  tool_call_id: string;
  version: number;
  [k: string]: unknown;
}
export interface JobQuestionItem {
  header?: string | null;
  id: string;
  multi: boolean;
  options: JobQuestionOption[];
  question: string;
  recommended?: number | null;
  [k: string]: unknown;
}
export interface JobQuestionOption {
  description?: string | null;
  id: string;
  label: string;
  preview?: string | null;
  [k: string]: unknown;
}
export interface JobRun {
  attempt: number;
  candidate_sha?: string | null;
  ended_at?: string | null;
  epoch: number;
  id: string;
  job_id: string;
  reason?: string | null;
  started_at?: string | null;
  state: JobState;
  stop_evidence?: JobStopEvidence | null;
  usage: JobUsage;
  worker_version?: string | null;
  [k: string]: unknown;
}
export interface JobStopEvidence {
  boot_id: string;
  container_id: string;
  descendants_stopped: boolean;
  reason: string;
  stopped_at: string;
  [k: string]: unknown;
}
export interface JobTaskSpec {
  acceptance_criteria: string[];
  base_ref: string;
  base_sha: string;
  instruction: string;
  model_profile: string;
  repository: string;
  source_url?: string | null;
  verification_profile: string;
  verification_profile_digest: string;
  worker_profile: string;
  worker_profile_digest: string;
  [k: string]: unknown;
}
export interface JobApiError {
  code: string;
  error: string;
  request_id?: string | null;
  retryable: boolean;
  [k: string]: unknown;
}
export interface JobEventsResponse {
  events: JobEvent[];
  has_more: boolean;
  high_watermark: number;
  next_seq: number;
  resync_required: boolean;
  [k: string]: unknown;
}
export interface JobEvent {
  created_at: string;
  detail?: unknown;
  id: string;
  job_id: string;
  kind: JobEventKind;
  run_id?: string | null;
  seq: number;
  summary: string;
  [k: string]: unknown;
}
export interface JobListResponse {
  jobs: JobSummary[];
  next_cursor?: string | null;
  [k: string]: unknown;
}
export interface JobMutationRequest {
  expected_version: number;
  request_id: string;
}
export interface EffectiveJobPolicy {
  digest: string;
  policy: JobPolicy;
  requested?: JobPolicyOverride | null;
  sources: JobPolicySource[];
  [k: string]: unknown;
}
export interface JobPolicyOverride {
  enabled?: boolean | null;
  limits?: JobLimits | null;
  max_active_cluster?: number | null;
  max_active_machine?: number | null;
  max_active_project?: number | null;
  max_queued_project?: number | null;
  model_profiles?: string[] | null;
  publish_enabled?: boolean | null;
  verification_profiles?: string[] | null;
  worker_profiles?: string[] | null;
}
export interface JobPolicySource {
  id: string;
  revision: number;
  scope: string;
  [k: string]: unknown;
}
export interface JobPolicyManagement {
  can_manage: boolean;
  effective?: EffectiveJobPolicy | null;
  inherited?: EffectiveJobPolicy | null;
  requested?: JobPolicyDocument | null;
  unavailable_reasons: string[];
  [k: string]: unknown;
}
export interface JobPreview {
  cluster_owner_user_id: string;
  expires_at: string;
  fingerprint: string;
  hostname: string;
  id: string;
  limits: JobLimits;
  machine_id: string;
  policy_digest: string;
  project_id: string;
  task: JobTaskSpec;
  [k: string]: unknown;
}
export interface JobPreviewRequest {
  acceptance_criteria: string[];
  base_ref: string;
  instruction: string;
  limits: JobLimits;
  machine_id: string;
  model_profile: string;
  source_url?: string | null;
  verification_profile: string;
  worker_profile: string;
}
export interface ProjectJobListResponse {
  jobs: ProjectJobSummary[];
  next_cursor?: string | null;
  [k: string]: unknown;
}
export interface ProjectJobSummary {
  allowed_actions: JobAllowedActions;
  attention_count: number;
  cluster_owner_user_id: string;
  created_at: string;
  current_run_id: string;
  description: string;
  hostname: string;
  id: string;
  limits: JobLimits;
  machine_id: string;
  project_id: string;
  project_name: string;
  reason?: string | null;
  state: JobState;
  state_version: number;
  title: string;
  updated_at: string;
  usage: JobUsage;
  [k: string]: unknown;
}
export interface JobMutationReceipt {
  error_code?: string | null;
  job_id?: string | null;
  message?: string | null;
  operation: JobMutationKind;
  request_id: string;
  run_id?: string | null;
  status: JobReceiptStatus;
  [k: string]: unknown;
}
export interface JobReview {
  base_sha: string;
  candidate_sha?: string | null;
  files: JobDiffFile[];
  job_id: string;
  next_cursor?: string | null;
  publication?: JobPublication | null;
  reason?: string | null;
  run_id: string;
  verification?: JobVerification | null;
  [k: string]: unknown;
}
export interface JobDiffFile {
  additions: number;
  artifact_id?: string | null;
  binary: boolean;
  deletions: number;
  path: string;
  unavailable_reason?: string | null;
  [k: string]: unknown;
}
export interface JobVerification {
  base_sha: string;
  candidate_sha: string;
  checks: JobCheckResult[];
  completed_at: string;
  image_digest: string;
  profile_digest: string;
  [k: string]: unknown;
}
export interface JobCheckResult {
  argv: string[];
  elapsed_seconds: number;
  exit_code?: number | null;
  log_artifact_id?: string | null;
  name: string;
  status: JobCheckStatus;
  [k: string]: unknown;
}
export interface JobTargetsResponse {
  project_id: string;
  targets: JobTarget[];
  unavailable_reasons: string[];
  [k: string]: unknown;
}
export interface JobTarget {
  allowed_actions: JobAllowedActions;
  available: boolean;
  cluster_owner_user_id: string;
  hostname: string;
  machine_id: string;
  policy: EffectiveJobPolicy;
  profiles: JobWorkerProfile[];
  project_id: string;
  unavailable_reasons: string[];
  [k: string]: unknown;
}
export interface JobWorkerProfile {
  digest: string;
  image: string;
  key: string;
  label: string;
  limits: JobLimits;
  model_profiles: JobModelProfile[];
  omp_version: string;
  ready: boolean;
  unavailable_reasons: string[];
  verification_profiles: JobVerificationProfile[];
  [k: string]: unknown;
}
export interface JobModelProfile {
  api: JobModelApi;
  key: string;
  label: string;
  model: string;
  [k: string]: unknown;
}
export interface JobVerificationProfile {
  digest: string;
  image: string;
  key: string;
  label: string;
  [k: string]: unknown;
}
export interface MobileLoginRequest {
  app_version: string;
  device_name?: string | null;
  email: string;
  installation_id: string;
  password: string;
  platform: MobilePlatform;
  [k: string]: unknown;
}
export interface MobileLogoutRequest {
  refresh_token: string;
  [k: string]: unknown;
}
export interface MobileNotificationPreferences {
  completions?: boolean;
  decisions?: boolean;
  failures?: boolean;
  pull_requests?: boolean;
  [k: string]: unknown;
}
export interface MobilePushTokenRequest {
  installation_id: string;
  platform: MobilePlatform;
  token: string;
  [k: string]: unknown;
}
export interface MobileRefreshRequest {
  installation_id: string;
  refresh_token: string;
  [k: string]: unknown;
}
/**
 * Current lifecycle capabilities and the per-pane reboot consequence list.
 * Empty/default values make the message safe while CLI versions roll out.
 */
export interface CliLifecycleInventory {
  panes?: PanePreservationInfo[];
  persistent_terminal_hosting?: boolean;
  [k: string]: unknown;
}
export interface PanePreservationInfo {
  mode: PanePreservationMode;
  pane_id: number;
  runtime_id?: string | null;
  [k: string]: unknown;
}
/**
 * Current lifecycle capabilities and the per-pane reboot consequence list.
 * Empty/default values make the message safe while CLI versions roll out.
 */
export interface CliLifecycleInventory1 {
  panes?: PanePreservationInfo[];
  persistent_terminal_hosting?: boolean;
  [k: string]: unknown;
}
/**
 * Information about a CLI client
 */
export interface CliClientInfo {
  /**
   * Active session ID if the CLI has a local session running
   */
  active_session?: string | null;
  id: string;
  last_seen?: string | null;
  name?: string | null;
  status: CliClientStatus;
  version?: string | null;
  [k: string]: unknown;
}
/**
 * Claude assistant message structure
 */
export interface ClaudeAssistantMessage {
  content: ClaudeContentBlock[];
  model?: string;
  [k: string]: unknown;
}
/**
 * Claude user message structure (for tool results)
 */
export interface ClaudeUserMessage {
  content: ClaudeContentBlock[];
  role?: string;
  [k: string]: unknown;
}
/**
 * Information about a persisted session
 */
export interface SessionInfo {
  cli_client_id?: string | null;
  created_at?: string | null;
  /**
   * Canonical `host/owner/repo` of the project's git `origin` remote. The
   * web sidebar groups projects with the same value under one repo header.
   * `None`/absent means "no remote" (its own sidebar group).
   */
  git_remote?: string | null;
  /**
   * Raw `origin` URL for this project's repo (the cloneable URL). Surfaced
   * so the web can prefill the clone URL when creating a new instance.
   */
  git_remote_url?: string | null;
  hostname?: string | null;
  id: string;
  /**
   * True if this session has an active CLI client connected
   */
  is_active?: boolean;
  /**
   * True if this session is shared with the user (not owned)
   */
  is_shared?: boolean;
  /**
   * True when at least one pane is currently reporting work. This is
   * meaningful only while `is_active` is true.
   */
  is_working?: boolean;
  /**
   * Email of the session owner (only set if is_shared is true)
   */
  owner_email?: string | null;
  /**
   * The panes in this session and whether each is working.
   *
   * `is_working` above answers "is anything happening here", which cannot
   * answer "which agent is waiting for me": a project with one busy pane
   * reads as working, hiding every idle pane in it. Both are derived from
   * the same pane statuses, so they cannot disagree.
   *
   * Defaulted: an older server omits it, and a client must read that as "no
   * pane detail" rather than as a session with no panes.
   */
  panes?: MobilePaneSummary[];
  /**
   * Stable project identity from `.apas`. Web UI groups by this.
   * Falls back to `id` for legacy rows that pre-date the column.
   */
  project_id?: string | null;
  /**
   * Share role for this user on the session ("owner", "admin", or "user")
   */
  share_role?: string | null;
  status: string;
  working_dir?: string | null;
  [k: string]: unknown;
}
/**
 * Information about a persisted message
 */
export interface MessageInfo {
  content: string;
  created_at?: string | null;
  id: string;
  message_type: string;
  pane_id?: number | null;
  pane_type?: string | null;
  role: string;
  [k: string]: unknown;
}
/**
 * Configuration for a single pane
 */
export interface PaneConfig {
  /**
   * Free-form additional context appended to the system prompt
   * (project conventions, constraints, prior decisions). Long-ish
   * is fine — claude's context window is large.
   */
  backstory?: string | null;
  effort?: string | null;
  /**
   * One-line objective the pane is currently working toward, e.g.
   * "make the auth tests green". Surfaced in the system prompt and
   * (later) on the pane header.
   */
  goal?: string | null;
  is_paused?: boolean;
  /**
   * How a pane hosts its agent process. Orthogonal to [`Provider`] (which
   * binary) and [`PaneMode`] (how autonomous).
   *
   * * [`PaneKind::Agent`] — the legacy/managed-team path: the CLI runs the provider
   *   headlessly (`claude --print --output-format stream-json`,
   *   `codex exec --json`) and parses structured events into
   *   `CliToServer::StreamMessage`. Everything team-mode depends on —
   *   usage counters, pane status, diffs, plan review, scratchpad
   *   publishing, Tech Lead delegation — is built on those events. New
   *   unmanaged panes are no longer created with this kind, but existing panes
   *   remain compatible and managed team panes continue to use it.
   * * [`PaneKind::Terminal`] — the pane instead hosts the provider's real
   *   interactive TUI on a pty. Raw bytes flow over the dedicated
   *   `Terminal*` messages and are rendered by xterm.js in the browser.
   *   Nothing is parsed, so a terminal pane has none of the structured
   *   integrations above and is never a delegation target. This is the normal
   *   kind for new user-created Claude, Codex, and OpenCode panes.
   *
   * `#[serde(default)]` on `PaneConfig::kind` keeps `.apas` files written
   * before this existed deserializing as `Agent`.
   */
  kind?: "agent" | "terminal";
  label?: string | null;
  /**
   * v3.5 — managed vs. unmanaged. `true` = this pane is part of the
   * project team, usually created by the Overview Start team role
   * slots or by accepted worker suggestions / manual managed-worker
   * flows. Such panes show up on the Overview Pane Grid and the Tech
   * Lead may consider them for delegation. `false` (the compatibility
   * default and the value for user-created terminal panes) = not part of
   * the team queue and never a Tech Lead delegation target.
   */
  managed?: boolean;
  /**
   * v3.2 — worker mode. `false` (default) = **autonomous**: the Tech
   * Lead may delegate to this pane via `.apas-team.jsonl`. `true` =
   * **manual**: the worker only takes user chat; the Tech Lead should
   * skip it when picking a delegation target. Persisted to `.apas`.
   */
  manual_mode?: boolean;
  min_iteration_interval_minutes?: number | null;
  mode: PaneMode;
  model?: string | null;
  pane_id: number;
  /**
   * Phase 3.2a: per-pane policy for the editable plan checkpoint.
   * Default is `Never` (today's behaviour) so existing panes keep
   * running without prompts.
   */
  plan_review_mode?: "always" | "risky_only" | "never";
  prompt?: string | null;
  provider: Provider;
  /**
   * Short role label for the agent in this pane, e.g. "backend
   * implementer" or "reviewer". When set, gets prepended to claude's
   * system prompt at spawn so the agent self-identifies. Phase 2.1.
   */
  role?: string | null;
  session_id: string;
  stop_requested?: boolean;
  /**
   * Absolute path to an isolated git worktree this pane should run in.
   * When `None`, the pane runs in the project's main working_dir as before
   * (legacy behaviour, all panes share one tree → potential conflicts).
   * Phase 1.1 of the swarm plan adds an opt-in path that puts each pane
   * on its own branch+worktree so parallel work doesn't race; this field
   * is the persistence hook for that. The worktree itself is created
   * out-of-band (CLI subcommand / web action) — apas does not touch git
   * just because the field is set.
   */
  worktree_path?: string | null;
  [k: string]: unknown;
}
/**
 * Usage limits from the provider API/logs
 */
export interface UsageLimits {
  /**
   * When the usage was last fetched (ISO 8601 timestamp)
   */
  fetched_at?: string | null;
  /**
   * 5-hour rolling window usage
   */
  five_hour?: UsageLimitWindow | null;
  /**
   * 7-day (weekly) rolling window usage
   */
  seven_day?: UsageLimitWindow | null;
  /**
   * Present only when the provider says a usage limit is actively blocking
   * requests. A full utilization meter alone is not sufficient.
   */
  usage_limited?: UsageLimited | null;
  [k: string]: unknown;
}
/**
 * Usage limit information for a time window (5-hour or 7-day)
 */
export interface UsageLimitWindow {
  /**
   * When the limit resets (ISO 8601 timestamp)
   */
  resets_at?: string | null;
  /**
   * Utilization as a fraction (0.0 to 1.0+)
   */
  utilization: number;
  [k: string]: unknown;
}
/**
 * Project-level usage: the per-pane breakdown plus the project totals
 * (sum over all panes of every session that shares this project_id).
 */
export interface ProjectUsageStats {
  last_7d?: UsageCounters;
  last_active?: string | null;
  lifetime?: UsageCounters1;
  panes?: PaneUsageStats[];
  today?: UsageCounters5;
  [k: string]: unknown;
}
/**
 * Aggregated usage counters for a pane or project over one time window.
 * All token counts come from the per-turn Claude/Codex stream `result`
 * usage; `prompts` counts user/loop inputs and `responses` counts completed
 * turns. Fields are snake_case so the wire keys match the web store verbatim.
 */
export interface UsageCounters {
  cache_creation_tokens?: number;
  cache_read_tokens?: number;
  /**
   * Sum of provider-reported cost in USD. A zero is meaningful only when
   * `cost_usd_reported` is true.
   */
  cost_usd?: number;
  /**
   * Whether at least one event in this window supplied an authoritative
   * cost value. Older peers omit this and therefore read as unknown.
   */
  cost_usd_reported?: boolean;
  input_tokens?: number;
  output_tokens?: number;
  prompts?: number;
  responses?: number;
  [k: string]: unknown;
}
/**
 * Aggregated usage counters for a pane or project over one time window.
 * All token counts come from the per-turn Claude/Codex stream `result`
 * usage; `prompts` counts user/loop inputs and `responses` counts completed
 * turns. Fields are snake_case so the wire keys match the web store verbatim.
 */
export interface UsageCounters1 {
  cache_creation_tokens?: number;
  cache_read_tokens?: number;
  /**
   * Sum of provider-reported cost in USD. A zero is meaningful only when
   * `cost_usd_reported` is true.
   */
  cost_usd?: number;
  /**
   * Whether at least one event in this window supplied an authoritative
   * cost value. Older peers omit this and therefore read as unknown.
   */
  cost_usd_reported?: boolean;
  input_tokens?: number;
  output_tokens?: number;
  prompts?: number;
  responses?: number;
  [k: string]: unknown;
}
/**
 * Per-pane usage broken down by time window (cumulative lifetime plus the
 * rolling 7-day and today windows derived from day-bucketed rows).
 */
export interface PaneUsageStats {
  last_7d?: UsageCounters2;
  /**
   * Most recent activity timestamp (ISO 8601), max over the pane's buckets.
   */
  last_active?: string | null;
  lifetime?: UsageCounters3;
  pane_id: number;
  today?: UsageCounters4;
  [k: string]: unknown;
}
/**
 * Aggregated usage counters for a pane or project over one time window.
 * All token counts come from the per-turn Claude/Codex stream `result`
 * usage; `prompts` counts user/loop inputs and `responses` counts completed
 * turns. Fields are snake_case so the wire keys match the web store verbatim.
 */
export interface UsageCounters2 {
  cache_creation_tokens?: number;
  cache_read_tokens?: number;
  /**
   * Sum of provider-reported cost in USD. A zero is meaningful only when
   * `cost_usd_reported` is true.
   */
  cost_usd?: number;
  /**
   * Whether at least one event in this window supplied an authoritative
   * cost value. Older peers omit this and therefore read as unknown.
   */
  cost_usd_reported?: boolean;
  input_tokens?: number;
  output_tokens?: number;
  prompts?: number;
  responses?: number;
  [k: string]: unknown;
}
/**
 * Aggregated usage counters for a pane or project over one time window.
 * All token counts come from the per-turn Claude/Codex stream `result`
 * usage; `prompts` counts user/loop inputs and `responses` counts completed
 * turns. Fields are snake_case so the wire keys match the web store verbatim.
 */
export interface UsageCounters3 {
  cache_creation_tokens?: number;
  cache_read_tokens?: number;
  /**
   * Sum of provider-reported cost in USD. A zero is meaningful only when
   * `cost_usd_reported` is true.
   */
  cost_usd?: number;
  /**
   * Whether at least one event in this window supplied an authoritative
   * cost value. Older peers omit this and therefore read as unknown.
   */
  cost_usd_reported?: boolean;
  input_tokens?: number;
  output_tokens?: number;
  prompts?: number;
  responses?: number;
  [k: string]: unknown;
}
/**
 * Aggregated usage counters for a pane or project over one time window.
 * All token counts come from the per-turn Claude/Codex stream `result`
 * usage; `prompts` counts user/loop inputs and `responses` counts completed
 * turns. Fields are snake_case so the wire keys match the web store verbatim.
 */
export interface UsageCounters4 {
  cache_creation_tokens?: number;
  cache_read_tokens?: number;
  /**
   * Sum of provider-reported cost in USD. A zero is meaningful only when
   * `cost_usd_reported` is true.
   */
  cost_usd?: number;
  /**
   * Whether at least one event in this window supplied an authoritative
   * cost value. Older peers omit this and therefore read as unknown.
   */
  cost_usd_reported?: boolean;
  input_tokens?: number;
  output_tokens?: number;
  prompts?: number;
  responses?: number;
  [k: string]: unknown;
}
/**
 * Aggregated usage counters for a pane or project over one time window.
 * All token counts come from the per-turn Claude/Codex stream `result`
 * usage; `prompts` counts user/loop inputs and `responses` counts completed
 * turns. Fields are snake_case so the wire keys match the web store verbatim.
 */
export interface UsageCounters5 {
  cache_creation_tokens?: number;
  cache_read_tokens?: number;
  /**
   * Sum of provider-reported cost in USD. A zero is meaningful only when
   * `cost_usd_reported` is true.
   */
  cost_usd?: number;
  /**
   * Whether at least one event in this window supplied an authoritative
   * cost value. Older peers omit this and therefore read as unknown.
   */
  cost_usd_reported?: boolean;
  input_tokens?: number;
  output_tokens?: number;
  prompts?: number;
  responses?: number;
  [k: string]: unknown;
}
export interface EffectiveProjectPolicy {
  allowed_launch_profiles: string[];
  project_suspended?: boolean;
  team_available: boolean;
  version: number;
  [k: string]: unknown;
}
/**
 * Host-owned terminal continuity metadata reported during reconciliation.
 */
export interface TerminalRuntimeReconciliation {
  current_seq?: number;
  live_adopted?: boolean;
  oldest_seq?: number;
  runtime_id?: string | null;
  truncated?: boolean;
  [k: string]: unknown;
}
/**
 * One cached summary record. Source text and intermediate notes are never
 * persisted in this record.
 */
export interface PaneWorkSummary {
  attempts?: number;
  error?: string | null;
  generated_at?: string | null;
  model?: string | null;
  pane_id: number;
  protocol_version?: number;
  provider?: string | null;
  session_id: string;
  source_digest?: string;
  source_message_count?: number;
  source_through?: string | null;
  source_through_id?: string | null;
  /**
   * Durable generation state for one pane/window/source digest.
   */
  status?: "queued" | "generating" | "complete" | "partial" | "stale" | "failed" | "source_expired";
  summary?: string | null;
  updated_at?: string | null;
  window_end: string;
  /**
   * Whether the cached record covers a closed window or the still-open window.
   */
  window_kind?: "completed" | "current";
  window_start: string;
  [k: string]: unknown;
}
export interface MobileTaskLaunchRequest {
  instruction: string;
  machine_id: string;
  profile_key: string;
  project_id: string;
  request_id: string;
  [k: string]: unknown;
}
export interface MobileTaskLaunchResponse {
  pane_id?: number | null;
  request_id: string;
  session_id: string;
  status: string;
  [k: string]: unknown;
}
/**
 * A native OMP answer. Question identity and selections must not be flattened
 * into question text or comma-separated labels.
 */
export interface OmpQuestionAnswer {
  customInput?: string | null;
  id: string;
  selectedOptions: string[];
  [k: string]: unknown;
}
