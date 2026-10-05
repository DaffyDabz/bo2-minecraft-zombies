//! Minecraft's sound events on the Minecraft map, resolved as the game
//! resolves them: `sounds.json` names each event's weighted variants (files,
//! or other events) with their own volume and pitch.
use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::Vec3;
use minecraft_terrain::pack::{PackStack, ResourceId};

struct Entry {
    name: String,
    volume: f32,
    pitch: f32,
    weight: u32,
    /// `name` is another event rather than a file.
    event: bool,
}

pub(crate) struct Sounds {
    events: HashMap<String, Vec<Entry>>,
    files: HashMap<String, Option<Arc<[u8]>>>,
    rng: u64,
    unresolved: std::collections::HashSet<String>,
    /// Requests for the audio system this frame.
    pub(crate) queued: Vec<audio::McSoundRequest>,
}

impl Sounds {
    pub(crate) fn load(packs: &PackStack) -> Self {
        let mut events = HashMap::new();
        let json = ResourceId::parse("minecraft:sounds")
            .ok()
            .and_then(|id| packs.json(&id, "sounds.json").ok().flatten());
        if let Some(entries) = json.as_ref().and_then(serde_json::Value::as_object) {
            for (event, value) in entries {
                let list = value["sounds"]
                    .as_array()
                    .map(|sounds| {
                        sounds
                            .iter()
                            .filter_map(|sound| match sound {
                                serde_json::Value::String(name) => Some(Entry {
                                    name: name.clone(),
                                    volume: 1.0,
                                    pitch: 1.0,
                                    weight: 1,
                                    event: false,
                                }),
                                serde_json::Value::Object(o) => Some(Entry {
                                    name: o.get("name")?.as_str()?.to_owned(),
                                    volume: o.get("volume").and_then(serde_json::Value::as_f64).unwrap_or(1.0) as f32,
                                    pitch: o.get("pitch").and_then(serde_json::Value::as_f64).unwrap_or(1.0) as f32,
                                    weight: o.get("weight").and_then(serde_json::Value::as_u64).unwrap_or(1) as u32,
                                    event: o.get("type").and_then(serde_json::Value::as_str) == Some("event"),
                                }),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                events.insert(event.clone(), list);
            }
        }
        diag::info!(World, "Minecraft sounds: {} events", events.len());
        Self {
            events,
            files: HashMap::new(),
            rng: 0x9e37_79b9_7f4a_7c15,
            unresolved: std::collections::HashSet::new(),
            queued: Vec::new(),
        }
    }

    pub(crate) fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        ((self.rng >> 40) as u32 as f32) / ((1u32 << 24) as f32)
    }

    /// Plays `event` at a map point (or at the listener), picking one of its
    /// variants by weight.
    pub(crate) fn play(&mut self, packs: &PackStack, event: &str, position: Option<Vec3>, volume: f32, pitch: f32) {
        self.play_depth(packs, event, position, volume, pitch, 0);
    }

    fn play_depth(&mut self, packs: &PackStack, event: &str, position: Option<Vec3>, volume: f32, pitch: f32, depth: u8) {
        let key = event.strip_prefix("minecraft:").unwrap_or(event);
        let total: u32 = self.events.get(key).map_or(0, |e| e.iter().map(|x| x.weight).sum());
        if total == 0 || depth > 4 {
            if self.unresolved.insert(key.to_owned()) {
                diag::info!(World, "Minecraft sound event `{event}` has no sounds");
            }
            return;
        }
        let mut pick = (self.random() * total as f32) as u32;
        let Some(entries) = self.events.get(key) else { return };
        let Some(entry) = entries.iter().find(|entry| {
            if pick < entry.weight {
                true
            } else {
                pick -= entry.weight;
                false
            }
        }) else {
            return;
        };
        let (name, entry_volume, entry_pitch, is_event) = (entry.name.clone(), entry.volume, entry.pitch, entry.event);
        if is_event {
            self.play_depth(packs, &name, position, volume * entry_volume, pitch * entry_pitch, depth + 1);
            return;
        }
        let bytes = self
            .files
            .entry(name.clone())
            .or_insert_with(|| {
                let (namespace, path) = name.split_once(':').unwrap_or(("minecraft", &name));
                let id = ResourceId::parse(&format!("{namespace}:{path}")).ok()?;
                packs.get(&id, &format!("sounds/{path}.ogg")).ok().flatten().map(Arc::from)
            })
            .clone();
        let Some(bytes) = bytes else {
            if self.unresolved.insert(name.clone()) {
                diag::info!(World, "Minecraft sound file `{name}` is not in the pack");
            }
            return;
        };
        self.queued.push(audio::McSoundRequest {
            key: name,
            bytes,
            position,
            volume: volume * entry_volume,
            pitch: pitch * entry_pitch,
            block: sim::voxel::BLOCK,
        });
    }
}
