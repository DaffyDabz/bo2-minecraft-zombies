# zm_prison (Mob of the Dead)

Owned by the prison lane (branch `zm-prison`). Journal: `<work-dir>\lanes\prison\JOURNAL.md`.

## How the map starts (from its own scripts)

- Game type `zclassic`, mode group `zclassic`, location `prison` (`zm_prison::main` sets
  `level.default_game_mode`; `_zm_utility::is_classic` reads `ui_zm_gamemodegroup`, hash 6b64b9b4;
  `is_gametype_active` reads `g_gametype`, hash 4f118387). Nuketown is the only `zstandard` map.
- `gamemode_callback_setup` -> `zm_alcatraz_gamemodes::init`: `zclassic` (preinit
  `zm_prison::zclassic_preinit` = `init_characters`) and location `prison`
  (`zm_alcatraz_classic::precache` / `::main`). `zgrief` + `cellblock` is the Grief mode (Cell Block).
- Classic start zones: `zone_start`, `zone_library`. Zombie limit 24, cull distance 18000.
- `zm_alcatraz_classic::give_afterlife`: after `initial_players_connected` + 0.5 s every player is
  `fake_kill_player`ed into Afterlife; the body lies at struct `corpse_starting_point_<n>`.
- Zones: `zm_prison.ff`, `zm_prison_patch.ff`, and the mode zone `so_zclassic_zm_prison.ff` (the four
  characters' viewhands, `vision/zm_afterlife.vision`, 2 weapons; no scripts).
- Zombie types are in the map zones: `aitype/zm_alcatraz_basic`, `aitype/zm_alcatraz_brutus`.

## Reading the scripts

`T6CENSUS_DUMPALL=<work-dir>/lanes/prison/gscdump t6census zm_prison`, then
`python <work-dir>/lanes/prison/tools/t6dec.py <script> <function>...` (lane copy of the decompiler).
Run the game: `bash <work-dir>/lanes/prison/tools/prun.sh NAME SECONDS`.

## Afterlife start (measured 2026-10-06)

`give_afterlife` -> `fake_kill_player` -> `_zm_afterlife::afterlife_laststand` -> `afterlife_fake_death`, which
takes all weapons, goes prone and loops `while (self is_jumping()) wait 0.05`. `_zm_utility::is_jumping` is
`!isdefined(self getgroundent())`, so with `getgroundent` unbound the player never "lands" and Afterlife never
begins (no corpse, no blue vision, no lightning hands; the HUD shows 500 and 0/0). Fixed by binding
`getgroundent` (`natives_player.rs`). After it: `afterlife_spawn_corpse` (`_zm_clone::spawn_player_clone`),
`afterlife_fake_revive` (gives `lightning_hands_zm`, score 0), `afterlife_enter` (vision `zm_afterlife` via
`_visionset_mgr`, `enableafterlife`, model `c_zom_hero_ghost_fb`, viewmodel `c_zom_ghost_viewhands`), then the
corpse waits for `player_revived`.

`fake_kill_player` places the body at struct `corpse_starting_point_<n>` through `physicstrace(...)["position"]`,
so `physicstrace` must return a trace array (it returned a point: "vector index is not a number"). With both
fixes (runs/al2, al3): body and revive glow in front of the player, W + hold E revives (`afterlife_revive_do_revive`,
`afterlife_leave`), HUD 500 and pistol 8/24 + grenades; `round_start` -> `afterlife_start_zombie_logic`, and
`round_spawning` starts ~88 s in (zombie_total 4 -> 0 by 100 s). No zombie seen on screen yet at 100 s.

## Afterlife lives (2026-10-07, runs/lv1, lv2, rv1)

Solo (`level.is_forever_solo_game` 1): `init_player` sets `lives` 3; the start Afterlife (`fake_kill_player` ->
`afterlife_remove`) takes one (2); 2 s after `afterlife_start_over`, `afterlife_start_zombie_logic` gives it back
(3); every `end_of_round` + 2 s `afterlife_player_refill_watch` adds one (max 3, 1 in co-op). A killing hit with
lives > 0 (`afterlife_player_damage_callback`) takes one and starts `afterlife_laststand` (health 1000 -> ghost).
Mana: 200, drains `0.05 * afterlifedeaths * 3` every 0.05 s (67 s for the first death, half that for the second),
paused while being revived. Out of mana: `afterlife_leave(0)` -> `afterlife_remove(1)` (lives 0) and
`dodamage(1000, origin)` -> down for good (solo: game over). IW4L_T6_LIVES=1 logs lives/afterlife/health on change.
- lv1 (before the fix): no revive, mana out at 79.8 s, lives 0 but he stayed alive. Our `dodamage` passed the
  player as his own attacker; `_zm::callback_playerdamage` drops self damage that is not explosive/burn
  ("damage type verbotten"). Fixed (fb9445a): no attacker = undefined (world damage).
- lv2: same at 76.8 s -> `player_laststand` -> intermission at 82 s. That is BO2's rule, not a bug.
- rv1: start 2, revived at the body at 55 s, add at 57.8 s -> 3; zombie hit at 107 s -> Afterlife (2, health 940).
- soak1 (IW4L_T6_AUTOPLAY=1, 1280 s, 0 panics, exit 0): 5 games; each: start Afterlife, then 3 downs to Afterlife
  revived at the body (21 revives in all), 4th hit with lives 0 -> `player_laststand` -> game over -> map restart.
  The bot only has the pistol and runs dry in round 2 (max round 3), so games are short.

## Screenshots

A minimized test window screenshots as a 1x1 black PNG (the log still says "wrote"). Something on the desktop
minimizes test windows mid-run; `prun.sh` starts `<work-dir>/lanes/prison/tools/keepup.ps1`, which restores only
this lane's window (SW_SHOWNOACTIVATE, no focus). With it: 1280x720 shots (runs/ww2).

## Rounds (measured 2026-10-06, runs/s1)

With zm-tomb's window-board fix (e893b7d + fd991f7: barrier types read from `ZBarrierDef`, back-to-back
`animscripted`) zombies climb in the gondola-shaft windows, tear the boards and come to the player; before it they
stood at the windows. Round 1 starts ~88 s in (the Afterlife start). The zombies Mob deletes are
`zm_alcatraz_distance_tracking` (far and unseen, every 10 s, put back in `zombie_total`); the ones killed at the
player with `dodamage(816)` after a revive are the Afterlife revive shock (real behaviour).

Soak `IW4L_T6_AUTOPLAY=2 IW4L_T6_CENSUS=1 prun.sh s1 600`: rounds 1-8 in 880 s, 271 zombies, none stood still,
0 panics. The rounds test's M14 (given at 8 s) is lost: Afterlife gives back the loadout saved before it, so the
test player fights with the M1911.

Test player: `autoplay_prison.rs` walks a player in Afterlife (`self.afterlife`) to his body
(`self.e_afterlife_corpse`) and holds Use, so autoplay runs get past the start and every later down.
`prun.sh` takes `PRE="..."` console commands run before the waits (e.g. `+forward; wait 1s; -forward; +activate;`).

## Afterlife shock boxes (2026-10-07, runs/sh1-sh9)

Shock boxes are script models `targetname afterlife_interact` (`_zm_afterlife::afterlife_interact_object_think`):
`setcandamage(1)`, then each "damage" from a player holding `lightning_hands_zm` within 256 units does
`level notify(self.script_string)` and swaps the model to `p6_zm_al_shock_box_on`. The perk scripts wait for
those notifies (`sleight_on` = Speed Cola, `juggernog_on`, `doubletap_on`, `deadshot_on`, `electric_cherry_on`;
no `revive_on` box). Other boxes: `gondola_powered_on_roof/_docks`, `laundry_power_switch_afterlife`,
`tower_trap_upgraded`, `intro_powerup_activate`, `cell_1/2_powerup_activate`.

- Engine: shots, knives and blasts now also reach damageable script models (`triggers::damage_triggers`, the
  model's box from `brushes::model_box`): "damage" with the usual 10 args, no "trigger".
- `lightning_hands_zm` (and `electrocuted_hands_zm`) have clip 0, start 0, max 0 and BO2's `unlimitedAmmo` = 1
  (`t6m2 --weapons` with `T6M2_AMMO=1`, zones zm_prison + so_zclassic_zm_prison). The engine now reads that
  flag (`unlimited_ammo` in the weapon facts): such a weapon fires with an empty clip and never spends ammo.
  Nuketown has no unlimited-ammo weapon. Measured runs/sh8-sh9: the hands fire every 320 ms, the bullet
  reaches the `sleight_on` box and the box turns to `p6_zm_al_shock_box_on`.
- Test aid: `IW4L_T6_SHOCK=<script_string>` (`autoplay_prison::shock_test`): logs every box at 1 s; once the
  player holds the lightning hands (the Afterlife start, 10 to 40 s in, slower while other games load) puts him
  at the first spot 80-160 units out whose eye line reaches the box, taps fire for 20 s (weapon, clip and ammo
  facts logged each second), logs the boxes again, then puts him by his body holding Use for 10 s (revived).
  With `IW4L_T6_PERK=<machine targetname>` too, the perk test (10000 points, use at the machine, perks logged
  8 s later) runs right after. `IW4L_T6_HITLOG=1` logs each shot against each damageable model's box.

## Perks (2026-10-07, runs/pk1, pj, pd3, pds2, pec2)

The machines show their "_on" model from the start (Mob's `machine_assets` off_model = on_model: normal); the
use trigger says "You must turn on the Power first!" until the box is shocked. Shock box, then buy, measured
(points 10000 before):

| box | machine targetname | perk | cost |
|---|---|---|---|
| `sleight_on` | `vending_sleight` | `specialty_fastreload` (Speed Cola) | 3000 |
| `juggernog_on` | `vending_jugg` | `specialty_armorvest` | 2500 |
| `doubletap_on` | `vending_doubletap` | `specialty_rof` | 2000 |
| `deadshot_on` | `vending_deadshot_model` | `specialty_deadshot` | 1500 |
| `electric_cherry_on` | `vendingelectric_cherry` | `specialty_grenadepulldeath` | 2000 |

No Quick Revive on Mob (Afterlife). Speed Cola's icon shows on the HUD (runs/pk1/zm_prison.png).

## Doors and box (2026-10-07, runs/dr1-dr3, ds2, dro, sl1)

- `IW4L_T6_DOORS=1` (or a comma list of script_flags) runs the door sweep. It starts once the player holds the
  lightning hands: he is revived at his body, then every 4 s gets 10000 points, stands in the next
  zombie_door/zombie_debris trigger looking at what it opens, holds Use, and the flag, `_door_open`, points and
  hint are logged. With `IW4L_T6_SHOCK` also set, it waits for the shock/perk test to finish first. The box test
  runs after the sweep and logs each box's unitrigger zone (enabled/active).
- runs/dr2: 16 doors buy by hand (flag set, `_door_open` 1, cost taken). 3 doors are `afterlife_door` and read
  "Door needs power". They open only when their shock box (`p6_zm_al_shock_box*`, the struct's target) is zapped
  with the lightning hands, and then they are free:
  - warden's office (ent284) via pf3663
  - showers (ent497) via pf3765
  - roof (ent329, listed at 750) via pf3687
- The box starts at the warden's office (-780 9002 1336). Its use trigger is a unitrigger, which is only offered
  in active zones. zone_warden_office stays inactive until the office door opens. After that (dr3), the box
  takes 950 and gave minigun_alcatraz_zm, a real Mob box weapon.
- Brutus: `brutus_round_tracker` skips rounds < 9 in a solo game (`is_forever_solo_game`), so seeing no Brutus
  in the rounds 1-8 soak (s1) is correct.

## Brutus (2026-10-07, runs/br1-br3)

- `IW4L_T6_BRUTUS=<seconds>` does `level notify("spawn_brutus", 1)` at that match time (what Mob's devgui
  `spawn_Brutus` dvar does); any value (0 = no notify) logs each living Brutus (`is_brutus`) every 2 s: place,
  distance, health, `brutus_lockdown_state`, `has_helmet`, script state and animation.
- br1 (before the fix): every Brutus ran toward the player, played `ai_zombie_cellbreaker_enrage_start` (the
  `zm_taunt` of `brutus_stuck_teleport`) and was deleted and respawned with the same health every ~14 s.
  Cause: `findpath` was unbound, so `brutus_stuck_watcher` counted every check as a failed path
  (`brutus_failed_paths_to_teleport` = 4). `findpath` is now bound (natives_ai.rs): true when the path nodes
  join the two points.
- br2 (after): spawns at a `brutus_spawn` spot, runs to the player, swings (`attack_swingright_a/left`),
  loses his helmet to gunfire (`has_helmet` 0), health 1000 -> 137 -> killed at ~136 s (points +1050).
- br3 (`IW4L_T6_ROUND_JUMP=9`, no forced spawn): `next_brutus_round` is 5 at the start; in solo the tracker
  waits for round 9+, and Brutus came by himself after round 10 started (170 s), health 1000 (first Brutus:
  `brutus_health_increase` x `brutus_round_count`), died in round 11.
- Known: `sndbrutusvox` stops on `soundgetplaybacktime` (unbound here; fixed on zm-highrise 09d211f, comes with
  the bo2zm merge), so `level.sndbrutusistalking` stays 1 and Brutus says only his first line.
- br5 (damage on, `IW4L_T6_AUTOPLAY=1 IW4L_T6_AUTOPLAY_GUN=m14_zm`: the gun and ammo refills of the rounds test):
  Brutus first takes a blocker as his `priority_item` (`ai_state blocker`, item at 285 10415 1367), stands idle
  ~16 s, then chases (`find_flesh`); his swing takes the player to 1-2 health and he drops into Afterlife (lives
  3 -> 2, health 901); the ghost revives at the body; Brutus died at ~155 s. br6 (knife only): Brutus spawns by
  the player, aggros (within 128 units, `brutus_aggro_dist_sq`) and is knifed to death.
- Not yet seen: Brutus locking a perk machine / the box / a table (he only goes for items in the player's zone
  when the player is not within 128 units).

## Brutus spawn with the player at a perk (2026-10-07, runs/bl1-bl4)

- bl1 (`IW4L_T6_BRUTUS=30`, no autoplay: the ghost never walks back to its body): `get_best_brutus_spawn_pos`
  errors "size of undefined": `level.zombie_brutus_locations` is only made by `_zm_zonemgr::create_spawner_list`
  once the zone manager runs, which waits for the start Afterlife to end. A test-only case.
- bl2-bl4 (`IW4L_T6_SHOCK=juggernog_on IW4L_T6_PERK=vending_jugg IW4L_T6_GOD=1 IW4L_T6_BRUTUS=105/110`): the
  player buys Juggernog (7500 left) and stands at (513 6647 208) in `zone_start` (is_occupied 1, enabled 1, 2
  brutus spots). `spawn_brutus` runs `spawn_zombie` -> `brutus_spawn` -> `get_best_brutus_spawn_pos(undefined)`
  -> `get_brutus_spawn_pos_val` for both zone_start spots, and no Brutus appears: both values come back not > 0,
  so `brutus_spawn` deletes him ("no brutus spawn_positions", a dev-only print). br2 (same notify, autoplay
  player elsewhere) spawned fine.
- Score: 0 if the zone is off or `get_players_in_zone(zone, 1)` (istouching the zone volumes) is empty; else
  1 + `linear_map(dist2d, 2000, 0, 0, level.brutus_players_in_zone_spawn_point_cap)` for each player
  `findpath` joins (bound on this branch since 46407dfa; br2 may have had no path), plus each valid interaction's
  `spawn_bias`. Suspects: `level.brutus_players_in_zone_spawn_point_cap` or a `spawn_bias` undefined (then the
  sum is undefined and `newval > val` is false), or istouching missing the player at the machine.
