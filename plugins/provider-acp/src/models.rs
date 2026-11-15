//! Which models an ACP agent offers. The protocol tells only once a session exists,
//! so an agent that can say it on the command line is asked there: `opencode models`
//! prints one `provider/model` per line.

use qcc_plugin_sdk::stdio::{ProbeParser, exit_reason};
use qcc_plugin_sdk::types::{ModelInfo, Outcome};

pub struct ModelsProbe {
    agent: &'static str,
    models: Vec<ModelInfo>,
}

impl ModelsProbe {
    pub fn new(agent: &'static str) -> Self {
        Self { agent, models: Vec::new() }
    }
}

impl ProbeParser for ModelsProbe {
    fn on_line(&mut self, line: &str) -> Option<Result<Outcome, String>> {
        let id = line.trim();
        // Anything with spaces is a message, not a model id.
        if !id.is_empty() && !id.contains(char::is_whitespace) {
            self.models.push(ModelInfo {
                id: id.to_owned(),
                name: id.to_owned(),
                description: None,
                is_default: false,
            });
        }
        None
    }

    fn exited(&mut self, code: Option<i32>, stderr: Option<&str>) -> Result<Outcome, String> {
        if code == Some(0) {
            Ok(Outcome::Models(std::mem::take(&mut self.models)))
        } else {
            Err(exit_reason(self.agent, code, stderr))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_model_per_line() {
        let mut probe = ModelsProbe::new("opencode");
        for line in ["opencode/big-pickle", "", "Fetching models from somewhere", "openai/gpt-x"] {
            assert!(probe.on_line(line).is_none());
        }
        let Ok(Outcome::Models(models)) = probe.exited(Some(0), None) else { panic!("no models") };
        let ids: Vec<_> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["opencode/big-pickle", "openai/gpt-x"]);
    }

    #[test]
    fn a_failed_listing_says_why() {
        let mut probe = ModelsProbe::new("opencode");
        assert_eq!(
            probe.exited(Some(1), Some("not logged in")),
            Err("opencode exited with code 1: not logged in".into())
        );
    }
}
