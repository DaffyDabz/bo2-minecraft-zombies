//! bo2mc: every Black Ops II perk in the spawn room.
//!
//! Nuketown's own zones hold four perk machines (Quick Revive, Speed Cola,
//! Double Tap, Juggernog) and Pack-a-Punch. The other perks' machines,
//! bottles, HUD icons, jingles, effects and scripts are in the other
//! Zombies maps' zones (measured 10-05, <work-dir>/briefs/perks-census.txt):
//! Stamin-Up and Tombstone in zm_transit, Mule Kick, Who's Who and PhD
//! Flopper's machine in zm_highrise, Deadshot and Electric Cherry in
//! zm_prison, Vulture Aid in zm_buried (its and PhD's scripts in
//! zm_buried_patch), PhD's bottle in zm_tomb. Each such zone is walked and
//! then trimmed to just those assets, so nothing else of those maps loads:
//! no world, no entities, no other scripts, weapons or sounds. The trimmed
//! captures go before Nuketown's, so Nuketown's own copy of anything they
//! share wins.
//!
//! On with `IW4L_BO2MC=1` (Minecraft Zombies) or `IW4L_T6_ALLPERKS=1`, on
//! `zm_nuked` only.

use std::collections::BTreeSet;
use std::path::Path;

use asset_t6::ZoneCapture;

/// The other maps' zones, in load order (earlier ones lose a shared name).
const PERK_ZONES: [&str; 8] = [
    // TranZit Survival's hellhound sounds (bo2mc's souls rounds): no other
    // zone has them (Nuketown's hellhound has none of its own).
    "so_zsurvival_zm_transit",
    "zm_tomb",
    "zm_transit",
    // TranZit's bus (bo2mc's arrival): its sounds.
    "so_zclassic_zm_transit",
    "zm_highrise",
    "zm_prison",
    "zm_buried",
    "zm_buried_patch",
];

/// Their language zones (the perks' hint strings).
const PERK_LANG_ZONES: [&str; 5] = ["zm_tomb", "zm_transit", "zm_highrise", "zm_prison", "zm_buried"];

/// The bus's driver's voice (TEDD), in TranZit's English zone.
const BUS_VOICE_ZONE: &str = "en_so_zclassic_zm_transit";

/// The bus's animations (its doors) and its driver's.
const BUS_ANIMS: [&str; 7] = [
    "v_zombie_bus_all_doors_open",
    "v_zombie_bus_all_doors_close",
    "v_zombie_bus_all_doors_idle_closed",
    "v_zombie_bus_all_doors_idle_open",
    "ai_zombie_bus_driver_idle",
    "ai_zombie_bus_driver_idle_dialog",
    "ai_zombie_bus_driver_idle_dialog_angry",
];

/// Models no Nuketown script names but the perks need: the machines the
/// generic perk script does not know (Deadshot's, Electric Cherry's,
/// Vulture Aid's, PhD Flopper's), Vulture Aid's drops and the Carpenter;
/// and bo2mc's arrival: TranZit's bus and its driver.
pub(crate) const MODELS: [&str; 13] = [
    "veh_t6_civ_bus_zombie",
    "p6_anim_zm_bus_driver",
    "p6_zm_al_vending_ads_on",
    "p6_zm_vending_electric_cherry_off",
    "p6_zm_vending_electric_cherry_on",
    "p6_zm_vending_vultureaid",
    "p6_zm_vending_vultureaid_on",
    "p6_zm_perk_vulture_ammo",
    "p6_zm_perk_vulture_points",
    "zombie_vending_nuke_on_lo",
    "p6_zm_al_vending_nuke_on",
    "ch_tombstone1",
    "zombie_carpenter",
];

/// The perk scripts Nuketown's zones lack (Vulture Aid, PhD Flopper).
const SCRIPTS: [&str; 2] = [
    "script:maps/mp/zombies/_zm_perk_vulture.gsc",
    "script:maps/mp/zombies/_zm_perk_divetonuke.gsc",
];

/// The bottles (weapons) the perks hand over while drinking.
const BOTTLE_PREFIX: &str = "zombie_perk_bottle_";

/// Other maps' weapons loaded whole (no Nuketown script names them; bo2mc's
/// own rules or `weapon_give` hand them out), each with its models,
/// animations, effects, tracers, camo and sounds (`extra_captures`). Mob of
/// the Dead's: the Spork (its spoon) and the Golden Spork (melee, like the Bowie knife), the
/// Blundergat and its Pack-a-Punched Sweeper, Hell's Retriever and its
/// upgrade and the catch animation's weapon (its script, `_zm_weap_tomahawk`,
/// is not ported: thrown, it is a plain grenade).
pub(crate) const WEAPONS: [&str; 7] = [
    "spoon_zm_alcatraz",
    "spork_zm_alcatraz",
    "blundergat_zm",
    "blundergat_upgraded_zm",
    "bouncing_tomahawk_zm",
    "upgraded_tomahawk_zm",
    "zombie_tomahawk_flourish",
];

/// Sound aliases kept: the perks' jingles and stings and their own sounds,
/// and the hellhounds'.
const ALIAS_PARTS: [&str; 15] = [
    "zmb_hellhound_",
    "zmb_dog_",
    "fly_dog_",
    "aml_dog_",
    "zmb_bus_",
    "vox_bus_",
    "perk",
    "cherry",
    "whoswho",
    "chugabud",
    "evt_ww_",
    "vulture",
    "phd",
    "tombstone",
    "divetonuke",
];

/// The other perks' HUD icons (BO2's perk HUD names them by client field).
pub(crate) const HUD_ICONS: [&str; 8] = [
    "specialty_marathon_zombies",
    "specialty_additionalprimaryweapon_zombies",
    "specialty_ads_zombies",
    "specialty_tombstone_zombies",
    "specialty_chugabud_zombies",
    "specialty_electric_cherry_zombie",
    "specialty_vulture_zombies",
    "specialty_divetonuke_zombies",
];

/// Is the extra perk load on for this map?
pub(crate) fn wanted(map: &str) -> bool {
    let on = |k: &str| std::env::var(k).is_ok_and(|v| v == "1");
    map == "zm_nuked" && (on("IW4L_BO2MC") || on("IW4L_T6_ALLPERKS"))
}

fn real(name: &str) -> bool {
    !name.is_empty() && !name.starts_with(',')
}

/// Every printable string constant of the scripts (what `loadfx`,
/// `precachemodel` and `precacheitem` name).
fn script_tokens<'a>(captures: impl Iterator<Item = &'a ZoneCapture>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for c in captures {
        for (name, bytes) in &c.raw_files {
            if !name.starts_with("script:") || !name.ends_with(".gsc") {
                continue;
            }
            for token in bytes.split(|&b| b == 0) {
                if (3..96).contains(&token.len()) && token.iter().all(|b| b.is_ascii_graphic()) {
                    out.insert(String::from_utf8_lossy(token).into_owned());
                }
            }
        }
    }
    out
}

/// The other maps' zones, trimmed to the perks' assets. `base` = the map's
/// own captures (what they already hold is dropped from the extras).
pub(crate) fn extra_captures(
    map_path: &Path,
    base: &[&ZoneCapture],
    report: &mut Vec<String>,
) -> Vec<ZoneCapture> {
    let dir = map_path.parent().unwrap_or(Path::new("."));
    let t0 = std::time::Instant::now();
    let mut caps: Vec<ZoneCapture> = Vec::new();
    // The voice zone sits in zone/english beside zone/all.
    let voice = dir.parent().map(|d| d.join("english").join(format!("{BUS_VOICE_ZONE}.ff")));
    for path in PERK_ZONES.iter().map(|z| dir.join(format!("{z}.ff"))).chain(voice) {
        let z = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        if !path.is_file() {
            report.push(format!("t6 perks: zone {z} not found"));
            continue;
        }
        match asset_t6::capture_zone(&path) {
            Ok(mut c) => {
                // The scripts first: what they name is kept below.
                c.raw_files
                    .retain(|(n, _)| SCRIPTS.contains(&n.as_str()) && !in_base_raw(base, n));
                strip(&mut c);
                caps.push(c);
            }
            Err(e) => report.push(format!("t6 perks: zone gap {e}")),
        }
    }
    // What the scripts (Nuketown's and the kept ones) name.
    let tokens = script_tokens(base.iter().copied().chain(caps.iter()));
    let base_models: BTreeSet<&str> = base
        .iter()
        .flat_map(|c| c.xmodels.iter().map(|m| m.name.as_str()))
        .filter(|n| real(n))
        .collect();
    let base_weapons: BTreeSet<&str> = base
        .iter()
        .flat_map(|c| c.weapons.iter().map(|w| w.name.as_str()))
        .filter(|n| real(n))
        .collect();
    let base_fx: BTreeSet<&str> = base
        .iter()
        .flat_map(|c| c.fx.iter().map(|f| f.name.as_str()))
        .filter(|n| real(n))
        .collect();
    let base_anims: BTreeSet<&str> = base
        .iter()
        .flat_map(|c| c.xanims.iter().map(|a| a.name.as_str()))
        .filter(|n| real(n))
        .collect();
    let base_aliases: BTreeSet<String> = base
        .iter()
        .flat_map(|c| c.sound_banks.iter())
        .flat_map(|b| b.aliases.iter().map(|a| a.name.to_ascii_lowercase()))
        .collect();

    let mut seen_weapons: BTreeSet<String> = BTreeSet::new();
    let mut seen_models: BTreeSet<String> = BTreeSet::new();
    let mut seen_fx: BTreeSet<String> = BTreeSet::new();
    let mut seen_anims: BTreeSet<String> = BTreeSet::new();
    let mut seen_aliases: BTreeSet<String> = BTreeSet::new();
    // Weapons: the perk bottles the scripts name. Later extra zones win a
    // name too: walk them last to first.
    let mut want_models: BTreeSet<String> = MODELS.iter().map(|s| (*s).to_owned()).collect();
    let mut want_anims: BTreeSet<String> = BUS_ANIMS.iter().map(|s| (*s).to_owned()).collect();
    let mut want_fx: BTreeSet<String> = BTreeSet::new();
    // The kept weapons' sounds, tracers and camos.
    let mut want_aliases: BTreeSet<String> = BTreeSet::new();
    let mut want_tracers: BTreeSet<String> = BTreeSet::new();
    let mut want_camos: BTreeSet<String> = BTreeSet::new();
    // The whole weapons asked for, and the alternate modes they switch to.
    let mut whole: BTreeSet<String> = WEAPONS.iter().map(|s| (*s).to_owned()).collect();
    for c in &caps {
        for w in c.weapons.iter().filter(|w| WEAPONS.contains(&w.name.as_str())) {
            if real(&w.alt_weapon_name) {
                whole.insert(w.alt_weapon_name.clone());
            }
        }
    }
    let unprefix = |s: &String| s.trim_start_matches(',').to_owned();
    for c in caps.iter_mut().rev() {
        c.weapons.retain(|w| {
            real(&w.name)
                && ((w.name.starts_with(BOTTLE_PREFIX) && tokens.contains(&w.name)) || whole.contains(&w.name))
                && !base_weapons.contains(w.name.as_str())
                && seen_weapons.insert(w.name.clone())
        });
        for w in &c.weapons {
            want_models.extend(w.gun_models.iter().map(unprefix));
            want_models.extend(w.world_models.iter().map(unprefix));
            want_models.extend(w.models.iter().map(|(_, m)| unprefix(m)));
            want_models.extend(w.attach_view_models.iter().map(|a| unprefix(&a.1)));
            want_anims.extend(w.xanims.iter().map(unprefix));
            want_fx.extend(w.fx.iter().map(|(_, f)| unprefix(f)));
            want_tracers.extend(w.tracers.iter().map(|(_, t)| unprefix(t)));
            if real(&w.camo) {
                want_camos.insert(w.camo.clone());
            }
            // Fire, reload, raise, melee... sounds (every def string: what
            // is no alias matches none), the notetracks' and the bounces'.
            want_aliases.extend(w.strings.iter().map(|(_, s)| s.to_ascii_lowercase()));
            for (_, v) in &w.notetrack_sounds {
                want_aliases.extend(v.split_whitespace().map(str::to_ascii_lowercase));
            }
            want_aliases.extend(w.bounce_sounds.iter().map(|s| s.to_ascii_lowercase()));
        }
    }
    want_models.retain(|m| real(m));
    let base_tracers: BTreeSet<&str> = base.iter().flat_map(|c| c.tracers.iter().map(|t| t.name.as_str())).collect();
    let base_camos: BTreeSet<&str> = base.iter().flat_map(|c| c.weapon_camos.iter().map(|t| t.name.as_str())).collect();
    let mut seen_tracers: BTreeSet<String> = BTreeSet::new();
    let mut seen_camos: BTreeSet<String> = BTreeSet::new();
    for c in caps.iter_mut().rev() {
        c.tracers.retain(|t| {
            want_tracers.contains(&t.name) && !base_tracers.contains(t.name.as_str()) && seen_tracers.insert(t.name.clone())
        });
        c.weapon_camos.retain(|t| {
            want_camos.contains(&t.name) && !base_camos.contains(t.name.as_str()) && seen_camos.insert(t.name.clone())
        });
    }
    for c in caps.iter_mut().rev() {
        // Effects the scripts load, and the effects those run.
        let mut fx_keep: BTreeSet<String> = c
            .fx
            .iter()
            .filter(|f| real(&f.name) && (tokens.contains(&f.name) || want_fx.contains(&f.name)))
            .map(|f| f.name.clone())
            .collect();
        loop {
            let mut more = Vec::new();
            for f in c.fx.iter().filter(|f| fx_keep.contains(&f.name)) {
                for el in &f.elems {
                    for v in &el.visuals {
                        match v {
                            asset_t6::FxVisualRef::Effect(n) if !fx_keep.contains(n) => {
                                more.push(n.clone());
                            }
                            asset_t6::FxVisualRef::Model(m) => {
                                want_models.insert(m.clone());
                            }
                            _ => {}
                        }
                    }
                }
            }
            if more.is_empty() {
                break;
            }
            fx_keep.extend(more);
        }
        c.fx.retain(|f| {
            fx_keep.contains(&f.name)
                && !base_fx.contains(f.name.as_str())
                && seen_fx.insert(f.name.clone())
        });
    }
    for c in caps.iter_mut().rev() {
        c.xmodels.retain(|m| {
            real(&m.name)
                && (want_models.contains(&m.name) || tokens.contains(&m.name))
                && !base_models.contains(m.name.as_str())
                && seen_models.insert(m.name.clone())
        });
        c.xanims.retain(|a| {
            real(&a.name)
                && (want_anims.contains(&a.name) || tokens.contains(&a.name))
                && !base_anims.contains(a.name.as_str())
                && seen_anims.insert(a.name.clone())
        });
    }
    // The sounds the kept weapons' animations cue (`sndnt#<alias>`), and
    // the secondary aliases the wanted ones play along.
    for c in &caps {
        for a in c.xanims.iter().filter(|a| want_anims.contains(&a.name)) {
            for (note, _) in &a.notifies {
                if let Some(alias) = note.strip_prefix("sndnt#") {
                    want_aliases.insert(alias.to_ascii_lowercase());
                }
            }
        }
    }
    loop {
        let more: Vec<String> = caps
            .iter()
            .flat_map(|c| c.sound_banks.iter())
            .flat_map(|b| b.aliases.iter())
            .filter(|a| want_aliases.contains(&a.name.to_ascii_lowercase()))
            .flat_map(|a| a.entries.iter())
            .map(|e| e.secondary.to_ascii_lowercase())
            .filter(|s| !s.is_empty() && !want_aliases.contains(s))
            .collect();
        if more.is_empty() {
            break;
        }
        want_aliases.extend(more);
    }
    for c in caps.iter_mut().rev() {
        for b in &mut c.sound_banks {
            b.aliases.retain(|a| {
                let n = a.name.to_ascii_lowercase();
                (ALIAS_PARTS.iter().any(|p| n.contains(p)) || want_aliases.contains(&n))
                    && !base_aliases.contains(&n)
                    && seen_aliases.insert(n)
            });
        }
        c.sound_banks.retain(|b| !b.aliases.is_empty());
    }
    for c in &caps {
        report.push(format!(
            "t6 perks: {} kept {} weapons {:?}, {} models, {} effects, {} animations, {} sound aliases, {} tracers, {} camos, {} scripts",
            c.zone,
            c.weapons.len(),
            c.weapons.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(),
            c.xmodels.len(),
            c.fx.len(),
            c.xanims.len(),
            c.sound_banks.iter().map(|b| b.aliases.len()).sum::<usize>(),
            c.tracers.len(),
            c.weapon_camos.len(),
            c.raw_files.len(),
        ));
    }
    report.push(format!(
        "t6 perks: {} extra zones in {:.1} s",
        caps.len(),
        t0.elapsed().as_secs_f32()
    ));
    caps
}

fn in_base_raw(base: &[&ZoneCapture], name: &str) -> bool {
    base.iter().any(|c| c.raw_files.iter().any(|(n, _)| n == name))
}

/// Everything of another map that is never wanted.
fn strip(c: &mut ZoneCapture) {
    c.world = None;
    c.clip = None;
    c.primary_lights.clear();
    c.string_tables.clear();
    c.localize.clear();
    c.path_nodes.clear();
    c.map_ents.clear();
    // Camos and tracers: only those a kept weapon names (`extra_captures`).
    c.fonts.clear();
    c.impact_tables.clear();
    c.footstep_tables.clear();
    c.footstep_fx_tables.clear();
    c.snd_globals = None;
}

/// The other maps' language zones: their strings only (the perks' hints).
pub(crate) fn extra_lang_captures(lang_dir: Option<&Path>, report: &mut Vec<String>) -> Vec<ZoneCapture> {
    let Some(dir) = lang_dir else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for z in PERK_LANG_ZONES {
        let path = dir.join(format!("en_{z}.ff"));
        if !path.is_file() {
            continue;
        }
        match asset_t6::capture_zone(&path) {
            Ok(mut c) => {
                let localize = std::mem::take(&mut c.localize);
                let mut keep = ZoneCapture::default();
                keep.zone = c.zone;
                keep.localize = localize;
                out.push(keep);
            }
            Err(e) => report.push(format!("t6 perks: language zone gap {e}")),
        }
    }
    out
}
