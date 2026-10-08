# zm_highrise (Die Rise) - highrise lane notes

Map zm_highrise, start location `rooftop`, game type `zclassic` (Die Rise has no survival or grief mode and no
`so_*` mode zone: everything is in zm_highrise.ff + zm_highrise_patch.ff).

## How to run it
- Lane script: `<work-dir>/lanes/highrise/hrrun.sh NAME SECONDS "extra cmds;"` (runs through the game slot, logs
  to <work-dir>/lanes/highrise/logs/NAME.{log,full.log,out}, screenshots to shots/NAME/).
- The `gsc: installed ... gametype=dm` line in the log is the IW4 script layer, not BO2's: the BO2 VM reads
  `g_gametype` from sim/src/t6/mod.rs install.

## Facts measured
- `_zm_gametype::rungametypemain(mode)` returns at once when the map did not register `mode`
  (`level.gamemode_map_location_main[mode]`): started as `zstandard` Die Rise never starts its rounds.
  `_zm_utility::is_classic()` reads `ui_zm_gamemodegroup` (== "zclassic").
- Elevators: `zm_highrise_elevators` keys `floors[...]` by the map field `script_location` and indexes it with
  strings (`floors["1"]`). Typed as a number, every elevator thread died ('undefined is not an object' in
  elevator_think / predict_floor). Kept as text, the elevators run (`zmb_elevator_run` loop starts/stops).
- Jumping Jacks: `_zm_ai_leaper::leaper_calc_anim_offsets` needs getanimfromasd (method), getmovedelta,
  getnotetracktimes (bound in natives_ai.rs, from the anim's delta_trans and notifies).
- Power: `zm_highrise::electric_switch` waits on `use_elec_switch` (trigger_use_touch, brush *8, centre
  (2869 -122 1344)). Touching it shows "Hold [use] to turn on the Power"; use sets `power_on`, the switch sparks,
  every perk elevator starts moving (without power only Quick Revive's runs, solo).
- Perks ride the elevators: triggers are `zombie_vending` (trigger_radius_use, radius 40) with script_noteworthy
  = the perk, linked to the machine. Jugger-Nog bought: 10000 -> 7500 points, perk icon in the HUD.
- Pack-a-Punch rides an elevator too (`zombie_vending/specialty_weapupgrade`): 10000 -> 5000 points, the M1911
  goes in (player on fists), then "Hold [use] for upgraded weapon".
- Test aid: `IW4L_T6_USE="use_elec_switch:20,zombie_vending/specialty_armorvest:35"` (+ `IW4L_T6_USE_FLAG=power_on`)
  puts the player by each use trigger at that time (retried each tick for 15 s while it moves) and presses use.
- Window boards: with tomb's barrier fixes (e893b7d, fd991f7, 0c54c24 taken) zombies tear the boards and come
  in; before them round 1 often stood 40-95 s with one zombie waiting at a window. soak3 (autoplay, 300 s):
  round 5 at 220 s (was round 2 at 145 s).
- Jumping Jacks: `_zm_ai_leaper::leaper_can_use_anim` calls `self localtoworldcoords(local_mid)` on the leaper
  (taken from transit bc360ac, any entity).
- Escape pod: `zm_highrise_classic::escape_pod_linknodes` links the pod door nodes to the 2 nearest path nodes
  each way with `linknodes` (one-way, checked by `nodesarelinked`), unlinks with `unlinknodes`;
  `zm_highrise_utility` deletes one node at x == 3598.2 (`deletepathnode`). Bound in natives_game.rs + nav.rs.
- Jumping Jack rounds (round start+4..6, soak6 round 6): leapers spawn, run (`ai_zombie_leaper_run_bounce`),
  jump down (`traverse/zm_jump_down_190`) and attack (`ai_zombie_leaper_attack_v1/v2`); the round ends and
  round 7 brings normal zombies back. Wall leaps call `self animcustom(::leaper_play_anim)` (bound: the
  function is the actor's animscript until its thread ends, root motion moves it; log line
  `bo2zm t6 animcustom ent<N> starts/done at (...)`). A wall leap not yet seen in a log.
- `IW4L_T6_CENSUS=1` logs every zombie's script and anim each 10 s: the way to see which kind is alive.
- Riding the elevators: a car is a script model (`p6_anim_zm_hr_elevator_common`, freight `..._freight`) made a
  moving platform by `init_elevator` (`setmovingplatformenabled`, transit's riders.rs 03dfba9 taken). Script
  models do not block player movement here, so the car had no floor: the player fell through as soon as it
  moved. sim/src/t6/highrise.rs gives each car a floor slab (top 8 over the origin: the perk machine linked to a
  car stands there) and a roof slab (the model's top) as presence blockers, and riders carries whoever stands
  on one (still carried while airborne in his car: a car going down drops away each tick). ride4: in the
  Jugger-Nog car from 2714 up to 2847 and back down to 2528, on the car the whole run.
- getgroundent works on any entity (zombies, equipment, buildable pieces, grenades): a platform or script
  brush model whose box holds the point (riders::ground_ent), else `level` on the map, undefined in the air.
  Brush model hits come back from traces as the world, so this is how `object_is_on_elevator` sees a car.
- Test: `IW4L_T6_RIDELOG=1` logs platforms, riders on/off (and why), what getgroundent found.
- Path nodes on movers: 69 nodes have spawnflag 0x100 and `target` = a mover: 9 on each elevator car body
  (`elevator_bldg1b_body` .. `elevator_bldg3d_body`; the car's `elevator_bldg<N>_moving` roof nodes are linked
  to the `..._floor<N>` nodes by `elevator_paths_onoff` when it stops at a floor) and 6 in the escape pod
  (`elevator_bldg1a_body`, door nodes `escape_pod_door_l_node` / `_r_node`, linked to the 2 nearest nodes within
  128 by `escape_pod_linknodes` at the top and again after the crash). They move with their entity
  (nav.rs `follow_movers`, kept out of the drop-to-floor at load). Before, they stayed at the map's place: after
  the pod crashed nobody could path to a player in it, `zombie_pathing` fell back to breadcrumbs and then to the
  player's `spectator_respawn` on the roof (1423 1291 3421): round 1 stood 17 min (soak20).
- Escape pod: the trigger `escape_pod_trigger` (trigger_multiple); everyone alive inside it 3 s -> it drops
  (`escape_pod_move` to its target struct, 3 s), players get `elevator_crash` shellshock and go prone. Reset by
  `level notify("reset_escape_pod")` (a trigger elsewhere). The AI picks its player by path length
  (`level.calc_closest_player_using_paths`, `calcpathlength`; Die Rise accepts length 0 only within 36 units).
- `IW4L_T6_NAVDUMP=<file>` writes every path node (index, type, spawnflags, origin, targetname, target, links).
- Test aid `IW4L_T6_USE` takes any trigger now (`escape_pod_trigger:20` stands the player in the pod).
- The script `origin` of a carried node follows it too (`escape_pod_linknodes` searches around `node.origin`;
  with the map's place it relinked the crashed pod's doors to the top floor, pod4: zombies at the bottom stood
  with no path).
- Standing the player in the pod: `IW4L_T6_WALK="1110 1280 3420 0 0.3 20"` (the use aid finds no floor there:
  the pod body is a script model). He rides it down (rider on ent67), `elevator_crash` shellshock at the bottom
  (1176 1280 1456).
- Test aid `IW4L_T6_USE=name:<secs>+<again>`: a second use press <again> s after placing, then 2 s of fire and
  clip counts logged (PaP: take the upgraded gun). The PaP car moves after power: placing can miss it.
- `IW4L_T6_CENSUS_FIELDS=state,no_jump,...`: census shows those script fields per zombie.
- `IW4L_T6_SETLEVEL=next_leaper_round=2` (set at 15 s) brings a Jumping Jack round at round 2. leap1 (240 s): one wall leap ran (animcustom ent697 from (1256 1440 3397) to (1241 1617 3402)); leapers reach state=chasing, no_jump=0, in_player_zone=1.
- soundgetplaybacktime (1000 ms for a known alias, -1 otherwise) and ghostindemo (no-op) bound.

## Status
See <work-dir>/lanes/highrise/JOURNAL.md.
- Escape pod reset: after the crash the scripts set `escape_pod_needs_reset` and wait for `reset_escape_pod`,
  sent when a player inserts the elevator key (buildable `keys_zm`, part `P6_zm_hr_key`, 4 spawn spots, it
  respawns after use) at `escape_pod_key_console_trigger` (1872 1152 1516). key3: pod at z 3384, crashed at
  1448 (25-30 s), key inserted ~64 s ("Hold [use] to insert elevator key"), pod back at 3384 by 80 s.
- Needs istouching on spawned box triggers (zm-buried eb066e3, natives_ent.rs part taken) for unitrigger stubs.
- Test aids: `IW4L_T6_BUILD=keys_zm@60` (buildables aid from zm-buried, in t6/build_test.rs: picks up the part,
  holds use at the stub), `IW4L_T6_ENTLOG=elevator_bldg1a_body` (an entity's origin every 5 s),
  `IW4L_T6_GOD=1` (the player stays alive at the crash site). Soaks: `IW4L_T6_AUTOPLAY=2` (gun + ammo top-up);
  with `=1` the pistol runs dry and the game ends at round 2 every ~140 s (soak21: 9 games, 0 panics).
