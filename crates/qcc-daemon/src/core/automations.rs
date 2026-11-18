//! Automations: the daemon sends an agent a prompt on a schedule.
//!
//! An automation is the user's own standing request, kept with projects and
//! settings, and each run does what a client does: it starts a thread, or sends a
//! message to the thread of earlier runs. One timer covers every automation: it
//! wakes the core when the earliest one is due, and at least once a minute so a
//! change of the wall clock (a suspended laptop, a time zone) is noticed.

use std::time::Duration;

use jiff::tz::TimeZone;
use qcc_domain::{
    Automation, AutomationId, AutomationOutcome, AutomationOverlap, AutomationRun, AutomationSpec,
    AutomationThreadMode, AutomationTrigger, ThreadId,
};
use qcc_protocol::jsonrpc::{RpcError, codes};
use qcc_protocol::methods::{
    AUTOMATIONS_UPDATED, AutomationEntry, AutomationIdParams, AutomationListResult,
    AutomationResult, AutomationRunResult, AutomationRunsParams, AutomationRunsResult,
    AutomationSetEnabledParams, AutomationUpdateParams, AutomationsUpdatedNotification, Empty,
    SendMode, ThreadSendParams, ThreadSpawnParams,
};

use super::{Command, Core, notify, now_ms};
use crate::schedule::Schedule;

/// The longest the timer sleeps: it runs on the monotonic clock, which stands still
/// while the computer sleeps, so the wall clock is looked at again this often.
const MAX_SLEEP: Duration = Duration::from_secs(60);

/// How late a scheduled run may start and still count as on time. Later than this
/// it was missed: the daemon was not running, or the computer slept. Never more
/// than half the time between runs, so a short interval is not run twice in a row.
const ON_TIME: u64 = 60_000;

fn invalid(message: impl Into<String>) -> RpcError {
    RpcError::new(codes::INVALID_PARAMS, message)
}

fn internal(error: impl std::fmt::Display) -> RpcError {
    RpcError::new(codes::INTERNAL_ERROR, error.to_string())
}

fn not_found(id: &AutomationId) -> RpcError {
    invalid(format!("automation {id} does not exist"))
}

/// The local time zone, looked up each time so a change of it is followed.
fn zone() -> TimeZone {
    TimeZone::system()
}

/// The next scheduled time of an automation that is switched on.
fn next_run(automation: &Automation) -> Option<u64> {
    if !automation.spec.enabled {
        return None;
    }
    let schedule = Schedule::parse(&automation.spec.schedule).ok()?;
    schedule.next_after(automation.anchor, &zone())
}

impl Core {
    pub(super) fn automation_list(&self) -> AutomationListResult {
        let automations = self.automations.iter().map(|a| self.automation_entry(a)).collect();
        AutomationListResult { automations }
    }

    fn automation_entry(&self, automation: &Automation) -> AutomationEntry {
        let last_run = match self.store.automation_runs(&automation.id, 1) {
            Ok(mut runs) => runs.pop(),
            Err(error) => {
                tracing::error!(automation = %automation.id, %error, "cannot read the runs");
                None
            }
        };
        AutomationEntry {
            automation: automation.clone(),
            next_run_at: next_run(automation),
            last_run,
        }
    }

    fn automation(&self, id: &AutomationId) -> Result<&Automation, RpcError> {
        self.automations.iter().find(|a| &a.id == id).ok_or_else(|| not_found(id))
    }

    /// What a run needs is there now: an automation that could never run is
    /// refused when it is made, not found out at night.
    fn check_automation(&self, spec: &mut AutomationSpec) -> Result<(), RpcError> {
        spec.name = spec.name.trim().to_owned();
        if spec.name.is_empty() {
            return Err(invalid("an automation needs a name"));
        }
        if spec.prompt.trim().is_empty() {
            return Err(invalid("an automation needs a prompt"));
        }
        let schedule = Schedule::parse(&spec.schedule).map_err(invalid)?;
        if schedule.next_after(now_ms(), &zone()).is_none() {
            return Err(invalid(format!("the schedule {:?} never comes", spec.schedule)));
        }
        spec.schedule = spec.schedule.trim().to_owned();
        self.check_mode(&spec.provider, spec.permission_mode)?;
        let cwd = std::path::Path::new(&spec.cwd);
        if !cwd.is_absolute() || !cwd.is_dir() {
            return Err(invalid(format!(
                "cwd {:?} is not an absolute path to a directory",
                spec.cwd
            )));
        }
        if let Some(environment) = &spec.environment
            && self.host.environment(environment).is_none()
        {
            return Err(invalid(format!("no plugin provides environment {environment:?}")));
        }
        Ok(())
    }

    pub(super) fn automation_create(
        &mut self,
        mut spec: AutomationSpec,
    ) -> Result<AutomationResult, RpcError> {
        self.check_automation(&mut spec)?;
        let now = now_ms();
        let automation = Automation {
            id: AutomationId(format!("aut_{}", uuid::Uuid::now_v7().simple())),
            spec,
            created_at: now,
            thread_id: None,
            anchor: now,
        };
        self.save_automation(automation.clone())?;
        Ok(AutomationResult { automation: self.automation_entry(&automation) })
    }

    pub(super) fn automation_update(
        &mut self,
        p: AutomationUpdateParams,
    ) -> Result<AutomationResult, RpcError> {
        let AutomationUpdateParams { id, mut spec } = p;
        let current = self.automation(&id)?.clone();
        self.check_automation(&mut spec)?;
        let mut automation = current.clone();
        // A new schedule, or one switched back on, counts from now: the runs it
        // would have had are not owed.
        if spec.schedule != current.spec.schedule || (spec.enabled && !current.spec.enabled) {
            automation.anchor = now_ms();
        }
        // The thread of earlier runs is of no use to runs that go elsewhere.
        let elsewhere = spec.provider != current.spec.provider
            || spec.cwd != current.spec.cwd
            || spec.environment != current.spec.environment
            || spec.thread_mode != AutomationThreadMode::Continue;
        if elsewhere {
            automation.thread_id = None;
        }
        automation.spec = spec;
        self.save_automation(automation.clone())?;
        Ok(AutomationResult { automation: self.automation_entry(&automation) })
    }

    pub(super) fn automation_set_enabled(
        &mut self,
        p: AutomationSetEnabledParams,
    ) -> Result<AutomationResult, RpcError> {
        let mut automation = self.automation(&p.id)?.clone();
        if automation.spec.enabled != p.enabled {
            automation.spec.enabled = p.enabled;
            if p.enabled {
                automation.anchor = now_ms();
            }
            self.save_automation(automation.clone())?;
        }
        Ok(AutomationResult { automation: self.automation_entry(&automation) })
    }

    pub(super) fn automation_delete(&mut self, p: AutomationIdParams) -> Result<Empty, RpcError> {
        self.automation(&p.id)?;
        self.store.delete_automation(&p.id).map_err(internal)?;
        self.automations.retain(|a| a.id != p.id);
        self.automations_changed();
        Ok(Empty {})
    }

    pub(super) fn automation_run_now(
        &mut self,
        p: AutomationIdParams,
    ) -> Result<AutomationRunResult, RpcError> {
        self.automation(&p.id)?;
        let run = self.run_automation(&p.id, AutomationTrigger::Manual, None);
        self.automations_changed();
        Ok(AutomationRunResult { run })
    }

    pub(super) fn automation_runs(
        &self,
        p: AutomationRunsParams,
    ) -> Result<AutomationRunsResult, RpcError> {
        self.automation(&p.id)?;
        let limit = p.limit.map_or(qcc_store::AUTOMATION_RUNS_KEPT, |limit| limit as usize);
        let runs = self.store.automation_runs(&p.id, limit).map_err(internal)?;
        Ok(AutomationRunsResult { runs })
    }

    fn save_automation(&mut self, automation: Automation) -> Result<(), RpcError> {
        self.store.save_automation(&automation).map_err(internal)?;
        match self.automations.iter_mut().find(|known| known.id == automation.id) {
            Some(known) => *known = automation,
            None => self.automations.push(automation),
        }
        self.automations_changed();
        Ok(())
    }

    /// Tell every connection that watches threads what the automations are now,
    /// and set the timer for the next one due.
    fn automations_changed(&mut self) {
        self.arm_automations();
        if self.watchers.is_empty() {
            return;
        }
        let automations = self.automation_list().automations;
        self.watchers.retain(|watcher| {
            let subscription = watcher.subscription;
            let params =
                AutomationsUpdatedNotification { subscription, automations: automations.clone() };
            notify(&watcher.out, AUTOMATIONS_UPDATED, params)
        });
    }

    /// Wake the core when the earliest automation is due, or in a minute. A timer
    /// set before is forgotten: its tick no longer matches.
    pub(super) fn arm_automations(&mut self) {
        self.automation_timer += 1;
        let Some(due) = self.automations.iter().filter_map(next_run).min() else {
            return;
        };
        let delay = Duration::from_millis(due.saturating_sub(now_ms())).min(MAX_SLEEP);
        let generation = self.automation_timer;
        let commands = self.commands.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = commands.send(Command::AutomationTick { generation });
        });
    }

    /// The timer went off: run what is due, then set it again.
    pub(super) fn automation_tick(&mut self, generation: u64) {
        if generation != self.automation_timer {
            return;
        }
        let now = now_ms();
        let zone = zone();
        let due: Vec<AutomationId> = self
            .automations
            .iter()
            .filter(|a| next_run(a).is_some_and(|at| at <= now))
            .map(|a| a.id.clone())
            .collect();
        let mut ran = false;
        for id in due {
            let Ok(automation) = self.automation(&id) else {
                continue;
            };
            let Ok(schedule) = Schedule::parse(&automation.spec.schedule) else {
                continue;
            };
            let Some(slot) = schedule.next_after(automation.anchor, &zone) else {
                continue;
            };
            let after = schedule.next_after(slot, &zone);
            let period = after.map_or(u64::MAX, |after| after - slot);
            let missed = now - slot > ON_TIME.min(period / 2);
            let catch_up = automation.spec.catch_up;
            // Runs missed in a row come to one: the latest slot passed.
            let anchor = if after.is_some_and(|after| after > now) { slot } else { now };
            let run = if !missed {
                self.run_automation(&id, AutomationTrigger::Schedule, Some(slot))
            } else if catch_up {
                self.run_automation(&id, AutomationTrigger::Missed, Some(slot))
            } else {
                let run = AutomationRun {
                    automation_id: id.clone(),
                    at: now,
                    scheduled_for: Some(slot),
                    trigger: AutomationTrigger::Missed,
                    outcome: AutomationOutcome::Skipped,
                    thread_id: None,
                    reason: Some(
                        "missed: the daemon was not running or the computer was asleep".into(),
                    ),
                };
                self.record_run(&run);
                run
            };
            tracing::info!(automation = %id, outcome = ?run.outcome, "automation came due");
            if let Some(automation) = self.automations.iter_mut().find(|a| a.id == id) {
                automation.anchor = anchor;
                let automation = automation.clone();
                if let Err(error) = self.store.save_automation(&automation) {
                    self.fatal = Some(anyhow::Error::new(error).context("cannot save automations"));
                    return;
                }
            }
            ran = true;
        }
        if ran {
            self.automations_changed();
        } else {
            self.arm_automations();
        }
    }

    /// The thread of the automation's previous run, if it is still open.
    fn previous_run(&self, automation: &Automation) -> Option<ThreadId> {
        if let Some(thread) = &automation.thread_id {
            return Some(thread.clone());
        }
        self.supervisor
            .threads()
            .filter(|thread| thread.automation.as_ref() == Some(&automation.id))
            .filter(|thread| !thread.archived)
            .max_by_key(|thread| (thread.created_at, thread.id.clone()))
            .map(|thread| thread.id.clone())
    }

    /// Send the automation's prompt, and record how that went.
    fn run_automation(
        &mut self,
        id: &AutomationId,
        trigger: AutomationTrigger,
        scheduled_for: Option<u64>,
    ) -> AutomationRun {
        let mut run = AutomationRun {
            automation_id: id.clone(),
            at: now_ms(),
            scheduled_for,
            trigger,
            outcome: AutomationOutcome::Started,
            thread_id: None,
            reason: None,
        };
        let Ok(automation) = self.automation(id).cloned() else {
            run.outcome = AutomationOutcome::Failed;
            run.reason = Some("the automation is gone".into());
            return run;
        };
        // The thread `continue` mode sends to, while it can take messages.
        let kept = automation
            .thread_id
            .clone()
            .filter(|thread| self.check_open(thread).is_ok())
            .filter(|_| automation.spec.thread_mode == AutomationThreadMode::Continue);
        let previous = self.previous_run(&automation);
        let working = previous
            .as_ref()
            .and_then(|thread| self.thread_state(thread))
            .is_some_and(|state| !state.is_settled());
        if working && automation.spec.overlap == AutomationOverlap::Skip {
            run.outcome = AutomationOutcome::Skipped;
            run.thread_id = previous.clone();
            run.reason = Some(format!(
                "the previous run ({}) is still at work",
                previous.map(|thread| thread.to_string()).unwrap_or_default()
            ));
            self.record_run(&run);
            return run;
        }

        let spec = &automation.spec;
        let result = match kept {
            Some(thread) => self
                .thread_send(ThreadSendParams {
                    thread_id: thread.clone(),
                    text: spec.prompt.clone(),
                    delivery: SendMode::Queue,
                    context: Vec::new(),
                    attachments: Vec::new(),
                })
                .map(|_| thread),
            None => self
                .spawn_thread(
                    ThreadSpawnParams {
                        provider: spec.provider.clone(),
                        cwd: spec.cwd.clone(),
                        environment: spec.environment.clone(),
                        base: None,
                        prompt: Some(spec.prompt.clone()),
                        attachments: Vec::new(),
                        model: spec.model.clone(),
                        effort: spec.effort.clone(),
                        parent: None,
                        project: None,
                        permission_mode: spec.permission_mode,
                        skills_enabled: Vec::new(),
                        skills_disabled: Vec::new(),
                        started_by: None,
                        hidden: false,
                    },
                    Some((automation.id.clone(), spec.name.clone())),
                )
                .map(|result| result.thread.id),
        };
        match result {
            Ok(thread) => {
                run.thread_id = Some(thread.clone());
                if spec.thread_mode == AutomationThreadMode::Continue
                    && automation.thread_id.as_ref() != Some(&thread)
                    && let Some(known) = self.automations.iter_mut().find(|a| &a.id == id)
                {
                    known.thread_id = Some(thread);
                    let known = known.clone();
                    if let Err(error) = self.store.save_automation(&known) {
                        tracing::error!(automation = %id, %error, "cannot save the automation");
                    }
                }
            }
            Err(error) => {
                run.outcome = AutomationOutcome::Failed;
                run.reason = Some(error.message);
            }
        }
        self.record_run(&run);
        run
    }

    fn record_run(&self, run: &AutomationRun) {
        if let Err(error) = self.store.add_automation_run(run) {
            tracing::error!(automation = %run.automation_id, %error, "cannot record the run");
        }
    }
}
