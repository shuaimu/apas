//! New panes in flight: the ids handed to panes their instance has not
//! reported yet, and panes waiting for their machine's instance to start.
//!
//! The server allocates every pane id against the whole project, so ids are
//! unique across machines. Until the instance that receives a new pane
//! reports it, its id is only reserved here. Without that reservation two
//! quick adds could both draw an id the roster does not list yet, and the
//! second pane would collide with the first on whichever host got both.
//!
//! A pane requested on a machine where the project is registered but not
//! running waits here for that machine's instance. Its daemon is asked to
//! start the project, answers whether it did, and the instance's
//! `SessionStart` lets the pane through. Pending starts live in memory only:
//! a server restart drops them, and the client shows the launch as
//! unconfirmed rather than resending it.

use super::{SessionManager, SessionState};
use std::collections::HashSet;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;
use uuid::Uuid;

/// Pane ids the server allocates to new panes. 1 and 2 are the legacy
/// deadloop and interactive panes.
const FIRST_ALLOCATED_PANE_ID: u32 = 3;
const LAST_ALLOCATED_PANE_ID: u32 = 1002;
/// Native task launches derive their pane id from the request id instead,
/// so a retried launch finds the pane it already created. Their range starts
/// above every allocated id, so the two never collide.
const NATIVE_TASK_PANE_ID_BASE: u32 = 10_000;
const NATIVE_TASK_PANE_ID_SPAN: u128 = 1_000_000;

/// How long an allocated id stays reserved when its instance never reports
/// the pane.
pub const PANE_ID_RESERVATION_TTL: Duration = Duration::from_secs(60);
/// How long a pane waits for its machine's instance to start.
pub const INSTANCE_START_TIMEOUT: Duration = Duration::from_secs(90);

/// The pane id of the native task launched by `request_id`.
pub fn native_task_pane_id(request_id: Uuid) -> u32 {
    NATIVE_TASK_PANE_ID_BASE + (request_id.as_u128() % NATIVE_TASK_PANE_ID_SPAN) as u32
}

/// An id from the allocation range that `used` does not hold, or `None` when
/// the range is exhausted.
///
/// Ids used to be drawn blindly from the range, so a new pane could be
/// assigned an in-use id. The project host then discarded the creation in its
/// idempotent-duplicate guard and the tab never appeared, with no error
/// surfaced anywhere: the pane simply vanished between the click and the tab.
/// Random draws keep a freed id from being handed straight back out; the scan
/// after them finds the last free ids of a nearly full range.
fn pick_pane_id(used: &HashSet<u32>) -> Option<u32> {
    let span = LAST_ALLOCATED_PANE_ID - FIRST_ALLOCATED_PANE_ID + 1;
    for _ in 0..100 {
        let candidate = FIRST_ALLOCATED_PANE_ID + (Uuid::new_v4().as_u128() % span as u128) as u32;
        if !used.contains(&candidate) {
            return Some(candidate);
        }
    }
    (FIRST_ALLOCATED_PANE_ID..=LAST_ALLOCATED_PANE_ID).find(|candidate| !used.contains(candidate))
}

/// An id handed to a new pane that its instance has not reported yet.
#[derive(Debug, Clone)]
pub struct PaneIdReservation {
    /// The machine the pane was sent to, or `None` for an untagged pane sent
    /// to an instance without the capability.
    pub machine_id: Option<Uuid>,
    pub reserved_at: Instant,
}

impl SessionState {
    fn prune_pane_reservations(&mut self, ttl: Duration, now: Instant) {
        self.pane_reservations
            .retain(|_, reservation| now.duration_since(reservation.reserved_at) < ttl);
    }
}

/// How long ids stay reserved and pane launches wait for an instance.
/// Constants in production; tests shorten them.
#[derive(Debug, Clone, Copy)]
pub(super) struct PaneLaunchTiming {
    reservation_ttl: Duration,
    start_timeout: Duration,
}

impl Default for PaneLaunchTiming {
    fn default() -> Self {
        Self {
            reservation_ttl: PANE_ID_RESERVATION_TTL,
            start_timeout: INSTANCE_START_TIMEOUT,
        }
    }
}

/// What became of a machine's instance start that new panes wait on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceStartOutcome {
    /// The instance attached and the server finished its start, so it holds
    /// the project's policy and accepts new panes.
    Started,
    /// The daemon did not start it, for the reason it gave.
    Refused(String),
    /// The daemon's connection closed or was replaced before the instance
    /// attached, so nothing will answer the start.
    DaemonGone,
}

/// The answer to [`SessionManager::wait_for_instance_start`].
#[derive(Debug)]
pub enum InstanceStartWait {
    /// The machine's instance is running: send the pane to it now.
    Ready,
    /// Wait for `outcome` (see [`SessionManager::await_instance_start`]).
    /// `start` is the request id of a new start the caller must send to the
    /// machine's daemon as `StartProjectCli`. `None` sends nothing: a start
    /// is in flight already, or the instance has attached and the server is
    /// still finishing its start.
    Waiting {
        waiter: Uuid,
        start: Option<Uuid>,
        outcome: oneshot::Receiver<InstanceStartOutcome>,
    },
}

/// See [`SessionManager::withdraw_on_drop`].
pub struct WithdrawOnDrop<'a> {
    sessions: &'a SessionManager,
    waiter: Uuid,
}

impl Drop for WithdrawOnDrop<'_> {
    fn drop(&mut self) {
        self.sessions.cancel_instance_wait(self.waiter);
    }
}

/// One machine's instance being started, and the panes waiting for it.
#[derive(Debug)]
struct PendingInstanceStart {
    project_id: String,
    /// The `StartProjectCli.request_id` its daemon answers, or `None` for an
    /// instance that attached on its own and whose start the server is still
    /// finishing.
    request_id: Option<Uuid>,
    waiters: Vec<(Uuid, oneshot::Sender<InstanceStartOutcome>)>,
}

#[derive(Debug, Default)]
pub(super) struct InstanceStarts {
    /// By (session, machine).
    pending: std::collections::HashMap<(Uuid, Uuid), PendingInstanceStart>,
    /// The connection of each (session, machine) instance whose start the
    /// server finished. A pane that finds no instance on its machine and then
    /// asks to wait may come just after the start it would have waited for;
    /// this is how it learns the instance is already running.
    started: std::collections::HashMap<(Uuid, Uuid), Uuid>,
}

impl InstanceStarts {
    /// Remove the start for `key` and tell every waiting pane `outcome`.
    /// Done under the registry's lock, so that a waiter giving up at the same
    /// moment either is answered or was already gone.
    fn finish(&mut self, key: (Uuid, Uuid), outcome: InstanceStartOutcome) -> usize {
        let Some(pending) = self.pending.remove(&key) else {
            return 0;
        };
        let count = pending.waiters.len();
        for (_, waiter) in pending.waiters {
            let _ = waiter.send(outcome.clone());
        }
        count
    }
}

impl SessionManager {
    fn pane_launch_timing(&self) -> PaneLaunchTiming {
        *self
            .pane_launch_timing
            .lock()
            .expect("pane launch timing poisoned")
    }

    #[cfg(test)]
    pub(crate) fn set_pane_launch_timing(
        &self,
        reservation_ttl: Duration,
        start_timeout: Duration,
    ) {
        *self
            .pane_launch_timing
            .lock()
            .expect("pane launch timing poisoned") = PaneLaunchTiming {
            reservation_ttl,
            start_timeout,
        };
    }

    /// How long a pane waits for its machine's instance to start.
    pub fn instance_start_timeout(&self) -> Duration {
        self.pane_launch_timing().start_timeout
    }

    /// Allocate a pane id for a new pane on `machine_id` (`None` for an
    /// untagged pane), against the project's whole roster and the ids still
    /// in flight, and reserve it. `None` when every id is taken or the session
    /// is not loaded.
    pub fn allocate_pane_id(&self, session_id: &Uuid, machine_id: Option<Uuid>) -> Option<u32> {
        let ttl = self.pane_launch_timing().reservation_ttl;
        let now = Instant::now();
        let mut session = self.sessions.get_mut(session_id)?;
        session.prune_pane_reservations(ttl, now);
        let used: HashSet<u32> = session
            .panes
            .iter()
            .map(|pane| pane.pane_id)
            .chain(session.pane_reservations.keys().copied())
            .collect();
        let pane_id = pick_pane_id(&used)?;
        session.pane_reservations.insert(
            pane_id,
            PaneIdReservation {
                machine_id,
                reserved_at: now,
            },
        );
        Some(pane_id)
    }

    /// Reserve `pane_id`, chosen by its caller, for a new pane on
    /// `machine_id`: a native task launch, whose id comes from its request.
    pub fn reserve_pane_id(&self, session_id: &Uuid, pane_id: u32, machine_id: Option<Uuid>) {
        let ttl = self.pane_launch_timing().reservation_ttl;
        let now = Instant::now();
        if let Some(mut session) = self.sessions.get_mut(session_id) {
            session.prune_pane_reservations(ttl, now);
            session.pane_reservations.insert(
                pane_id,
                PaneIdReservation {
                    machine_id,
                    reserved_at: now,
                },
            );
        }
    }

    /// Give back an id whose pane was never delivered.
    pub fn release_pane_id(&self, session_id: &Uuid, pane_id: u32) {
        if let Some(mut session) = self.sessions.get_mut(session_id) {
            session.pane_reservations.remove(&pane_id);
        }
    }

    /// `cli_id` reported its roster: the ids reserved for its machine that it
    /// lists are no longer in flight, because the roster now holds them.
    pub fn clear_reported_pane_reservations(
        &self,
        session_id: &Uuid,
        cli_id: &Uuid,
        reported: &HashSet<u32>,
    ) {
        let Some(reporter) = self.instance_for_cli(session_id, cli_id) else {
            return;
        };
        if let Some(mut session) = self.sessions.get_mut(session_id) {
            session.pane_reservations.retain(|pane_id, reservation| {
                !(reported.contains(pane_id)
                    && (reservation.machine_id.is_none()
                        || reservation.machine_id == reporter.machine_id))
            });
        }
    }

    /// Wait for `machine_id`'s instance of `session_id` to finish starting.
    ///
    /// [`InstanceStartWait::Ready`] when that instance is attached and the
    /// server finished its start. A pane joins a start in flight, and waits
    /// for an attached instance whose start the server is still finishing:
    /// sent sooner, it could reach that CLI before the project's policy, and
    /// a CLI refuses every new pane until it holds one. With nothing attached
    /// and `start` naming the project, a new start begins and the caller
    /// sends it to the machine's daemon. With neither, the instance left since
    /// the caller looked; the answer is `Ready`, and the caller finds it gone.
    pub fn wait_for_instance_start(
        &self,
        session_id: Uuid,
        machine_id: Uuid,
        start: Option<&str>,
    ) -> InstanceStartWait {
        let key = (session_id, machine_id);
        let mut starts = self.instance_starts.lock().expect("instance starts poisoned");
        let (sender, outcome) = oneshot::channel();
        let waiter = Uuid::new_v4();
        if let Some(pending) = starts.pending.get_mut(&key) {
            pending.waiters.push((waiter, sender));
            return InstanceStartWait::Waiting {
                waiter,
                start: None,
                outcome,
            };
        }
        let attached = self
            .instance_on_machine(&session_id, &machine_id)
            .map(|instance| instance.cli_id);
        let request_id = match (attached, start) {
            (Some(cli_id), _) if starts.started.get(&key) == Some(&cli_id) => {
                return InstanceStartWait::Ready
            }
            (Some(_), _) => None,
            (None, Some(_)) => Some(Uuid::new_v4()),
            (None, None) => return InstanceStartWait::Ready,
        };
        starts.pending.insert(
            key,
            PendingInstanceStart {
                project_id: start.unwrap_or_default().to_string(),
                request_id,
                waiters: vec![(waiter, sender)],
            },
        );
        InstanceStartWait::Waiting {
            waiter,
            start: request_id,
            outcome,
        }
    }

    /// Wait up to `limit` for a start's outcome. `None` when none came in
    /// time: the waiter is then withdrawn, and the start abandoned once no
    /// pane waits on it. A wait dropped midway withdraws its waiter too.
    pub async fn await_instance_start(
        &self,
        waiter: Uuid,
        mut outcome: oneshot::Receiver<InstanceStartOutcome>,
        limit: Duration,
    ) -> Option<InstanceStartOutcome> {
        let _withdraw = self.withdraw_on_drop(waiter);
        match tokio::time::timeout(limit, &mut outcome).await {
            Ok(result) => result.ok(),
            // An outcome sent while this wait was timing out is still in the
            // channel: it is sent under the lock this withdrawal takes.
            Err(_) if self.cancel_instance_wait(waiter) => None,
            Err(_) => outcome.try_recv().ok(),
        }
    }

    /// Withdraw `waiter` when the returned guard is dropped, unless an
    /// outcome reached it first. A request that stops waiting midway, such as
    /// one whose client went away, otherwise leaves a waiter behind that keeps
    /// its start registered: later panes for that machine would join a start
    /// nothing answers, instead of sending a new one.
    pub fn withdraw_on_drop(&self, waiter: Uuid) -> WithdrawOnDrop<'_> {
        WithdrawOnDrop {
            sessions: self,
            waiter,
        }
    }

    /// Withdraw a waiting pane. False when an outcome was already sent to it.
    pub fn cancel_instance_wait(&self, waiter: Uuid) -> bool {
        let mut starts = self.instance_starts.lock().expect("instance starts poisoned");
        let found = starts.pending.iter().find_map(|(key, pending)| {
            pending
                .waiters
                .iter()
                .position(|(id, _)| *id == waiter)
                .map(|index| (*key, index))
        });
        let Some((key, index)) = found else {
            return false;
        };
        if let Some(pending) = starts.pending.get_mut(&key) {
            pending.waiters.remove(index);
            if pending.waiters.is_empty() {
                starts.pending.remove(&key);
            }
        }
        true
    }

    /// The server finished `cli_id`'s `SessionStart` as `machine_id`'s
    /// instance of `session_id`, policy included: the panes waiting for it
    /// can go to it now.
    pub fn instance_started(&self, session_id: Uuid, machine_id: Uuid, cli_id: Uuid) {
        let key = (session_id, machine_id);
        let mut starts = self.instance_starts.lock().expect("instance starts poisoned");
        starts.started.insert(key, cli_id);
        let released = starts.finish(key, InstanceStartOutcome::Started);
        if released > 0 {
            tracing::info!(
                %session_id,
                %machine_id,
                %cli_id,
                released,
                "instance started for waiting panes"
            );
        }
    }

    /// `machine_id`'s daemon answered the start `request_id`. A refusal fails
    /// every pane waiting on it; a success only says the runtime started, so
    /// they keep waiting for its instance to attach. False when no pending
    /// start matches, such as one every waiting pane gave up on.
    pub fn instance_start_result(
        &self,
        machine_id: Uuid,
        request_id: Uuid,
        project_id: &str,
        success: bool,
        error: Option<String>,
    ) -> bool {
        let host = self
            .machine_hostname(&machine_id)
            .unwrap_or_else(|| "The machine".to_string());
        let reason = error
            .map(|error| error.trim().to_string())
            .filter(|error| !error.is_empty())
            .unwrap_or_else(|| format!("{host} did not start this project"));
        let mut starts = self.instance_starts.lock().expect("instance starts poisoned");
        let Some((key, expected_project)) = starts
            .pending
            .iter()
            .find(|(key, pending)| key.1 == machine_id && pending.request_id == Some(request_id))
            .map(|(key, pending)| (*key, pending.project_id.clone()))
        else {
            return false;
        };
        if expected_project != project_id {
            tracing::warn!(
                %machine_id,
                %request_id,
                expected = %expected_project,
                reported = %project_id,
                "ignoring a start result for another project"
            );
            return false;
        }
        if !success {
            starts.finish(key, InstanceStartOutcome::Refused(reason));
        }
        true
    }

    /// The start `request_id` could not be sent to `machine_id`'s daemon.
    pub fn fail_instance_start(&self, session_id: Uuid, machine_id: Uuid, request_id: Uuid) {
        let key = (session_id, machine_id);
        let mut starts = self.instance_starts.lock().expect("instance starts poisoned");
        if starts
            .pending
            .get(&key)
            .is_some_and(|pending| pending.request_id == Some(request_id))
        {
            starts.finish(key, InstanceStartOutcome::DaemonGone);
        }
    }

    /// `machine_id`'s daemon connection closed or was replaced. Starts sent
    /// on it will not be answered on another, so the panes waiting on them
    /// fail now rather than at their timeout.
    pub(super) fn fail_instance_starts_for_machine(&self, machine_id: &Uuid) {
        let mut starts = self.instance_starts.lock().expect("instance starts poisoned");
        let keys: Vec<(Uuid, Uuid)> = starts
            .pending
            .keys()
            .filter(|key| key.1 == *machine_id)
            .copied()
            .collect();
        for key in keys {
            let failed = starts.finish(key, InstanceStartOutcome::DaemonGone);
            tracing::info!(
                session_id = %key.0,
                %machine_id,
                failed,
                "daemon left while an instance start was pending"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::AttachOutcome;
    use shared::{PaneConfig, ServerToCli, PROJECT_INSTANCES_CAPABILITY};
    use tokio::sync::mpsc;

    fn pane(pane_id: u32) -> PaneConfig {
        let mut pane = PaneConfig::defaults().remove(0);
        pane.pane_id = pane_id;
        pane.kind = shared::PaneKind::Terminal;
        pane
    }

    /// A session whose roster takes every allocatable id except `free`, so
    /// allocation is deterministic.
    fn crowded_session(manager: &SessionManager, free: &[u32]) -> Uuid {
        let session_id = Uuid::new_v4();
        manager.create_session(session_id, Uuid::new_v4(), Uuid::new_v4());
        manager.set_session_panes(
            &session_id,
            (FIRST_ALLOCATED_PANE_ID..=LAST_ALLOCATED_PANE_ID)
                .filter(|pane_id| !free.contains(pane_id))
                .map(pane)
                .collect(),
        );
        session_id
    }

    fn capable_instance(
        manager: &SessionManager,
        session_id: Uuid,
        machine_id: Uuid,
    ) -> (Uuid, mpsc::Receiver<ServerToCli>) {
        let cli_id = Uuid::new_v4();
        let (tx, rx) = mpsc::channel(8);
        manager.register_cli(cli_id, Uuid::new_v4(), tx, None);
        manager.set_cli_capabilities(cli_id, vec![PROJECT_INSTANCES_CAPABILITY.to_string()]);
        assert!(matches!(
            manager.attach_cli_instance(session_id, cli_id, Some(machine_id), None, None),
            AttachOutcome::Attached { .. }
        ));
        (cli_id, rx)
    }

    #[test]
    fn fresh_session_allocates_inside_the_user_range() {
        for _ in 0..50 {
            let id = pick_pane_id(&HashSet::new()).expect("empty range has room");
            assert!((3..=1002).contains(&id), "reserved ids stay reserved: {id}");
        }
    }

    #[test]
    fn allocation_never_reuses_an_in_use_id() {
        let used: HashSet<u32> = [3, 226, 676, 1002].into_iter().collect();
        for _ in 0..100 {
            let id = pick_pane_id(&used).expect("range has room");
            assert!(!used.contains(&id), "collided with an in-use pane: {id}");
        }
    }

    #[test]
    fn exhausted_range_reports_none_instead_of_reusing() {
        let used: HashSet<u32> = (3..=1002).collect();
        assert_eq!(pick_pane_id(&used), None);
        let nearly: HashSet<u32> = (3..=1002).filter(|id| *id != 517).collect();
        assert_eq!(pick_pane_id(&nearly), Some(517), "the last free id is found");
    }

    /// Allocated ids and native task ids can never meet.
    #[test]
    fn native_task_ids_stay_above_every_allocated_id() {
        for _ in 0..1000 {
            let id = native_task_pane_id(Uuid::new_v4());
            assert!(id > LAST_ALLOCATED_PANE_ID && id >= 10_000, "{id}");
        }
        let request = Uuid::new_v4();
        assert_eq!(native_task_pane_id(request), native_task_pane_id(request));
    }

    /// Two allocations before any roster report lists either pane get
    /// different ids, and an id stays taken until its machine reports it.
    #[test]
    fn an_allocated_id_is_reserved_until_its_machine_reports_it() {
        let manager = SessionManager::new();
        let session_id = crowded_session(&manager, &[40, 41]);
        let (m5, m6) = (Uuid::new_v4(), Uuid::new_v4());
        let (cli5, _rx5) = capable_instance(&manager, session_id, m5);
        let (cli6, _rx6) = capable_instance(&manager, session_id, m6);

        let first = manager.allocate_pane_id(&session_id, Some(m5)).unwrap();
        let second = manager.allocate_pane_id(&session_id, Some(m6)).unwrap();
        assert_ne!(first, second);
        assert_eq!(manager.allocate_pane_id(&session_id, Some(m5)), None, "both are in flight");

        // Another machine listing the id does not clear zoo-005's reservation.
        manager.clear_reported_pane_reservations(&session_id, &cli6, &HashSet::from([first]));
        // zoo-005 reports its pane: the roster holds it now, and the
        // reservation is gone, so the id is free again once the pane closes.
        manager.clear_reported_pane_reservations(&session_id, &cli5, &HashSet::from([first]));
        assert_eq!(manager.allocate_pane_id(&session_id, None), Some(first));
        manager.release_pane_id(&session_id, first);
        assert_eq!(
            manager.allocate_pane_id(&session_id, None),
            Some(first),
            "a released id is free"
        );
    }

    /// A reservation whose pane never appears expires.
    #[test]
    fn a_reservation_expires_when_its_pane_never_appears() {
        let manager = SessionManager::new();
        manager.set_pane_launch_timing(Duration::from_millis(30), INSTANCE_START_TIMEOUT);
        let session_id = crowded_session(&manager, &[77]);
        assert_eq!(manager.allocate_pane_id(&session_id, None), Some(77));
        assert_eq!(manager.allocate_pane_id(&session_id, None), None);
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(manager.allocate_pane_id(&session_id, None), Some(77));
    }

    /// Several panes waiting on one machine share one start; the instance's
    /// start releases all of them.
    #[tokio::test]
    async fn panes_waiting_on_one_machine_share_its_start() {
        let manager = SessionManager::new();
        let session_id = Uuid::new_v4();
        let machine_id = Uuid::new_v4();
        let project = session_id.to_string();

        let InstanceStartWait::Waiting {
            waiter: first,
            start: Some(request_id),
            outcome: first_outcome,
        } = manager.wait_for_instance_start(session_id, machine_id, Some(&project))
        else {
            panic!("the first pane begins a start")
        };
        let InstanceStartWait::Waiting {
            waiter: second,
            start: None,
            outcome: second_outcome,
        } = manager.wait_for_instance_start(session_id, machine_id, Some(&project))
        else {
            panic!("the second pane joins it")
        };

        // Success only says the runtime started; nothing is released yet.
        assert!(manager.instance_start_result(machine_id, request_id, &project, true, None));
        let (cli_id, _rx) = capable_instance(&manager, session_id, machine_id);
        manager.instance_started(session_id, machine_id, cli_id);
        for (waiter, outcome) in [(first, first_outcome), (second, second_outcome)] {
            assert_eq!(
                manager
                    .await_instance_start(waiter, outcome, Duration::from_secs(1))
                    .await,
                Some(InstanceStartOutcome::Started)
            );
        }
        // Once it has started, a later pane goes straight to it.
        assert!(matches!(
            manager.wait_for_instance_start(session_id, machine_id, Some(&project)),
            InstanceStartWait::Ready
        ));
    }

    /// A refusal fails every pane waiting on that start with the daemon's
    /// reason; one for another start, machine or project changes nothing.
    #[tokio::test]
    async fn a_refused_start_fails_its_waiting_panes() {
        let manager = SessionManager::new();
        let session_id = Uuid::new_v4();
        let machine_id = Uuid::new_v4();
        let project = session_id.to_string();
        let InstanceStartWait::Waiting {
            waiter,
            start: Some(request_id),
            outcome,
        } = manager.wait_for_instance_start(session_id, machine_id, Some(&project))
        else {
            panic!("expected a new start")
        };

        assert!(!manager.instance_start_result(machine_id, Uuid::new_v4(), &project, false, None));
        assert!(!manager.instance_start_result(Uuid::new_v4(), request_id, &project, false, None));
        assert!(!manager.instance_start_result(
            machine_id,
            request_id,
            "other-project",
            false,
            None
        ));
        assert!(manager.instance_start_result(
            machine_id,
            request_id,
            &project,
            false,
            Some("Project policy is suspended".to_string()),
        ));
        assert_eq!(
            manager
                .await_instance_start(waiter, outcome, Duration::from_secs(1))
                .await,
            Some(InstanceStartOutcome::Refused(
                "Project policy is suspended".to_string()
            ))
        );
    }

    /// The daemon leaving fails the panes waiting on its machine only.
    #[tokio::test]
    async fn a_departing_daemon_fails_only_its_machines_starts() {
        let manager = SessionManager::new();
        let session_id = Uuid::new_v4();
        let (gone, staying) = (Uuid::new_v4(), Uuid::new_v4());
        let project = session_id.to_string();
        let wait = |machine_id| {
            match manager.wait_for_instance_start(session_id, machine_id, Some(&project)) {
                InstanceStartWait::Waiting { waiter, outcome, .. } => (waiter, outcome),
                InstanceStartWait::Ready => panic!("expected a start"),
            }
        };
        let (gone_waiter, gone_outcome) = wait(gone);
        let (staying_waiter, staying_outcome) = wait(staying);

        manager.unregister_daemon(&gone);

        assert_eq!(
            manager
                .await_instance_start(gone_waiter, gone_outcome, Duration::from_secs(1))
                .await,
            Some(InstanceStartOutcome::DaemonGone)
        );
        assert_eq!(
            manager
                .await_instance_start(staying_waiter, staying_outcome, Duration::from_millis(50))
                .await,
            None,
            "the other machine's start is still pending"
        );
    }

    /// A pane that stops waiting withdraws; the start is abandoned with its
    /// last pane, so a later one begins a fresh start.
    #[tokio::test]
    async fn a_timed_out_pane_withdraws_from_the_start() {
        let manager = SessionManager::new();
        let session_id = Uuid::new_v4();
        let machine_id = Uuid::new_v4();
        let project = session_id.to_string();
        let InstanceStartWait::Waiting {
            waiter,
            start: Some(first_request),
            outcome,
        } = manager.wait_for_instance_start(session_id, machine_id, Some(&project))
        else {
            panic!("expected a new start")
        };
        assert_eq!(
            manager
                .await_instance_start(waiter, outcome, Duration::from_millis(20))
                .await,
            None
        );
        assert!(!manager.cancel_instance_wait(waiter), "already withdrawn");
        assert!(!manager.instance_start_result(machine_id, first_request, &project, false, None));
        assert!(matches!(
            manager.wait_for_instance_start(session_id, machine_id, Some(&project)),
            InstanceStartWait::Waiting { start: Some(request), .. } if request != first_request
        ));
    }

    /// A wait abandoned midway, such as a request whose client went away,
    /// withdraws its pane, so the next pane for that machine starts afresh
    /// rather than joining a start nobody waits on.
    #[tokio::test]
    async fn an_abandoned_wait_does_not_keep_its_start() {
        let manager = std::sync::Arc::new(SessionManager::new());
        let session_id = Uuid::new_v4();
        let machine_id = Uuid::new_v4();
        let project = session_id.to_string();
        let InstanceStartWait::Waiting {
            waiter,
            start: Some(first_request),
            outcome,
        } = manager.wait_for_instance_start(session_id, machine_id, Some(&project))
        else {
            panic!("expected a new start")
        };
        let waiting = {
            let manager = manager.clone();
            tokio::spawn(async move {
                manager
                    .await_instance_start(waiter, outcome, Duration::from_secs(60))
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        waiting.abort();
        assert!(waiting.await.unwrap_err().is_cancelled());

        let guarded = manager.wait_for_instance_start(session_id, machine_id, Some(&project));
        let InstanceStartWait::Waiting {
            waiter: second,
            start: Some(second_request),
            ..
        } = guarded
        else {
            panic!("the abandoned start is gone, so this one begins afresh")
        };
        assert_ne!(second_request, first_request);
        drop(manager.withdraw_on_drop(second));
        assert!(!manager.instance_start_result(machine_id, second_request, &project, false, None));
    }

    /// An instance that finished starting before a pane asked to wait for it
    /// is not waited for, but one still starting is.
    #[tokio::test]
    async fn a_start_that_finished_first_is_not_waited_for() {
        let manager = SessionManager::new();
        let session_id = Uuid::new_v4();
        let machine_id = Uuid::new_v4();
        let project = session_id.to_string();
        let (cli_id, _rx) = capable_instance(&manager, session_id, machine_id);

        // Attached, but the server has not finished its start: wait for it,
        // without asking the daemon for another.
        let InstanceStartWait::Waiting {
            waiter,
            start: None,
            outcome,
        } = manager.wait_for_instance_start(session_id, machine_id, None)
        else {
            panic!("an unfinished start is waited for")
        };
        manager.instance_started(session_id, machine_id, cli_id);
        assert_eq!(
            manager
                .await_instance_start(waiter, outcome, Duration::from_secs(1))
                .await,
            Some(InstanceStartOutcome::Started)
        );
        assert!(matches!(
            manager.wait_for_instance_start(session_id, machine_id, Some(&project)),
            InstanceStartWait::Ready
        ));

        // A replacement connection on the same machine starts afresh.
        let (replacement, _rx2) = capable_instance(&manager, session_id, machine_id);
        assert!(matches!(
            manager.wait_for_instance_start(session_id, machine_id, Some(&project)),
            InstanceStartWait::Waiting { start: None, .. }
        ));
        manager.instance_started(session_id, machine_id, replacement);
        assert!(matches!(
            manager.wait_for_instance_start(session_id, machine_id, None),
            InstanceStartWait::Ready
        ));
        // An instance that left meanwhile is not waited for; the caller finds
        // it gone.
        manager.detach_cli_instance(&session_id, &replacement).unwrap();
        assert!(matches!(
            manager.wait_for_instance_start(session_id, machine_id, None),
            InstanceStartWait::Ready
        ));
    }
}
