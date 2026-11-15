//! Skills: which of those plugins offer each session gets, and switching them in
//! one thread.

use qcc_domain::SkillRef;
use qcc_protocol::jsonrpc::{RpcError, codes};
use qcc_protocol::methods::{SkillListResult, SkillStatus, ThreadResult, ThreadSetSkillParams};

use super::{Core, now_ms, thread_not_found};
use crate::plugins::OfferedSkill;

impl Core {
    /// Whether a session of a thread with these choices gets the skill.
    fn takes(&self, skill: &OfferedSkill, enabled: &[String], disabled: &[String]) -> bool {
        let name = &skill.reference.name;
        let off = self.settings.skills.get(name).is_some_and(|settings| settings.disabled);
        !off && if skill.on_request { enabled.contains(name) } else { !disabled.contains(name) }
    }

    /// The skills a session of a thread with these choices gets.
    pub(super) fn session_skills(&self, enabled: &[String], disabled: &[String]) -> Vec<SkillRef> {
        self.skills
            .iter()
            .filter(|skill| self.takes(skill, enabled, disabled))
            .map(|skill| skill.reference.clone())
            .collect()
    }

    /// Every name must be a skill some plugin offers.
    pub(super) fn check_skills<'a>(
        &self,
        names: impl IntoIterator<Item = &'a String>,
    ) -> Result<(), RpcError> {
        for name in names {
            if !self.skills.iter().any(|skill| &skill.reference.name == name) {
                return Err(RpcError::new(
                    codes::INVALID_PARAMS,
                    format!(
                        "no plugin offers a skill called {name:?}; `qcc skill list` lists them"
                    ),
                ));
            }
        }
        Ok(())
    }

    pub(super) fn skill_list(&self) -> SkillListResult {
        let mut skills: Vec<SkillStatus> = self
            .skills
            .iter()
            .map(|skill| SkillStatus {
                name: skill.reference.name.clone(),
                description: skill.reference.description.clone(),
                plugin: skill.plugin.clone(),
                on_request: skill.on_request,
                disabled: self
                    .settings
                    .skills
                    .get(&skill.reference.name)
                    .is_some_and(|settings| settings.disabled),
                path: skill.reference.path.clone(),
            })
            .collect();
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        SkillListResult { skills }
    }

    pub(super) fn thread_set_skill(
        &mut self,
        p: ThreadSetSkillParams,
    ) -> Result<ThreadResult, RpcError> {
        let ThreadSetSkillParams { thread_id, skill: name, enabled } = p;
        self.check_skills([&name])?;
        self.check_open(&thread_id)?;
        let thread =
            self.supervisor.thread(&thread_id).ok_or_else(|| thread_not_found(&thread_id))?;
        let skill = self
            .skills
            .iter()
            .find(|skill| skill.reference.name == name)
            .cloned()
            .expect("checked above");
        let had = self.takes(&skill, &thread.skills_enabled, &thread.skills_disabled);
        // A skill every session gets is switched by leaving it out; one offered on
        // request by asking for it. Either way the other list forgets it.
        let mut on: Vec<String> =
            thread.skills_enabled.iter().filter(|n| **n != name).cloned().collect();
        let mut off: Vec<String> =
            thread.skills_disabled.iter().filter(|n| **n != name).cloned().collect();
        match (skill.on_request, enabled) {
            (true, true) => on.push(name.clone()),
            (false, false) => off.push(name.clone()),
            (true, false) | (false, true) => {}
        }
        let has = self.takes(&skill, &on, &off);
        let engine =
            self.supervisor.engine(&thread_id).map_err(|_| thread_not_found(&thread_id))?;
        let events = engine.set_skills(on, off, now_ms());
        self.commit(events);
        // A running session hears of a skill it can use now with the next message;
        // one it should stop using is left to the next session, which is not told
        // of it at all: what the agent has already read cannot be taken back.
        if has && !had {
            let SkillRef { name, description, path } = skill.reference;
            self.add_note(
                &thread_id,
                format!(
                    "qcc now offers the skill {name}: {description} It is instructions kept in \
                     {path}; when a task matches, read the file first and follow it."
                ),
            );
        }
        let thread = self.supervisor.thread(&thread_id).cloned();
        thread.map(|thread| ThreadResult { thread }).ok_or_else(|| thread_not_found(&thread_id))
    }
}
