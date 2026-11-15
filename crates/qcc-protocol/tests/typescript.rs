//! The TypeScript side of the daemon API is generated from the Rust types and
//! committed. This test fails when the committed file is stale;
//! `QCC_UPDATE_TS=1 cargo test -p qcc-protocol --test typescript` rewrites it.

use std::path::Path;

use qcc_protocol::Method;
use qcc_protocol::jsonrpc::RpcError;
use qcc_protocol::methods::*;
use specta::Types;
use specta_typescript::Typescript;
use specta_util::Remapper;

const GENERATED: &str = "../../apps/desktop/src/api/protocol.gen.ts";

struct Api {
    types: Types,
    methods: Vec<(&'static str, &'static str, &'static str)>,
}

fn short_name<T>() -> &'static str {
    std::any::type_name::<T>().rsplit("::").next().expect("a type has a name")
}

impl Api {
    fn method<M: Method>(mut self) -> Self
    where
        M::Params: specta::Type,
        M::Result: specta::Type,
    {
        self.types = self.types.register::<M::Params>().register::<M::Result>();
        self.methods.push((M::NAME, short_name::<M::Params>(), short_name::<M::Result>()));
        self
    }
}

fn generate() -> String {
    let api = Api { types: Types::default(), methods: Vec::new() }
        .method::<DaemonStatus>()
        .method::<ThreadSpawn>()
        .method::<ThreadSend>()
        .method::<ThreadSetMode>()
        .method::<ThreadStop>()
        .method::<ThreadList>()
        .method::<ThreadWatch>()
        .method::<ThreadGet>()
        .method::<ThreadArchive>()
        .method::<ThreadChanges>()
        .method::<ThreadDiff>()
        .method::<ThreadEvents>()
        .method::<ThreadUnsubscribe>()
        .method::<InteractionResolve>()
        .method::<PluginList>()
        .method::<UiContributions>()
        .method::<UiRender>()
        .method::<UiAction>()
        .method::<UiRenderItem>();
    let types = api
        .types
        .register::<ThreadEventNotification>()
        .register::<ThreadUpdatedNotification>()
        .register::<RpcError>();
    // Sequence numbers, timestamps and token counts stay far below 2^53.
    let types = Remapper::new().dangerous_bigints_as_number().remap_types(types);

    let mut out = Typescript::default()
        .header("// Generated from crates/qcc-protocol by its `typescript` test. Do not edit.")
        .export(&types, specta_serde::Format)
        .expect("the protocol types export to TypeScript");
    out.push_str("\n/** Every request method: its parameters and its result. */\n");
    out.push_str("export type Methods = {\n");
    for (name, params, result) in &api.methods {
        out.push_str(&format!("\t\"{name}\": {{ params: {params}; result: {result} }},\n"));
    }
    out.push_str("};\n\n/** Notifications the daemon pushes, by method name. */\n");
    out.push_str("export type Notifications = {\n");
    out.push_str(&format!("\t\"{THREAD_EVENT}\": ThreadEventNotification,\n"));
    out.push_str(&format!("\t\"{THREAD_UPDATED}\": ThreadUpdatedNotification,\n"));
    out.push_str("};\n");
    out
}

#[test]
fn generated_typescript_is_up_to_date() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(GENERATED);
    let generated = generate();
    if std::env::var_os("QCC_UPDATE_TS").is_some() {
        std::fs::write(&path, &generated).expect("the generated file is writable");
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == generated,
        "{} is stale; run `QCC_UPDATE_TS=1 cargo test -p qcc-protocol --test typescript`",
        path.display()
    );
}
