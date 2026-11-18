//! What plugins offer besides sessions: models, the `threads` import, commands and UI.

use qcc_domain::{AgentSkill, ModelInfo, ProviderId, ThreadId};
use qcc_plugin_host::{
    Contribution, Outcome, PluginRequest, ProviderRequest, ThreadsAnswer, ThreadsCall, UiRequest,
};
use qcc_protocol::jsonrpc::{RpcError, codes};
use qcc_protocol::methods::{
    CommandContribution, CommandListResult, CommandRunParams, CommandRunResult, Empty,
    ProviderModelsParams, ProviderModelsResult, ProviderSkillsParams, ProviderSkillsResult,
    SendMode, ThreadSendParams, ThreadSpawnParams, UiContributionsResult, UiItemContribution,
    UiTreeResult, UiViewContribution,
};
use serde_json::Value;

use super::{
    Command, Core, MODELS_TIMEOUT, QUERY_TIMEOUT, Responder, Watch, owns_kind, thread_not_found,
};

impl Core {
    pub(super) fn provider_models(&mut self, p: ProviderModelsParams, responder: Responder) {
        let ProviderModelsParams { provider, refresh } = p;
        if !refresh && let Some(models) = self.models.get(&provider) {
            return responder.reply(Ok(ProviderModelsResult { models: models.clone() }));
        }
        let pick = |outcome| match outcome {
            Outcome::Models(models) => Ok(models),
            other => Err(other),
        };
        let done =
            |provider, result, responder| Command::ModelsListed { provider, result, responder };
        self.list_from_provider(
            provider,
            ProviderRequest::ListModels,
            "models",
            pick,
            done,
            responder,
        );
    }

    pub(super) fn provider_skills(&mut self, p: ProviderSkillsParams, responder: Responder) {
        let ProviderSkillsParams { provider, refresh } = p;
        if !refresh && let Some(skills) = self.agent_skills.get(&provider) {
            return responder.reply(Ok(ProviderSkillsResult { skills: skills.clone() }));
        }
        let pick = |outcome| match outcome {
            Outcome::Skills(mut skills) => {
                skills.sort_by(|a: &AgentSkill, b| a.name.cmp(&b.name));
                Ok(skills)
            }
            other => Err(other),
        };
        let done =
            |provider, result, responder| Command::SkillsListed { provider, result, responder };
        self.list_from_provider(
            provider,
            ProviderRequest::ListSkills,
            "skills",
            pick,
            done,
            responder,
        );
    }

    /// Ask a provider for a list in the background; `done` brings the answer back
    /// to the core, which caches it and replies.
    fn list_from_provider<T: Send + 'static>(
        &self,
        provider: ProviderId,
        request: ProviderRequest,
        what: &'static str,
        pick: fn(Outcome) -> Result<Vec<T>, Outcome>,
        done: fn(ProviderId, Result<Vec<T>, String>, Responder) -> Command,
        responder: Responder,
    ) {
        let plugin = match self.provider_of(&provider) {
            Ok((plugin, _)) => plugin.clone(),
            Err(error) => return responder.reply::<Empty>(Err(error)),
        };
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let request = PluginRequest::Provider { provider: provider.clone(), request };
            let result = match tokio::time::timeout(MODELS_TIMEOUT, plugin.call(request)).await {
                Ok(Ok(outcome)) => {
                    pick(outcome).map_err(|other| format!("the provider answered with {other:?}"))
                }
                Ok(Err(error)) => Err(error.to_string()),
                Err(_) => Err(format!(
                    "the provider did not list its {what} within {} s",
                    MODELS_TIMEOUT.as_secs()
                )),
            };
            let _ = commands.send(done(provider, result, responder));
        });
    }

    pub(super) fn skills_listed(
        &mut self,
        provider: ProviderId,
        result: Result<Vec<AgentSkill>, String>,
        responder: Responder,
    ) {
        let reply = listed(&mut self.agent_skills, provider, result);
        responder.reply(reply.map(|skills| ProviderSkillsResult { skills }));
    }

    pub(super) fn models_listed(
        &mut self,
        provider: ProviderId,
        result: Result<Vec<ModelInfo>, String>,
        responder: Responder,
    ) {
        let reply = listed(&mut self.models, provider, result);
        responder.reply(reply.map(|models| ProviderModelsResult { models }));
    }

    pub(super) fn threads_call(
        &mut self,
        plugin: &str,
        call: ThreadsCall,
    ) -> Result<ThreadsAnswer, String> {
        let summary = |core: &mut Self, id: &ThreadId| {
            core.summary(id)
                .map(|summary| ThreadsAnswer::Thread(Box::new(summary)))
                .ok_or_else(|| format!("thread {id} does not exist"))
        };
        match call {
            ThreadsCall::Spawn(spawn) => {
                tracing::info!(plugin, provider = %spawn.provider, "a plugin spawns a thread");
                let cwd = match spawn.cwd {
                    Some(cwd) => cwd,
                    // A directory of the plugin's own, apart from its key-value store.
                    None => {
                        let dir = self.config.plugin_data_dir(plugin).join("threads");
                        std::fs::create_dir_all(&dir)
                            .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
                        dir.display().to_string()
                    }
                };
                let result = self
                    .thread_spawn(ThreadSpawnParams {
                        provider: spawn.provider,
                        cwd,
                        environment: spawn.environment,
                        base: spawn.base,
                        prompt: spawn.prompt,
                        attachments: Vec::new(),
                        model: spawn.model,
                        effort: spawn.effort,
                        parent: spawn.parent,
                        project: None,
                        permission_mode: spawn.permission_mode,
                        skills_enabled: spawn.skills_enabled,
                        skills_disabled: spawn.skills_disabled,
                        started_by: None,
                        hidden: spawn.hidden,
                    })
                    .map_err(|error| error.message)?;
                summary(self, &result.thread.id)
            }
            ThreadsCall::Send { thread, text } => self
                .thread_send(ThreadSendParams {
                    thread_id: thread,
                    text,
                    delivery: SendMode::Queue,
                    context: Vec::new(),
                    attachments: Vec::new(),
                })
                .map(|_| ThreadsAnswer::Done)
                .map_err(|error| error.message),
            ThreadsCall::Get(thread) => summary(self, &thread),
            ThreadsCall::Transcript(thread) => {
                if self.supervisor.thread(&thread).is_none() {
                    return Err(format!("thread {thread} does not exist"));
                }
                self.store
                    .transcript(&thread, false)
                    .map(ThreadsAnswer::Transcript)
                    .map_err(|error| error.to_string())
            }
            ThreadsCall::Watch(thread) => {
                if self.supervisor.thread(&thread).is_none() {
                    return Err(format!("thread {thread} does not exist"));
                }
                self.watches.push(Watch { plugin: plugin.to_owned(), thread });
                Ok(ThreadsAnswer::Done)
            }
        }
    }

    pub(super) fn command_list(&self) -> CommandListResult {
        let mut commands = Vec::new();
        for plugin in self.host.plugins() {
            for contribution in plugin.contributions() {
                if let Contribution::Command(command) = contribution {
                    commands.push(CommandContribution {
                        plugin: plugin.id().to_owned(),
                        command: command.clone(),
                    });
                }
            }
        }
        CommandListResult { commands }
    }

    pub(super) fn command_run(&mut self, p: CommandRunParams, responder: Responder) {
        let CommandRunParams { plugin: plugin_id, command, thread_id, args } = p;
        let invalid = |message: String| RpcError::new(codes::INVALID_PARAMS, message);
        let Some(plugin) = self.host.plugin(&plugin_id).cloned() else {
            return responder.reply::<Empty>(Err(RpcError::new(
                codes::PLUGIN_ERROR,
                format!("no plugin {plugin_id:?} is loaded"),
            )));
        };
        let info = plugin.contributions().iter().find_map(|contribution| match contribution {
            Contribution::Command(info) if info.id == command => Some(info.clone()),
            _ => None,
        });
        let Some(info) = info else {
            return responder.reply::<Empty>(Err(RpcError::new(
                codes::PLUGIN_ERROR,
                format!("plugin {plugin_id:?} has no command {command:?}"),
            )));
        };
        let thread = match (info.thread, thread_id) {
            (true, None) => {
                return responder
                    .reply::<Empty>(Err(invalid(format!("{command} acts on a thread; name one"))));
            }
            (true, Some(id)) if self.supervisor.thread(&id).is_none() => {
                return responder.reply::<Empty>(Err(thread_not_found(&id)));
            }
            (true, thread) => thread,
            (false, _) => None,
        };
        if let Some(unknown) =
            args.keys().find(|name| !info.params.iter().any(|p| &p.name == *name))
        {
            return responder
                .reply::<Empty>(Err(invalid(format!("{command} takes no parameter {unknown:?}"))));
        }
        if let Some(missing) = info
            .params
            .iter()
            .find(|p| p.required && args.get(&p.name).is_none_or(|value| value.trim().is_empty()))
        {
            return responder.reply::<Empty>(Err(invalid(format!(
                "{command} needs {:?} ({})",
                missing.name, missing.label
            ))));
        }
        let request = PluginRequest::Command { command, thread, args: args.into_iter().collect() };
        tokio::spawn(async move {
            let answer = tokio::time::timeout(QUERY_TIMEOUT, plugin.call(request)).await;
            let error = |message: String| RpcError::new(codes::PLUGIN_ERROR, message);
            let result = match answer {
                Ok(Ok(Outcome::CommandResult { message, thread })) => {
                    Ok(CommandRunResult { message, thread_id: thread })
                }
                Ok(Ok(other)) => Err(error(format!("the plugin answered with {other:?}"))),
                Ok(Err(failure)) => Err(error(failure.to_string())),
                Err(_) => Err(error("the plugin did not answer in time".into())),
            };
            responder.reply(result);
        });
    }

    pub(super) fn ui_contributions(&self) -> UiContributionsResult {
        let mut result = UiContributionsResult { views: Vec::new(), items: Vec::new() };
        for plugin in self.host.plugins() {
            let providers: Vec<ProviderId> = plugin
                .contributions()
                .iter()
                .filter_map(|contribution| match contribution {
                    Contribution::Provider(provider) => Some(provider.id.clone()),
                    _ => None,
                })
                .collect();
            for contribution in plugin.contributions() {
                match contribution {
                    Contribution::UiView(view) => result.views.push(UiViewContribution {
                        plugin: plugin.id().to_owned(),
                        view: view.clone(),
                        providers: providers.clone(),
                    }),
                    // A plugin draws only its own kinds: it cannot take over the
                    // items of another plugin.
                    Contribution::UiItem(kind) if owns_kind(plugin.id(), kind) => {
                        result.items.push(UiItemContribution {
                            plugin: plugin.id().to_owned(),
                            kind: kind.clone(),
                        });
                    }
                    _ => {}
                }
            }
        }
        result
    }

    /// Pass a drawing request to the plugin that contributes the view or item kind.
    pub(super) fn ui_call(
        &mut self,
        plugin_id: &str,
        view: Option<&str>,
        kind: Option<&str>,
        request: UiRequest,
        responder: Responder,
    ) {
        let plugin_error = |message: String| RpcError::new(codes::PLUGIN_ERROR, message);
        let Some(plugin) = self.host.plugin(plugin_id) else {
            return responder
                .reply::<Empty>(Err(plugin_error(format!("no plugin {plugin_id:?} is loaded"))));
        };
        let contributed = plugin.contributions().iter().any(|contribution| match contribution {
            Contribution::UiView(info) => view == Some(info.id.as_str()),
            Contribution::UiItem(own) => kind == Some(own.as_str()) && owns_kind(plugin_id, own),
            _ => false,
        });
        if !contributed {
            let what = view.or(kind).unwrap_or_default();
            return responder.reply::<Empty>(Err(plugin_error(format!(
                "plugin {plugin_id:?} does not contribute {what:?}"
            ))));
        }
        let plugin = plugin.clone();
        tokio::spawn(async move {
            let answer =
                tokio::time::timeout(QUERY_TIMEOUT, plugin.call(PluginRequest::Ui(request))).await;
            let result = match answer {
                Ok(Ok(Outcome::UiTree(tree))) => {
                    serde_json::from_str::<Value>(&tree).map(|tree| UiTreeResult { tree }).map_err(
                        |error| plugin_error(format!("the plugin drew invalid JSON: {error}")),
                    )
                }
                Ok(Ok(other)) => Err(plugin_error(format!("the plugin answered with {other:?}"))),
                Ok(Err(error)) => Err(plugin_error(error.to_string())),
                Err(_) => Err(plugin_error("the plugin did not answer in time".into())),
            };
            responder.reply(result);
        });
    }
}

/// Cache what a provider listed, or turn its failure into an RPC error.
fn listed<T: Clone>(
    cache: &mut std::collections::HashMap<ProviderId, Vec<T>>,
    provider: ProviderId,
    result: Result<Vec<T>, String>,
) -> Result<Vec<T>, RpcError> {
    let items = result.map_err(|message| RpcError::new(codes::PLUGIN_ERROR, message))?;
    cache.insert(provider, items.clone());
    Ok(items)
}
