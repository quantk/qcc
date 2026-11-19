//! Notifications: tells the user when an agent is done with its turn, failed, or
//! waits for an answer.
//!
//! Nothing here is part of the core. The plugin hears of every thread's changes
//! (the `thread-events` permission) and asks the daemon to show a notification (the
//! `notifications` permission), which the desktop app shows the way its system shows
//! notifications. What deserves one is decided by [`notification_for`], a pure
//! function of the change and the user's settings.

use qcc_plugin_sdk::notifications::{self, Notification};
use qcc_plugin_sdk::prelude::*;

/// How much of the agent's latest message a notification shows.
const PREVIEW_MAX_CHARS: usize = 140;
/// How much of a thread's title is the notification's title.
const TITLE_MAX_CHARS: usize = 80;

/// What happened that the user may want to hear about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A turn ended as it should.
    Finished,
    /// A turn failed, or the agent's session could not start or died.
    Failed,
    /// The agent asks the user to approve something or answer a question.
    Waiting,
}

/// Which changes of threads started by other agents are worth a notification. The
/// agent that started them usually waits for them itself, but it cannot answer
/// what they ask the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Children {
    Never,
    WhenWaiting,
    Always,
}

impl Children {
    const VALUES: [(&str, Self); 3] =
        [("never", Self::Never), ("when-waiting", Self::WhenWaiting), ("always", Self::Always)];

    fn parse(value: &str) -> Option<Self> {
        Self::VALUES.iter().find(|(name, _)| *name == value).map(|(_, children)| *children)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Settings {
    finished: bool,
    failed: bool,
    waiting: bool,
    children: Children,
    quiet_when_shown: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            finished: true,
            failed: true,
            waiting: true,
            children: Children::WhenWaiting,
            quiet_when_shown: true,
        }
    }
}

const ON_FINISHED: &str = "on-finished";
const ON_FAILED: &str = "on-failed";
const ON_WAITING: &str = "on-waiting";
const CHILDREN: &str = "children";
const QUIET_WHEN_SHOWN: &str = "quiet-when-shown";

impl Settings {
    /// The values the daemon sent; one that cannot be read keeps its default.
    fn parse(values: &[(String, String)]) -> Self {
        let mut settings = Self::default();
        for (id, value) in values {
            let flag = match value.as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            };
            match (id.as_str(), flag) {
                (ON_FINISHED, Some(flag)) => settings.finished = flag,
                (ON_FAILED, Some(flag)) => settings.failed = flag,
                (ON_WAITING, Some(flag)) => settings.waiting = flag,
                (QUIET_WHEN_SHOWN, Some(flag)) => settings.quiet_when_shown = flag,
                (CHILDREN, _) => {
                    settings.children = Children::parse(value).unwrap_or(settings.children);
                }
                _ => {}
            }
        }
        settings
    }

    fn fields() -> Vec<SettingField> {
        let flag = |id: &str, label: &str, description: &str| SettingField {
            id: id.into(),
            label: label.into(),
            description: Some(description.into()),
            kind: SettingKind::Flag,
            default: Some("true".into()),
        };
        vec![
            flag(ON_FINISHED, "When a turn ends", "An agent finished its turn and waits for you."),
            flag(
                ON_FAILED,
                "When an agent fails",
                "A turn failed, or the agent's session could not start or died.",
            ),
            flag(
                ON_WAITING,
                "When an agent asks",
                "An agent waits for you to approve something or answer a question.",
            ),
            SettingField {
                id: CHILDREN.into(),
                label: "Threads started by agents".into(),
                description: Some(
                    "For threads another agent started: never, only when they wait for you \
                     (their agent cannot answer for you), or always."
                        .into(),
                ),
                kind: SettingKind::Choice(
                    Children::VALUES.iter().map(|(name, _)| (*name).to_owned()).collect(),
                ),
                default: Some("when-waiting".into()),
            },
            flag(
                QUIET_WHEN_SHOWN,
                "Quiet about the thread on screen",
                "No notification about a thread open in the app while its window has focus.",
            ),
        ]
    }

    fn wants(&self, kind: Kind) -> bool {
        match kind {
            Kind::Finished => self.finished,
            Kind::Failed => self.failed,
            Kind::Waiting => self.waiting,
        }
    }
}

/// What the change is, if it is one of those the user may hear about.
fn kind_of(change: &ThreadChange) -> Option<Kind> {
    match change.thread.state {
        // Each report of a waiting thread is a new request: it started waiting, or
        // it waits on another one.
        ThreadState::Waiting => change.request.is_some().then_some(Kind::Waiting),
        ThreadState::Failed => {
            (change.previous != Some(ThreadState::Failed)).then_some(Kind::Failed)
        }
        ThreadState::Idle if change.turn_ended => match change.thread.last_turn {
            Some(TurnStatus::Completed) | None => Some(Kind::Finished),
            Some(TurnStatus::Failed) => Some(Kind::Failed),
            // The user stopped it, or the daemon did on its way down.
            Some(TurnStatus::Interrupted) => None,
        },
        ThreadState::Idle | ThreadState::Working | ThreadState::Archived => None,
    }
}

/// At most `max` characters of `text` on one line, marked when cut.
fn one_line(text: &str, max: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= max {
        return line;
    }
    let kept: String = line.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", kept.trim_end())
}

fn title(thread: &ThreadInfo) -> String {
    match thread.title.as_deref().map(str::trim).filter(|title| !title.is_empty()) {
        Some(title) => one_line(title, TITLE_MAX_CHARS),
        None => format!("{} thread", thread.provider),
    }
}

fn body(kind: Kind, change: &ThreadChange) -> String {
    let (status, detail) = match kind {
        Kind::Finished => ("Finished", change.last_message.as_deref()),
        Kind::Failed => ("Failed", change.last_message.as_deref()),
        Kind::Waiting => ("Waiting for you", change.request.as_deref()),
    };
    let detail = detail.map(|text| one_line(text, PREVIEW_MAX_CHARS)).filter(|t| !t.is_empty());
    match detail {
        Some(detail) => format!("{status}: {detail}"),
        None => format!("{status}."),
    }
}

/// The notification a change deserves under `settings`, if any.
fn notification_for(change: &ThreadChange, settings: &Settings) -> Option<Notification> {
    let thread = &change.thread;
    // A plugin's thread for its own use is none of the user's business.
    if thread.hidden {
        return None;
    }
    let kind = kind_of(change)?;
    if !settings.wants(kind) {
        return None;
    }
    if thread.parent.is_some() {
        let wanted = match settings.children {
            Children::Never => false,
            Children::WhenWaiting => kind == Kind::Waiting,
            Children::Always => true,
        };
        if !wanted {
            return None;
        }
    }
    Some(Notification {
        title: title(thread),
        body: body(kind, change),
        thread: Some(thread.id.clone()),
        quiet_when_shown: settings.quiet_when_shown,
    })
}

#[derive(Default)]
struct Notify {
    settings: Settings,
}

impl Plugin for Notify {
    fn settings(&self) -> Vec<SettingField> {
        Settings::fields()
    }

    fn on_settings(&mut self, values: Vec<(String, String)>) {
        self.settings = Settings::parse(&values);
    }

    fn on_thread_changed(&mut self, change: ThreadChange) {
        if let Some(notification) = notification_for(&change, &self.settings)
            && let Err(error) = notifications::show(&notification)
        {
            log::warn(format!("cannot show a notification: {error}"));
        }
    }
}

qcc_plugin_sdk::export_plugin!(Notify);

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(state: ThreadState, last_turn: Option<TurnStatus>) -> ThreadInfo {
        ThreadInfo {
            id: "thr_1".into(),
            provider: "claude-code".into(),
            cwd: "/work".into(),
            parent: None,
            title: Some("Fix the login form".into()),
            hidden: false,
            model: None,
            effort: None,
            environment: None,
            state,
            last_turn,
        }
    }

    fn change(state: ThreadState, last_turn: Option<TurnStatus>) -> ThreadChange {
        ThreadChange {
            thread: thread(state, last_turn),
            previous: Some(ThreadState::Working),
            turn_ended: state == ThreadState::Idle,
            last_message: Some("All tests pass.\n\nThe form now checks the email.".into()),
            request: None,
        }
    }

    fn waiting() -> ThreadChange {
        ThreadChange {
            request: Some("Run `cargo test`?".into()),
            last_message: None,
            turn_ended: false,
            ..change(ThreadState::Waiting, None)
        }
    }

    fn child(mut change: ThreadChange) -> ThreadChange {
        change.thread.parent = Some("thr_lead".into());
        change
    }

    fn shown(change: &ThreadChange, settings: &Settings) -> Option<(String, String)> {
        notification_for(change, settings).map(|n| (n.title, n.body))
    }

    #[test]
    fn a_finished_turn_says_what_the_agent_said_last() {
        let n = notification_for(
            &change(ThreadState::Idle, Some(TurnStatus::Completed)),
            &Settings::default(),
        )
        .unwrap();
        assert_eq!(n.title, "Fix the login form");
        assert_eq!(n.body, "Finished: All tests pass. The form now checks the email.");
        assert_eq!(n.thread.as_deref(), Some("thr_1"));
        assert!(n.quiet_when_shown);
    }

    #[test]
    fn failures_and_requests_are_told_apart() {
        let settings = Settings::default();
        let failed_turn = change(ThreadState::Idle, Some(TurnStatus::Failed));
        assert!(shown(&failed_turn, &settings).unwrap().1.starts_with("Failed: "));
        let died = ThreadChange {
            last_message: None,
            turn_ended: false,
            ..change(ThreadState::Failed, None)
        };
        assert_eq!(shown(&died, &settings).unwrap().1, "Failed.");
        assert_eq!(shown(&waiting(), &settings).unwrap().1, "Waiting for you: Run `cargo test`?");
    }

    #[test]
    fn what_is_not_the_end_of_a_turn_is_not_told() {
        let settings = Settings::default();
        // Interrupted by the user, who knows.
        assert_eq!(
            shown(&change(ThreadState::Idle, Some(TurnStatus::Interrupted)), &settings),
            None
        );
        // Idle again without a turn: a session was released, say.
        let released = ThreadChange {
            turn_ended: false,
            ..change(ThreadState::Idle, Some(TurnStatus::Completed))
        };
        assert_eq!(shown(&released, &settings), None);
        assert_eq!(shown(&change(ThreadState::Working, None), &settings), None);
        assert_eq!(shown(&change(ThreadState::Archived, None), &settings), None);
        // Still failed: told once.
        let still = ThreadChange {
            previous: Some(ThreadState::Failed),
            ..change(ThreadState::Failed, None)
        };
        assert_eq!(shown(&still, &settings), None);
        let mut hidden = change(ThreadState::Idle, Some(TurnStatus::Completed));
        hidden.thread.hidden = true;
        assert_eq!(shown(&hidden, &settings), None);
    }

    #[test]
    fn each_kind_can_be_turned_off() {
        let finished = change(ThreadState::Idle, Some(TurnStatus::Completed));
        let failed = change(ThreadState::Idle, Some(TurnStatus::Failed));
        let off = |id: &str| Settings::parse(&[(id.to_owned(), "false".to_owned())]);
        assert_eq!(shown(&finished, &off(ON_FINISHED)), None);
        assert!(shown(&failed, &off(ON_FINISHED)).is_some());
        assert_eq!(shown(&failed, &off(ON_FAILED)), None);
        assert_eq!(shown(&waiting(), &off(ON_WAITING)), None);
        assert!(!notification_for(&finished, &off(QUIET_WHEN_SHOWN)).unwrap().quiet_when_shown);
    }

    #[test]
    fn threads_started_by_agents_are_told_as_the_user_chose() {
        let finished = child(change(ThreadState::Idle, Some(TurnStatus::Completed)));
        let asks = child(waiting());
        let children = |value: &str| Settings::parse(&[(CHILDREN.to_owned(), value.to_owned())]);

        // By default only what the lead agent cannot do for the user.
        assert_eq!(shown(&finished, &Settings::default()), None);
        assert!(shown(&asks, &Settings::default()).is_some());
        assert_eq!(shown(&asks, &children("never")), None);
        assert!(shown(&finished, &children("always")).is_some());
        // Waiting stays off when the user turned it off for every thread.
        let mut quiet = children("always");
        quiet.waiting = false;
        assert_eq!(shown(&asks, &quiet), None);
    }

    #[test]
    fn settings_have_defaults_and_bad_values_keep_them() {
        assert_eq!(Settings::parse(&[]), Settings::default());
        let odd = Settings::parse(&[
            (ON_FINISHED.into(), "maybe".into()),
            (CHILDREN.into(), "sometimes".into()),
            ("unknown".into(), "false".into()),
        ]);
        assert_eq!(odd, Settings::default());
        // Every field declares the default the plugin assumes.
        let declared: Vec<(String, String)> = Settings::fields()
            .into_iter()
            .map(|field| (field.id, field.default.unwrap()))
            .collect();
        assert_eq!(Settings::parse(&declared), Settings::default());
    }

    #[test]
    fn titles_and_previews_fit_on_a_line() {
        let mut long = change(ThreadState::Idle, Some(TurnStatus::Completed));
        long.thread.title = None;
        long.last_message = Some("word ".repeat(100));
        let (title, body) = shown(&long, &Settings::default()).unwrap();
        assert_eq!(title, "claude-code thread");
        assert!(body.ends_with('…'), "{body}");
        assert!(body.chars().count() <= "Finished: ".len() + PREVIEW_MAX_CHARS, "{body}");
        long.last_message = Some("  ".into());
        assert_eq!(shown(&long, &Settings::default()).unwrap().1, "Finished.");
    }
}
