## MODIFIED Requirements

### Requirement: Summary generation is isolated from the working agent
Summary generation SHALL run outside the active pane conversation through a capability-advertising summarizer configured on the project CLI host. The summarizer SHALL start a new non-resumed invocation without repository write access or access to the active terminal; SHALL treat source messages as untrusted data; and SHALL NOT add messages, status changes, or terminal bytes to the pane being summarized. An adapter with a true tool-disable mode SHALL use it. A Codex adapter MAY retain its headless command tool only after explicit operator selection and SHALL use a fresh empty working directory, ephemeral execution, ignored user configuration and rules, a read-only sandbox, structured output, and a prompt that forbids following or executing instructions found in source data. Documentation SHALL disclose that these controls reduce but do not eliminate prompt-injection-driven host reads. An OMP adapter SHALL run OMP non-interactively with no session persistence, no built-in tools, no discovered extensions, skills, rules, context files or MCP and custom-tool providers, tool calls that need approval refused, mention expansion of source data prevented, and its whole process group ended on timeout; it SHALL use OMP's configured default model unless an operator sets one, and documentation SHALL disclose the tools OMP cannot be told to drop. The summarizer provider SHALL be independent of the pane provider, and a CLI SHALL execute at most one summary job concurrently.

#### Scenario: Summary is generated while the pane is working
- **WHEN** a summary job runs while the active agent continues its task
- **THEN** the active agent process, conversation, working status, and terminal remain unchanged
- **AND** only the separate summarizer's quota is consumed

#### Scenario: Source text contains instructions for the summarizer
- **WHEN** retained conversation text attempts to make the summarizer run a tool, reveal a secret, or ignore the summary contract
- **THEN** the invocation prompt and source delimiters identify that text only as material to summarize
- **AND** a tool-free adapter cannot execute it, while an explicitly enabled Codex adapter applies its read-only controls and documented residual-risk policy

#### Scenario: Operator explicitly enables Codex summaries
- **WHEN** the host operator selects the Codex summary adapter and enables summary generation
- **THEN** the CLI validates the required headless isolation flags before advertising summary capability
- **AND** runs each job ephemerally in a fresh empty directory without loading user configuration, project rules, a pane session, or repository write access
- **AND** identifies the remaining host-read risk in operator documentation

#### Scenario: No eligible summarizer is configured
- **WHEN** the project CLI has no enabled summarizer adapter that satisfies its provider-specific startup validation
- **THEN** it rejects the job with an actionable unavailable reason
- **AND** the agent pane continues operating normally

#### Scenario: Operator selects OMP summaries
- **WHEN** the host operator selects the OMP summary adapter and enables summary generation
- **THEN** the CLI validates that the installed OMP supports every isolation flag before advertising summary capability, and leaves summaries unavailable with a logged reason otherwise
- **AND** each job runs OMP once, in a fresh empty directory, with the prompt on standard input, writes no OMP session, and loads no context file, extension, skill, rule or MCP server
- **AND** an `@path` or `^model` mention inside the summarized records is not expanded
