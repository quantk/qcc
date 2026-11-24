//! The bundled agents declare their marks through the real wasm host, and the
//! host keeps them.

use qcc_domain::plausible_icon;
use qcc_plugin_host::Contribution;
use qcc_plugin_testkit::Harness;

/// The mark the provider `id` of the bundled plugin `plugin` declares.
fn mark(plugin: &str, id: &str) -> Option<String> {
    let harness = Harness::bundled(plugin).build();
    harness
        .handle()
        .contributions()
        .iter()
        .find_map(|contribution| match contribution {
            Contribution::Provider(info) if info.id.as_str() == id => Some(info.icon.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{plugin} does not provide {id}"))
}

#[test]
fn real_agents_come_with_their_marks() {
    for (plugin, id) in [
        ("provider-claude-code", "claude-code"),
        ("provider-codex", "codex"),
        ("provider-acp", "opencode"),
    ] {
        let icon = mark(plugin, id).unwrap_or_else(|| panic!("{id} has no mark"));
        assert!(plausible_icon(&icon), "{id}: {icon}");
    }
}

#[test]
fn the_test_agent_has_none() {
    assert_eq!(mark("provider-echo", "echo"), None);
}
