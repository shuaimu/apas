## 1. Server

- [x] 1.1 Make `authorize_existing_pane_launch` stop applying the launch-profile allowlist, keeping the retired-backend and team-availability refusals it shares with creation
- [x] 1.2 Stop the two CLI-reboot gates blocking on allowlist noncompliance, leaving the retired-backend block in place
- [x] 1.3 Leave `authorize_new_pane_launch`, the team launch path, and the model-change path enforcing it
- [x] 1.4 Tests: resume/reboot/start-bot on a pane outside the allowlist are authorized; creating that same profile is still refused; a retired backend and an unavailable team still refuse; a noncompliant pane no longer blocks a CLI reboot

## 2. Project host

- [x] 2.1 Stop `StartBot`, `ResumePane`, and `RebootPane` refusing on the allowlist, keeping their retired-provider and managed/team checks
- [x] 2.2 Leave `AddPane` and the model switch enforcing it
- [x] 2.3 Remove the noncompliance status message, which existed to explain a restriction that no longer applies
- [x] 2.4 Tests: the three actions proceed for a pane outside the allowlist; `AddPane` and the model switch still refuse

## 3. Web

- [x] 3.1 Remove the toast shown on entering a project with noncompliant panes
- [x] 3.2 Reword the policy card: those panes run and relaunch; their profile cannot be chosen for something new
- [x] 3.3 Tests: entering a project with noncompliant panes raises no toast, and the suspended-project toast still does

## 4. Verification

- [x] 4.1 `cargo test` and `cargo clippy` clean for the workspace; web lint and tests clean
- [ ] 4.2 Live: open a project with a pane outside the allowlist on mobile, confirm no notice, and reboot that pane successfully
