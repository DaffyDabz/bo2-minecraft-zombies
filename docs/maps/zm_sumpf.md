# Shi No Numa (`zm_sumpf`, Zombies Declassified)

Owned by the declassified lane (branch `zm-declassified`). Files, load
order and the mod's fix scripts: see `zm_prototype.md` (same pack, same
loader). Run: `dzrun.sh NAME SECS "cmds" zm_sumpf`.

## What works (2026-10-07)

- Loads with no change: 412 entities, 2898 path nodes, the player starts
  upstairs in the main hut (Japanese zombies, pistol, 500 points).
- Rounds (test player `IW4L_T6_AUTOPLAY=2`, `sumpf2`): round 2 at 85 s,
  3 at 160 s, 4 at 280 s, 4990 points, 0 panics. `zzz_zm_roundfix` covers
  it (one round driver).
- Doors: 9 zombie_door + 2 zombie_debris triggers, all bought with
  `IW4L_T6_OPENALL=buy`; all 11 script zones enabled (main hut 3, four huts,
  four outside zones).
- Perk shuffle (the map's own `waitfor_flag_open_chest_location` in
  `zm_sumpf_magic_box`): `randomize_vending_machines` once, then
  `vending_randomization_effect(i)` when door flag `<nw|ne|se|sw>_magic_box`
  and `<dir>_building_unlocked` are both set (index 0 nw, 1 ne, 2 se, 3 sw);
  Quick Revive stays at the main hut (location 4, (9563 336 -529)). The four
  hut spots: (8520 3196 -667) nw, (11673 3603 -656) ne, (12382 -1200 -644)
  se, (7843 -1185 -683) sw. The machines keep their plain (not `_on`) models,
  as the map's own `vending_model_info` says. The mod's
  `zzz_zz_zzzzzzzsumpfshuffle` is only a safety net for the map path; not
  needed here (the map path runs).
- Jugger-Nog bought at its hut spot: 10000 -> 7500, `specialty_armorvest`.
- 20-min soak (`sumpfsoak`, test player): round 10 at 1100 s, 23000
  points, 0 panics, clean exit.

## Hellhounds (not working yet)

Dog rounds come (round 5 and 10 in the soak; forced to round 2 with
`IW4L_T6_DZDOGROUND=2`, run `dogs1`): `zombie_wolf` actors spawn by the
main hut, but they stand still and never attack. Actor log (115-130 s):
`script stop anim - hp 400 path 0/0 next (0 0 0) goal (0 0 0)`; player
health stays 100, points stop. The round then ends only by the failsafe.
Next: trace the map's dog think (`zm_sumpf_dogs` / `_zm_ai_dogs`) to see
which goal call our actors miss.

## The map's scripts

Decoded with `<work-dir>\tools\gscdis.py` from the zone image
(`T6ZONE_DUMP=. t6zone.exe zm_sumpf.ff`, then each script carved from its
GSC magic after its name up to the next script's name):
`<work-dir>\lanes\declassified\dump_sumpf\*.txt` (zm_sumpf, _perks,
_magic_box, _trap_pendulum, _zipline, _trap_perk_electric).

- Flogger (`zm_sumpf_trap_pendulum`): buy trigger `pendulum_buy_trigger`;
  `activatepen` turns a damage trigger on that is LINKED to the spinning
  log (`enablelinkto` / `linkto`, rotatepitch 6 x 30 s), `pendamage` waits
  for `trigger` and launches zombies (`do_launch`).
- Zipline (`zm_sumpf_zipline`, started when `ne_magic_box` opens).
- Electric trap (`zm_sumpf_trap_perk_electric`): the same code as
  Verrückt's (`electric_trap_think`, `zombie_elec_death`).
- Hellhounds: `zm_sumpf_dogs`, `next_dog_round`, `dog_round` flag.

## Test aids

- `IW4L_T6_DZENTS=<s>[,<s>...]:<text>[,...]` (`declassified.rs`): at those
  seconds, each entity whose targetname / classname / model /
  script_noteworthy holds a text: place, model, hidden.
- `IW4L_T6_DZDOGROUND=<round>`: at 10 s sets `level.next_dog_round`, so
  the hellhounds come at that round.
- `IW4L_T6_OPENALL=buy` now buys each door once, lowest entity first, even
  when a bought debris trigger is deleted (it used to skip the next one).

## Not done yet

- Hellhounds that move and attack, the flogger, the zipline, the electric
  traps, the box, Wunderwaffe.
