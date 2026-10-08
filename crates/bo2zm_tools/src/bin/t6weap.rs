//! bo2mc: what a Black Ops II weapon needs, read-only.
//!
//! `t6weap <zone.ff>... -- <name substring>...` prints, for each weapon whose
//! name holds one of the substrings: its strings (sounds, alt weapon...),
//! effects, models, materials, tracers, camo, animations (and the sounds
//! their notetracks name), notetrack sounds and bounce sounds; then each
//! script holding a substring among its string constants, with those
//! constants (`T6WEAP_SCRIPTS=0` skips the scripts).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::ExitCode;

use asset_t6::capture_zone;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let split = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let zones: Vec<PathBuf> = args[..split].iter().map(PathBuf::from).collect();
    let wants: Vec<String> = args.get(split + 1..).unwrap_or(&[]).to_vec();
    let scripts_on = std::env::var("T6WEAP_SCRIPTS").map_or(true, |v| v != "0");
    for zone in &zones {
        let c = match capture_zone(zone) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{}: {e}", zone.display());
                return ExitCode::FAILURE;
            }
        };
        let hit = |n: &str| wants.iter().any(|w| n.contains(w.as_str()));
        for w in c.weapons.iter().filter(|w| !w.name.starts_with(',') && hit(&w.name)) {
            println!("== {} weapon {} (display {:?})", c.zone, w.name, w.display_name);
            println!("   alt {:?} ammo {:?} clip {:?} camo {:?}", w.alt_weapon_name, w.ammo_name, w.clip_name, w.camo);
            println!("   gun {:?}", w.gun_models.iter().filter(|m| !m.is_empty()).collect::<Vec<_>>());
            println!("   world {:?}", w.world_models.iter().filter(|m| !m.is_empty()).collect::<Vec<_>>());
            println!("   attach {:?}", w.attach_view_models.iter().map(|a| &a.1).collect::<Vec<_>>());
            println!("   strings {:?}", w.strings);
            println!("   fx {:?}", w.fx);
            println!("   models {:?}", w.models);
            println!("   materials {:?} overlay {:?} dpad {:?}", w.materials, w.overlay_material, w.dpad_icon);
            println!("   tracers {:?}", w.tracers);
            println!("   notetrack sounds {:?}", w.notetrack_sounds);
            let bounce: BTreeSet<&String> = w.bounce_sounds.iter().filter(|s| !s.is_empty()).collect();
            println!("   bounce {bounce:?}");
            let anims: Vec<&String> = w.xanims.iter().filter(|a| !a.is_empty()).collect();
            println!("   anims {}: {anims:?}", anims.len());
            let mut snd: BTreeSet<String> = BTreeSet::new();
            for a in c.xanims.iter().filter(|a| anims.iter().any(|n| n.trim_start_matches(',') == a.name)) {
                for (note, _) in &a.notifies {
                    if let Some(s) = note.strip_prefix("sndnt#") {
                        snd.insert(s.to_owned());
                    }
                }
            }
            println!("   anim sounds {snd:?}");
            let missing: Vec<&&String> = anims
                .iter()
                .filter(|n| !c.xanims.iter().any(|a| a.name == n.trim_start_matches(',')))
                .collect();
            println!("   anims not in this zone {}: {missing:?}", missing.len());
        }
        // The string-table rows (`mp/zombiemode.csv`: costs, ...) naming a
        // wanted weapon.
        for t in &c.string_tables {
            for row in t.cells.chunks(t.columns.max(1)) {
                if row.iter().any(|cell| hit(cell)) {
                    println!("== {} table {}: {row:?}", c.zone, t.name);
                }
            }
        }
        if scripts_on {
            for (name, bytes) in c.raw_files.iter().filter(|(n, _)| n.starts_with("script:")) {
                let tokens: BTreeSet<String> = bytes
                    .split(|&b| b == 0)
                    .filter(|t| (3..96).contains(&t.len()) && t.iter().all(u8::is_ascii_graphic))
                    .map(|t| String::from_utf8_lossy(t).into_owned())
                    .collect();
                if hit(name) || tokens.iter().any(|t| hit(t)) {
                    println!("== {} {name}: {} tokens", c.zone, tokens.len());
                    // `T6WEAP_CALLS=a,b`: the calls to those functions, with
                    // their arguments as the decoder reads them.
                    if let Ok(fns) = std::env::var("T6WEAP_CALLS")
                        && let Ok(g) = asset_t6::GscObject::parse(bytes)
                    {
                        let run = g.run();
                        for f in fns.split(',') {
                            for (_, call) in run.calls_named(f) {
                                let args: Vec<String> = call.args.iter().map(|a| format!("{a:?}")).collect();
                                println!("   {} -> {}({})", call.caller, call.function, args.join(", "));
                            }
                        }
                    }
                    if hit(name) || std::env::var_os("T6WEAP_ALL").is_some() {
                        println!("   {tokens:?}");
                    } else {
                        println!("   {:?}", tokens.iter().filter(|t| hit(t)).collect::<Vec<_>>());
                    }
                }
            }
        }
    }
    ExitCode::SUCCESS
}
