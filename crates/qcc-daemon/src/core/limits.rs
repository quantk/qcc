//! Usage limits of the providers' accounts: the latest each provider reported,
//! during a session or when asked, kept across restarts.

use qcc_domain::{ProviderId, ProviderLimits, RateLimits};
use qcc_plugin_host::{Outcome, PluginRequest, ProviderRequest};
use qcc_protocol::jsonrpc::{RpcError, codes};
use qcc_protocol::methods::{
    Empty, LIMITS_UPDATED, LimitsUpdatedNotification, ProviderLimitsError, ProviderLimitsParams,
    ProviderLimitsResult,
};

use super::{Command, Core, MODELS_TIMEOUT, Responder, notify, now_ms};

impl Core {
    pub(super) fn provider_limits(&mut self, p: ProviderLimitsParams, responder: Responder) {
        let ProviderLimitsParams { provider, refresh } = p;
        if let Some(id) = &provider {
            match self.provider_of(id) {
                Ok((_, info)) if !info.capabilities.rate_limits => {
                    let message = format!("provider {id} does not report usage limits");
                    return responder
                        .reply::<Empty>(Err(RpcError::new(codes::INVALID_PARAMS, message)));
                }
                Ok(_) => {}
                Err(error) => return responder.reply::<Empty>(Err(error)),
            }
        }
        if !refresh {
            return responder.reply(Ok(self.limits_result(provider.as_ref(), Vec::new())));
        }
        let targets: Vec<_> = self
            .host
            .providers()
            .filter(|(_, info)| info.capabilities.rate_limits)
            .filter(|(_, info)| provider.as_ref().is_none_or(|id| *id == info.id))
            .map(|(plugin, info)| (info.id.clone(), plugin.clone()))
            .collect();
        let commands = self.commands.clone();
        tokio::spawn(async move {
            // Every provider at once: each runs its own agent, and one that hangs
            // must not hold up the others past the common limit.
            let reads = targets.into_iter().map(|(provider, plugin)| {
                tokio::spawn(async move {
                    let request = PluginRequest::Provider {
                        provider: provider.clone(),
                        request: ProviderRequest::ReadRateLimits,
                    };
                    let result =
                        match tokio::time::timeout(MODELS_TIMEOUT, plugin.call(request)).await {
                            Ok(Ok(Outcome::RateLimits(limits))) => Ok(limits),
                            Ok(Ok(other)) => Err(format!("the provider answered with {other:?}")),
                            Ok(Err(error)) => Err(error.to_string()),
                            Err(_) => Err(format!(
                                "the provider did not read its limits within {} s",
                                MODELS_TIMEOUT.as_secs()
                            )),
                        };
                    (provider, result)
                })
            });
            let reads: Vec<_> = reads.collect();
            let mut results = Vec::new();
            for read in reads {
                if let Ok(result) = read.await {
                    results.push(result);
                }
            }
            let _ = commands.send(Command::LimitsRead { only: provider, results, responder });
        });
    }

    pub(super) fn limits_read(
        &mut self,
        only: Option<ProviderId>,
        results: Vec<(ProviderId, Result<RateLimits, String>)>,
        responder: Responder,
    ) {
        let mut errors = Vec::new();
        for (provider, result) in results {
            match result {
                // An answer to a read lists every window the account has.
                Ok(report) => {
                    self.limits_reported(provider, RateLimits { complete: true, ..report })
                }
                Err(message) => errors.push(ProviderLimitsError { provider, message }),
            }
        }
        responder.reply(Ok(self.limits_result(only.as_ref(), errors)));
    }

    /// A provider told how much of its account's limits is spent: keep it, save it
    /// and tell the watchers.
    pub(super) fn limits_reported(&mut self, provider: ProviderId, report: RateLimits) {
        let now = now_ms();
        let limits = match self.limits.get_mut(&provider) {
            Some(limits) => {
                limits.merge(report, now);
                limits.clone()
            }
            None => {
                let limits = ProviderLimits::new(provider.clone(), report, now);
                self.limits.insert(provider, limits.clone());
                limits
            }
        };
        // Losing this only means showing older limits after a restart; it is not
        // worth stopping the daemon over, as a journal that cannot be written is.
        if let Err(error) = self.store.save_provider_limits(&limits) {
            tracing::warn!(provider = %limits.provider, %error, "cannot save usage limits");
        }
        self.watchers.retain(|watcher| {
            let subscription = watcher.subscription;
            let params = LimitsUpdatedNotification { subscription, limits: limits.clone() };
            notify(&watcher.out, LIMITS_UPDATED, params)
        });
    }

    fn limits_result(
        &self,
        only: Option<&ProviderId>,
        errors: Vec<ProviderLimitsError>,
    ) -> ProviderLimitsResult {
        let limits = self
            .limits
            .values()
            // A provider whose plugin is gone has no one to say its limits are current.
            .filter(|limits| self.host.provider(&limits.provider).is_some())
            .filter(|limits| only.is_none_or(|id| *id == limits.provider))
            .cloned()
            .collect();
        ProviderLimitsResult { limits, errors }
    }
}
