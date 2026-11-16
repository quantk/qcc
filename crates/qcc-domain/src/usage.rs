use serde::{Deserialize, Serialize};

use crate::thread::TurnStatus;

/// Token counts with the provider's own semantics; the core never re-derives them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
}

/// The one usage dialect: cumulative session total plus the last request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UsageSnapshot {
    pub total: TokenUsage,
    pub last: TokenUsage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_context_window: Option<u64>,
    /// Tokens the context holds now. Only the bridge knows how its agent's counts
    /// add up to that; absent when it cannot tell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_used: Option<u64>,
}

impl TokenUsage {
    /// Whether `next` cannot be a later count of the same session as `self`: some
    /// count went down.
    fn restarted_by(&self, next: &TokenUsage) -> bool {
        next.input_tokens < self.input_tokens
            || next.output_tokens < self.output_tokens
            || next.cached_input_tokens.unwrap_or(0) < self.cached_input_tokens.unwrap_or(0)
    }

    /// What `next` counts beyond `self`, component by component.
    fn since(&self, next: &TokenUsage) -> TokenUsage {
        TokenUsage {
            input_tokens: next.input_tokens.saturating_sub(self.input_tokens),
            output_tokens: next.output_tokens.saturating_sub(self.output_tokens),
            cached_input_tokens: next
                .cached_input_tokens
                .map(|cached| cached.saturating_sub(self.cached_input_tokens.unwrap_or(0))),
        }
    }

    pub fn add(&mut self, other: &TokenUsage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cached_input_tokens = match (self.cached_input_tokens, other.cached_input_tokens) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
        };
    }

    pub fn is_zero(&self) -> bool {
        self.input_tokens == 0
            && self.output_tokens == 0
            && self.cached_input_tokens.unwrap_or(0) == 0
    }
}

/// What a thread has spent and how it is doing, over all its sessions: the numbers
/// worth seeing without reading its timeline. Part of the thread's projection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadUsage {
    /// Tokens every session of the thread spent, in the providers' own counts.
    pub spent: TokenUsage,
    /// The current session's total as last reported, which the next report counts
    /// on from.
    pub session: TokenUsage,
    /// Tokens the context holds, as the bridge last said. Absent before it said and
    /// after the conversation started afresh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_used: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    /// Turns that ended, by how.
    pub turns_completed: u32,
    pub turns_failed: u32,
    pub turns_interrupted: u32,
    /// Milliseconds spent in turns that ended.
    pub working_ms: u64,
}

impl ThreadUsage {
    /// Fold one usage report in, and say how many tokens it adds to what was spent.
    ///
    /// A provider counts its total per session. A session that ended without a word
    /// and a new one that counts from zero look the same as a count going down;
    /// providers that carry their count over a resume never go down, so nothing is
    /// counted twice.
    pub fn report(&mut self, usage: &UsageSnapshot) -> TokenUsage {
        let added = if self.session.restarted_by(&usage.total) {
            usage.total
        } else {
            self.session.since(&usage.total)
        };
        self.spent.add(&added);
        self.session = usage.total;
        if usage.model_context_window.is_some() {
            self.context_window = usage.model_context_window;
        }
        self.context_used = usage.context_used;
        added
    }

    /// A turn that started at `started_at` ended at `ended_at` with `status`.
    pub fn turn_ended(&mut self, status: TurnStatus, started_at: Option<u64>, ended_at: u64) {
        match status {
            TurnStatus::Completed => self.turns_completed += 1,
            TurnStatus::Failed => self.turns_failed += 1,
            TurnStatus::Interrupted => self.turns_interrupted += 1,
        }
        if let Some(started_at) = started_at {
            self.working_ms += ended_at.saturating_sub(started_at);
        }
    }

    /// The conversation starts afresh: the next session counts from zero and its
    /// context is empty. What was spent stays spent.
    pub fn reset(&mut self) {
        self.session = TokenUsage::default();
        self.context_used = None;
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// What one thread spent and how its turns went in one span of time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UsageBucket {
    pub thread_id: crate::ids::ThreadId,
    /// Start of the span, in milliseconds since the Unix epoch.
    pub hour: u64,
    pub spent: TokenUsage,
    pub turns_completed: u32,
    pub turns_failed: u32,
    pub turns_interrupted: u32,
    /// Time in turns that ended within the span.
    pub working_ms: u64,
}

impl UsageBucket {
    /// What changed from `before` to `after`, put in the span starting at `hour`.
    pub fn between(thread_id: &str, hour: u64, before: &ThreadUsage, after: &ThreadUsage) -> Self {
        Self {
            thread_id: crate::ids::ThreadId::new(thread_id),
            hour,
            spent: before.spent.since(&after.spent),
            turns_completed: after.turns_completed - before.turns_completed,
            turns_failed: after.turns_failed - before.turns_failed,
            turns_interrupted: after.turns_interrupted - before.turns_interrupted,
            working_ms: after.working_ms - before.working_ms,
        }
    }

    pub fn add(&mut self, other: &UsageBucket) {
        self.spent.add(&other.spent);
        self.turns_completed += other.turns_completed;
        self.turns_failed += other.turns_failed;
        self.turns_interrupted += other.turns_interrupted;
        self.working_ms += other.working_ms;
    }

    pub fn is_empty(&self) -> bool {
        self.spent.is_zero()
            && self.turns_completed == 0
            && self.turns_failed == 0
            && self.turns_interrupted == 0
            && self.working_ms == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(input: u64, output: u64) -> TokenUsage {
        TokenUsage { input_tokens: input, output_tokens: output, cached_input_tokens: None }
    }

    fn snapshot(total: TokenUsage, context_used: Option<u64>) -> UsageSnapshot {
        UsageSnapshot { total, last: total, model_context_window: Some(1000), context_used }
    }

    #[test]
    fn spending_carries_over_sessions_that_count_from_zero() {
        let mut usage = ThreadUsage::default();
        assert_eq!(usage.report(&snapshot(tokens(10, 2), Some(10))), tokens(10, 2));
        assert_eq!(usage.report(&snapshot(tokens(25, 5), Some(20))), tokens(15, 3));
        // A new session counts from zero.
        assert_eq!(usage.report(&snapshot(tokens(4, 1), None)), tokens(4, 1));
        assert_eq!(usage.spent, tokens(29, 6));
        assert_eq!((usage.context_used, usage.context_window), (None, Some(1000)));
    }

    #[test]
    fn a_fresh_conversation_counts_anew_and_keeps_what_was_spent() {
        let mut usage = ThreadUsage::default();
        usage.report(&snapshot(tokens(10, 2), Some(10)));
        usage.reset();
        // Larger than before, but a new count all the same.
        usage.report(&snapshot(tokens(30, 1), Some(30)));
        assert_eq!(usage.spent, tokens(40, 3));
    }
}
