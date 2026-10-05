//! bo2zm: read a Black Ops II map's compiled scripts, read-only.
//! `t6gsc [zone.ff]...` (default the Nuketown patch zone) checks the
//! decoder on every script (string references and import call sites read
//! where the object's tables say) and prints what the map scripts say
//! about effects and ambience.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::process::ExitCode;

use asset_t6::{GscObject, MapScriptFacts, capture_zone};

const DEFAULT_ZONE: &str =
    r"<Black Ops II folder>\zone\all\zm_nuked_patch.ff";

fn main() -> ExitCode {
    let mut zones: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    if zones.is_empty() {
        zones.push(PathBuf::from(DEFAULT_ZONE));
    }
    let mut scripts: HashMap<String, Vec<u8>> = HashMap::new();
    for zone in &zones {
        match capture_zone(zone) {
            Ok(c) => {
                for (name, bytes) in c.raw_files {
                    if let Some(name) = name.strip_prefix("script:") {
                        scripts.entry(name.to_owned()).or_insert(bytes);
                    }
                }
            }
            Err(e) => {
                eprintln!("{}: {e}", zone.display());
                return ExitCode::FAILURE;
            }
        }
    }
    let (mut s_hit, mut s_all, mut i_hit, mut i_all, mut bad) = (0, 0, 0, 0, 0);
    let sorted: BTreeMap<_, _> = scripts.iter().collect();
    for (name, bytes) in &sorted {
        if name.starts_with(',') {
            continue;
        }
        match GscObject::parse(bytes) {
            Ok(g) => {
                let (a, b, c, d) = g.coverage();
                if a != b || c != d {
                    println!("MISMATCH {name}: strings {a}/{b} imports {c}/{d}");
                }
                s_hit += a;
                s_all += b;
                i_hit += c;
                i_all += d;
            }
            Err(e) => {
                bad += 1;
                println!("PARSE FAIL {name}: {e}");
            }
        }
    }
    println!(
        "scripts {}: strings {s_hit}/{s_all}, imports {i_hit}/{i_all}, parse failures {bad}",
        sorted.len()
    );
    let map = std::env::var("T6GSC_MAP").unwrap_or_else(|_| "zm_nuked".to_owned());
    let facts = MapScriptFacts::read(&map, |n| scripts.get(n).cloned());
    println!("\n== {map}: {} effect table entries", facts.effects.len());
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    for p in &facts.placed {
        *kinds.entry(p.kind.as_str()).or_default() += 1;
    }
    println!("placed {}: {kinds:?}", facts.placed.len());
    let unresolved: Vec<&str> = facts
        .placed
        .iter()
        .filter(|p| !facts.effects.contains_key(&p.fxid))
        .map(|p| p.fxid.as_str())
        .collect();
    println!("placed fxids without an effect table entry: {}", unresolved.len());
    for u in unresolved.iter().take(10) {
        println!("   {u}");
    }
    for p in facts.placed.iter().take(6) {
        println!(
            "   {} {} -> {} at {:?} angles {:?} delay {} exploder {:?}",
            p.kind,
            p.fxid,
            facts.effects.get(&p.fxid).map_or("?", String::as_str),
            p.origin,
            p.angles,
            p.delay,
            p.exploder
        );
    }
    if std::env::var_os("T6GSC_PLACED").is_some() {
        for p in &facts.placed {
            println!(
                "placed	{}	{}	{}	{:.0}	{:.0}	{:.0}	{:.0}	{:.0}	{:.0}	{}",
                p.kind,
                p.fxid,
                facts.effects.get(&p.fxid).map_or("?", String::as_str),
                p.origin[0],
                p.origin[1],
                p.origin[2],
                p.angles[0],
                p.angles[1],
                p.angles[2],
                p.delay
            );
        }
    }
    println!("room tone {:?}", facts.room_tone);
    println!("sounds on effects {}:", facts.sounds_on_fx.len());
    for (fxid, alias, off) in &facts.sounds_on_fx {
        let n = facts.placed.iter().filter(|p| &p.fxid == fxid).count();
        println!("   {fxid} -> {alias} offset {off:?} ({n} placed)");
    }
    println!("loops at points {}: {:?}", facts.loops_at.len(), facts.loops_at);
    if std::env::var_os("T6GSC_CALLS").is_some() {
        for name in ["clientscripts/mp/zm_nuked_amb.csc"] {
            if let Some(g) = scripts.get(name).and_then(|b| GscObject::parse(b).ok()) {
                for c in g.run().calls {
                    println!("call [{}] {}({:?})", c.caller, c.function, c.args);
                }
            }
        }
    }
    ExitCode::SUCCESS
}
