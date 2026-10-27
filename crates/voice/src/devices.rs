use std::sync::Mutex;

use cpal::traits::{DeviceTrait, HostTrait};
use cpal::Device;
use serde_json::{json, Value as Json};

// the output the mod picked, by its id; None: the system's default
static SPEAKER: Mutex<Option<String>> = Mutex::new(None);

pub fn set_speaker(id: Option<String>) {
    *SPEAKER.lock().expect("speaker") = id;
}

pub fn id_of(d: &Device) -> Option<String> {
    d.id().ok().map(|id| id.to_string())
}

pub fn find(id: &str, input: bool) -> Option<Device> {
    let host = cpal::default_host();
    let mut all = host.devices().ok()?;
    all.find(|d| (if input { d.supports_input() } else { d.supports_output() }) && id_of(d).as_deref() == Some(id))
}

// `id` when it is still connected, else the system's default
pub fn input(id: Option<&str>) -> Option<Device> {
    id.and_then(|id| find(id, true)).or_else(|| cpal::default_host().default_input_device())
}

pub fn output() -> Option<Device> {
    let id = SPEAKER.lock().expect("speaker").clone();
    id.and_then(|id| find(&id, false)).or_else(|| cpal::default_host().default_output_device())
}

// { inputs: [{ id, name }], outputs: [...] }, the defaults first
pub fn list() -> Json {
    let host = cpal::default_host();
    let named = |input: bool| -> Vec<Json> {
        let default = if input { host.default_input_device() } else { host.default_output_device() }.and_then(|d| id_of(&d));
        let mut out: Vec<(bool, Json)> = host
            .devices()
            .map(|all| {
                all.filter(|d| if input { d.supports_input() } else { d.supports_output() })
                    .filter_map(|d| {
                        let id = id_of(&d)?;
                        Some((default.as_deref() == Some(id.as_str()), json!({ "id": id, "name": d.to_string() })))
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by_key(|(is_default, _)| !is_default);
        out.into_iter().map(|(_, d)| d).collect()
    };
    json!({ "type": "devices", "inputs": named(true), "outputs": named(false) })
}
