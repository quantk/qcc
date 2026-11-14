//! Host-side bindings of `wit/plugin.wit`.

wasmtime::component::bindgen!({
    path: "../../wit",
    world: "plugin",
});

pub(crate) use qcc::plugin::types as wit;
