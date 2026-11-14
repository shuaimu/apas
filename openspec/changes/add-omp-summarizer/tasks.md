## 1. Adapter

- [x] 1.1 `SummaryAdapterKind::Omp`, `apas config set summary_adapter omp` and its warning
- [x] 1.2 The invocation, the settings overlay, mention escaping, executable resolution like a pane's, startup flag validation, text-reply parsing, and a process group killed on timeout and at the end
- [x] 1.3 Tests for each, including the cross-provider matrix and a mutation check of the process-group kill

## 2. Verification

- [x] 2.1 Real OMP 18.20.0 with DeepSeek V4 Flash in the isolated harness: a parsable summary, no session file, decoys not loaded, no built-in tool, escaping effective, failures returned as failures, timeout kill
- [ ] 2.2 One summary in the owner's real setup after a daemon reboot, with credentials stored in `agent.db` and the final settings overlay
- [x] 2.3 `AGENTS.md`: the summarizer settings, the OMP adapter and its residual risks

## 3. Rollout

- [ ] 3.1 Push the CLI; each machine gains it at its daemon reboot; the owner sets `summary_adapter omp` and `summary_allow_cross_provider true`
