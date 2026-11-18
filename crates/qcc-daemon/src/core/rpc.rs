//! Dispatching JSON-RPC requests to the handlers.

use base64::Engine as _;
use qcc_domain::{Thread, ThreadState};
use qcc_plugin_host::UiRequest;
use qcc_protocol::jsonrpc::{RpcError, codes};
use qcc_protocol::methods::{
    CommandList, CommandRun, CommentAdd, CommentDelete, CommentList, CommentResolve, DaemonStatus,
    DaemonStatusResult, Empty, InteractionResolve, PluginDisable, PluginEnable, PluginList,
    ProjectCreate, ProjectDelete, ProjectList, ProjectListResult, ProjectUpdate, ProviderModels,
    ProviderSkills, SettingsGet, SettingsSet, SkillList, ThreadArchive, ThreadAttachment,
    ThreadChanges, ThreadCompact, ThreadDeliver, ThreadDiff, ThreadEvents, ThreadGet,
    ThreadGetParams, ThreadIsolate, ThreadList, ThreadListParams, ThreadListResult, ThreadRename,
    ThreadReset, ThreadSend, ThreadSetAgent, ThreadSetMcp, ThreadSetMode, ThreadSetParent,
    ThreadSetProject, ThreadSetSkill, ThreadSpawn, ThreadStop, ThreadUnqueue, ThreadUnsubscribe,
    ThreadUnsubscribeParams, ThreadWait, ThreadWatch, ThreadWatchParams, UiAction, UiActionParams,
    UiAsset, UiAssetParams, UiAssetResult, UiContributions, UiRender, UiRenderItem,
    UiRenderItemParams, UiRenderParams, UsageHistory, UsageHistoryResult, UsageTools,
    UsageToolsResult,
};
use qcc_protocol::{Method, PROTOCOL_VERSION};
use serde_json::Value;

use super::{Core, Responder, Subscriber, params};
use crate::plugins;

impl Core {
    pub(super) fn rpc(&mut self, connection: u64, method: &str, raw: Value, responder: Responder) {
        match method {
            DaemonStatus::NAME => responder.reply(Ok(DaemonStatusResult {
                version: env!("CARGO_PKG_VERSION").into(),
                protocol: PROTOCOL_VERSION,
                pid: std::process::id(),
                data_dir: self.config.data_dir.display().to_string(),
                socket: self.config.socket.display().to_string(),
            })),
            ThreadSpawn::NAME => {
                let result = params::<ThreadSpawn>(raw).and_then(|p| self.thread_spawn(p));
                responder.reply(result);
            }
            ThreadAttachment::NAME => match params::<ThreadAttachment>(raw) {
                Ok(p) => self.thread_attachment(p, responder),
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            ThreadSend::NAME => {
                let result = params::<ThreadSend>(raw).and_then(|p| self.thread_send(p));
                responder.reply(result);
            }
            ThreadSetMcp::NAME => {
                let result = params::<ThreadSetMcp>(raw).and_then(|p| self.thread_set_mcp(p));
                responder.reply(result);
            }
            ThreadSetMode::NAME => {
                let result = params::<ThreadSetMode>(raw).and_then(|p| self.thread_set_mode(p));
                responder.reply(result);
            }
            SettingsGet::NAME => responder.reply(Ok(self.settings_result())),
            SettingsSet::NAME => {
                let result = params::<SettingsSet>(raw).and_then(|p| self.settings_set(p));
                responder.reply(result);
            }
            ThreadDeliver::NAME => match params::<ThreadDeliver>(raw) {
                Ok(p) => self.thread_deliver(p, responder),
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            ThreadIsolate::NAME => {
                let result = params::<ThreadIsolate>(raw).and_then(|p| self.thread_isolate(p));
                responder.reply(result);
            }
            ThreadRename::NAME => {
                let result = params::<ThreadRename>(raw).and_then(|p| self.thread_rename(p));
                responder.reply(result);
            }
            ThreadSetParent::NAME => {
                let result = params::<ThreadSetParent>(raw).and_then(|p| self.thread_set_parent(p));
                responder.reply(result);
            }
            ThreadSetProject::NAME => {
                let result =
                    params::<ThreadSetProject>(raw).and_then(|p| self.thread_set_project(p));
                responder.reply(result);
            }
            CommentList::NAME => {
                let result = params::<CommentList>(raw).and_then(|p| self.comment_list(p));
                responder.reply(result);
            }
            CommentAdd::NAME => {
                let result = params::<CommentAdd>(raw).and_then(|p| self.comment_add(p));
                responder.reply(result);
            }
            CommentResolve::NAME => {
                let result = params::<CommentResolve>(raw).and_then(|p| self.comment_resolve(p));
                responder.reply(result);
            }
            CommentDelete::NAME => {
                let result = params::<CommentDelete>(raw).and_then(|p| self.comment_delete(p));
                responder.reply(result);
            }
            UsageHistory::NAME => {
                let result = self
                    .store
                    .usage_history()
                    .map(|buckets| UsageHistoryResult {
                        bucket_ms: qcc_store::USAGE_BUCKET_MS,
                        buckets,
                    })
                    .map_err(|error| RpcError::new(codes::INTERNAL_ERROR, error.to_string()));
                responder.reply(result);
            }
            UsageTools::NAME => {
                let result = self
                    .store
                    .tool_history()
                    .map(|buckets| UsageToolsResult {
                        bucket_ms: qcc_store::USAGE_BUCKET_MS,
                        buckets,
                    })
                    .map_err(|error| RpcError::new(codes::INTERNAL_ERROR, error.to_string()));
                responder.reply(result);
            }
            ProjectList::NAME => {
                responder.reply(Ok(ProjectListResult { projects: self.projects.clone() }));
            }
            ProjectCreate::NAME => {
                let result = params::<ProjectCreate>(raw).and_then(|p| self.project_create(p));
                responder.reply(result);
            }
            ProjectUpdate::NAME => {
                let result = params::<ProjectUpdate>(raw).and_then(|p| self.project_update(p));
                responder.reply(result);
            }
            ProjectDelete::NAME => {
                let result = params::<ProjectDelete>(raw).and_then(|p| self.project_delete(p));
                responder.reply(result);
            }
            ThreadUnqueue::NAME => {
                let result = params::<ThreadUnqueue>(raw).and_then(|p| self.thread_unqueue(p));
                responder.reply(result);
            }
            ThreadSetSkill::NAME => {
                let result = params::<ThreadSetSkill>(raw).and_then(|p| self.thread_set_skill(p));
                responder.reply(result);
            }
            ThreadReset::NAME => match params::<ThreadReset>(raw) {
                Ok(p) => self.thread_reset(p, responder),
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            ThreadCompact::NAME => {
                let result = params::<ThreadCompact>(raw).and_then(|p| self.thread_compact(p));
                responder.reply(result);
            }
            SkillList::NAME => {
                let result = params::<SkillList>(raw).map(|Empty {}| self.skill_list());
                responder.reply(result);
            }
            ThreadSetAgent::NAME => {
                let result = params::<ThreadSetAgent>(raw).and_then(|p| self.thread_set_agent(p));
                responder.reply(result);
            }
            ThreadStop::NAME => match params::<ThreadStop>(raw) {
                Ok(p) => self.thread_stop(p, responder),
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            ThreadList::NAME => {
                let result = params::<ThreadList>(raw).map(
                    |ThreadListParams { include_archived, state }| {
                        let archived = include_archived || state == Some(ThreadState::Archived);
                        let mut threads: Vec<_> = self
                            .supervisor
                            .threads()
                            .filter(|thread| archived || !thread.archived)
                            .cloned()
                            .collect();
                        if let Some(state) = state {
                            threads.retain(|thread| self.thread_state(&thread.id) == Some(state));
                        }
                        ThreadListResult { threads: oldest_first(threads) }
                    },
                );
                responder.reply(result);
            }
            ThreadWatch::NAME => match params::<ThreadWatch>(raw) {
                Ok(ThreadWatchParams { subscription }) => {
                    let out = responder.out.clone();
                    let threads = oldest_first(self.supervisor.threads().cloned().collect());
                    responder.reply(Ok(ThreadListResult { threads }));
                    // Registered in the same step as the listing, so no change is missed.
                    self.watchers.push(Subscriber { connection, subscription, out });
                }
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            ThreadArchive::NAME => match params::<ThreadArchive>(raw) {
                Ok(p) => self.thread_archive(p, responder),
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            ThreadChanges::NAME => match params::<ThreadChanges>(raw) {
                Ok(p) => self.environment_query(&p.thread_id, None, responder),
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            ThreadDiff::NAME => match params::<ThreadDiff>(raw) {
                Ok(p) => self.environment_query(&p.thread_id, Some(p.paths), responder),
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            ThreadGet::NAME => {
                let result = params::<ThreadGet>(raw)
                    .and_then(|ThreadGetParams { thread_id }| self.thread_result(&thread_id));
                responder.reply(result);
            }
            ThreadWait::NAME => match params::<ThreadWait>(raw) {
                Ok(p) => self.thread_wait(connection, p, responder),
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            ThreadEvents::NAME => match params::<ThreadEvents>(raw) {
                Ok(p) => self.thread_events(connection, p, responder),
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            ThreadUnsubscribe::NAME => {
                let result = params::<ThreadUnsubscribe>(raw).map(
                    |ThreadUnsubscribeParams { subscription }| {
                        for subscribers in self.subscribers.values_mut() {
                            subscribers.retain(|s| {
                                s.connection != connection || s.subscription != subscription
                            });
                        }
                        self.watchers.retain(|watcher| {
                            watcher.connection != connection || watcher.subscription != subscription
                        });
                        Empty {}
                    },
                );
                responder.reply(result);
            }
            InteractionResolve::NAME => {
                let result =
                    params::<InteractionResolve>(raw).and_then(|p| self.interaction_resolve(p));
                responder.reply(result);
            }
            PluginList::NAME => responder.reply(Ok(self.plugin_list())),
            PluginEnable::NAME => {
                let result = params::<PluginEnable>(raw).and_then(|p| self.plugin_enable(p));
                responder.reply(result);
            }
            PluginDisable::NAME => {
                let result = params::<PluginDisable>(raw).and_then(|p| self.plugin_disable(p));
                responder.reply(result);
            }
            ProviderModels::NAME => match params::<ProviderModels>(raw) {
                Ok(p) => self.provider_models(p, responder),
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            ProviderSkills::NAME => match params::<ProviderSkills>(raw) {
                Ok(p) => self.provider_skills(p, responder),
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            CommandList::NAME => responder.reply(Ok(self.command_list())),
            CommandRun::NAME => match params::<CommandRun>(raw) {
                Ok(p) => self.command_run(p, responder),
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            UiContributions::NAME => responder.reply(Ok(self.ui_contributions())),
            UiRender::NAME => match params::<UiRender>(raw) {
                Ok(UiRenderParams { plugin, view, thread_id }) => {
                    let request = UiRequest::Render { view: view.clone(), thread: thread_id };
                    self.ui_call(&plugin, Some(&view), None, request, responder);
                }
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            UiAction::NAME => match params::<UiAction>(raw) {
                Ok(UiActionParams { plugin, view, thread_id, action, value }) => {
                    let request =
                        UiRequest::Action { view: view.clone(), thread: thread_id, action, value };
                    self.ui_call(&plugin, Some(&view), None, request, responder);
                }
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            UiAsset::NAME => {
                let result = params::<UiAsset>(raw).and_then(|UiAssetParams { plugin, path }| {
                    let error = |message: String| RpcError::new(codes::PLUGIN_ERROR, message);
                    let handle = self
                        .host
                        .plugin(&plugin)
                        .ok_or_else(|| error(format!("no plugin {plugin:?} is loaded")))?;
                    let (data, content_type) =
                        plugins::read_asset(&self.config, handle.manifest(), &path)
                            .map_err(error)?;
                    Ok(UiAssetResult {
                        data: base64::engine::general_purpose::STANDARD.encode(data),
                        content_type: content_type.to_owned(),
                    })
                });
                responder.reply(result);
            }
            UiRenderItem::NAME => match params::<UiRenderItem>(raw) {
                Ok(UiRenderItemParams { plugin, kind, payload }) => {
                    let request =
                        UiRequest::RenderItem { kind: kind.clone(), payload: payload.to_string() };
                    self.ui_call(&plugin, None, Some(&kind), request, responder);
                }
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            unknown => responder.reply::<Empty>(Err(RpcError::new(
                codes::METHOD_NOT_FOUND,
                format!("unknown method {unknown:?}"),
            ))),
        }
    }
}

/// Threads in the order they were created, ties broken by id so the order is stable.
fn oldest_first(mut threads: Vec<Thread>) -> Vec<Thread> {
    threads.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
    threads
}
