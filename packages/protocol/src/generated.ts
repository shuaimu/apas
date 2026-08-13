/* Generated from Rust JSON Schema. Do not edit by hand. */

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
export type Provider = ("codex" | "deepseek" | "opencode" | "cursor-agent") | "claude" | "minimax" | "glm";
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
export type ServerToWeb =
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
      request_id: string;
      session_id: string;
      type: "mutation_ack";
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
      provider?: ("codex" | "deepseek" | "opencode" | "cursor-agent") | "claude" | "minimax" | "glm";
      type: "usage_limits";
      [k: string]: unknown;
    }
  | {
      created_at?: string | null;
      hostname?: string | null;
      messages: MessageInfo[];
      project_id?: string | null;
      session_id: string;
      type: "session_download";
      working_dir?: string | null;
      [k: string]: unknown;
    }
  | {
      record: TeamScratchpadRecord;
      session_id: string;
      type: "team_record";
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
      content: string;
      session_id: string;
      type: "project_goal_changed";
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
      session_id: string;
      state: TeamTodoStateMsg;
      type: "team_todo_state";
      [k: string]: unknown;
    }
  | {
      session_id: string;
      suggestions: SuggestedWorkerMsg[];
      type: "suggested_workers_state";
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
 * Pane type for dual-pane mode (legacy - kept for backward compatibility)
 */
export type PaneType = "deadloop" | "interactive";
/**
 * Session status
 */
export type SessionStatus = "pending" | "connected" | "disconnected" | "ended";
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
 * Messages sent from web client to server
 */
export type WebToServer =
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
      base_path?: string | null;
      branch: string;
      clone_url?: string | null;
      git_remote: string;
      instance_name: string;
      machine_id: string;
      request_id?: string | null;
      type: "create_project_instance";
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
      session_id: string;
      type: "download_session";
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
      goal: string;
      session_id?: string | null;
      type: "update_project_goal";
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
      developer?: TeamRoleSpec;
      manager?: TeamRoleSpec1;
      reviewer?: TeamRoleSpec2;
      session_id?: string | null;
      tech_lead?: TeamRoleSpec3;
      type: "start_team";
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
      session_id: string;
      type: "fetch_team_todo";
      [k: string]: unknown;
    }
  | {
      action: string;
      session_id: string;
      todo_id: string;
      type: "todo_approval";
      [k: string]: unknown;
    }
  | {
      body?: string;
      session_id: string;
      title: string;
      type: "add_todo";
      [k: string]: unknown;
    }
  | {
      session_id: string;
      type: "fetch_suggested_workers";
      [k: string]: unknown;
    }
  | {
      session_id: string;
      suggestion_id: string;
      type: "dismiss_suggestion";
      [k: string]: unknown;
    }
  | {
      pane_id: number;
      session_id: string;
      type: "promote_pane_to_managed";
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
  machine: MachineInfo;
  projects: MachineProjectInfo[];
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
 * One published artifact in the team scratchpad (`.apas-team.jsonl`),
 * mirroring the CLI's `crate::scratchpad::TeamRecord`. Separate type
 * here so the wire shape is stable across CLI/server/web even if the
 * CLI's internal helper grows extra columns. Phase 2.2b.
 */
export interface TeamScratchpadRecord {
  body: string;
  kind: string;
  pane_id?: number | null;
  tags?: string[];
  ts: string;
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
   * Real cost in USD (only Claude transport reports it; 0 otherwise).
   */
  cost_usd?: number;
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
   * Real cost in USD (only Claude transport reports it; 0 otherwise).
   */
  cost_usd?: number;
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
   * Real cost in USD (only Claude transport reports it; 0 otherwise).
   */
  cost_usd?: number;
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
   * Real cost in USD (only Claude transport reports it; 0 otherwise).
   */
  cost_usd?: number;
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
   * Real cost in USD (only Claude transport reports it; 0 otherwise).
   */
  cost_usd?: number;
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
   * Real cost in USD (only Claude transport reports it; 0 otherwise).
   */
  cost_usd?: number;
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
 * Wire format for a snapshot of `team-todo.md`. Mirrors the CLI's
 * `team_todo::TeamTodo` but with status fields kept as strings so we
 * don't have to ship the enum definitions across crate / language
 * boundaries. The web parses these into UI state.
 */
export interface TeamTodoStateMsg {
  globals: TeamTodoGlobalMsg[];
  reviewer_cursor?: string | null;
  /**
   * Per-agent scratchpad cursor (RFC3339 timestamp of the last
   * record acted on). `None` means the cursor file is missing —
   * either the agent hasn't iterated yet, or it was wiped.
   */
  tech_lead_cursor?: string | null;
  workers: TeamTodoWorkerMsg[];
  [k: string]: unknown;
}
export interface TeamTodoGlobalMsg {
  body: string;
  id: string;
  /**
   * user | tech-lead
   */
  origin: string;
  /**
   * One PR per contributing worker. Empty until any worker's branch
   * has been pushed and PR'd.
   */
  prs?: PaneTodoPrMsg[];
  /**
   * proposed | approved | in_progress | under_review | pr_open | done | rejected
   */
  status: string;
  title: string;
  [k: string]: unknown;
}
export interface PaneTodoPrMsg {
  annotation?: string | null;
  pane_id: number;
  url: string;
  [k: string]: unknown;
}
export interface TeamTodoWorkerMsg {
  pane_id: number;
  role_hint?: string | null;
  subtasks: TeamTodoSubtaskMsg[];
  [k: string]: unknown;
}
export interface TeamTodoSubtaskMsg {
  body: string;
  id: string;
  parent: string;
  /**
   * pending | in_progress | done | reviewing | revising | approved
   */
  status: string;
  title: string;
  [k: string]: unknown;
}
/**
 * One row in the suggested-workers queue. The Manager pane appends
 * these as `## SUG-NNN — label` sections to `suggested-workers.md`;
 * the Overview renders each as a card with Accept / Dismiss buttons.
 */
export interface SuggestedWorkerMsg {
  backstory: string;
  goal: string;
  id: string;
  label: string;
  needs_worktree?: boolean;
  role: string;
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
 * Per-role provider/model pair the user picks in the "Team setup"
 * card before clicking Start team. Empty fields fall back to the
 * CLI's defaults (Claude / unset model).
 */
export interface TeamRoleSpec {
  model?: string | null;
  provider?: Provider | null;
  [k: string]: unknown;
}
/**
 * Per-role provider/model pair the user picks in the "Team setup"
 * card before clicking Start team. Empty fields fall back to the
 * CLI's defaults (Claude / unset model).
 */
export interface TeamRoleSpec1 {
  model?: string | null;
  provider?: Provider | null;
  [k: string]: unknown;
}
/**
 * Per-role provider/model pair the user picks in the "Team setup"
 * card before clicking Start team. Empty fields fall back to the
 * CLI's defaults (Claude / unset model).
 */
export interface TeamRoleSpec2 {
  model?: string | null;
  provider?: Provider | null;
  [k: string]: unknown;
}
/**
 * Per-role provider/model pair the user picks in the "Team setup"
 * card before clicking Start team. Empty fields fall back to the
 * CLI's defaults (Claude / unset model).
 */
export interface TeamRoleSpec3 {
  model?: string | null;
  provider?: Provider | null;
  [k: string]: unknown;
}
