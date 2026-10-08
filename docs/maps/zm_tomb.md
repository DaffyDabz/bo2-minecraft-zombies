# zm_tomb (Origins) in bo2zm

Lane notes for the tomb lane (owner: the tomb lane; journal <work-dir>\lanes\tomb\JOURNAL.md).

## How it starts
- Game mode: Origins registers only `zclassic` with location `tomb` (`zm_tomb_gamemodes::init`:
  `add_map_gamemode("zclassic", zm_tomb::zstandard_preinit)`, `add_map_location_gamemode("zclassic", "tomb",
  zm_tomb_classic::precache, zm_tomb_classic::main)`). `zm_tomb_standard.gsc` exists but is not registered.
- `_zm_utility::is_classic()` reads `ui_zm_gamemodegroup` (hash 0x6b64b9b4), so that dvar must be `zclassic`
  too, not Nuketown's `zsurvival`.
- Run: `bash <work-dir>/lanes/tomb/tools/run.sh NAME SECONDS ["cmds"]` (game slot, own build, log + shots in
  <work-dir>\lanes\tomb\runs\NAME).
- Read the scripts: `T6CENSUS_DUMPALL=<work-dir>/lanes/tomb/gscdump t6census zm_tomb` once, then
  `python <work-dir>/lanes/tomb/tools/t6dec.py <script> [func...]`.

## Generators (zm_tomb_capture_zones.gsc)
- Six generators (`generator_start_bunker`, `_tank_trench`, `_mid_trench`, `_nml_left`, `_nml_right`,
  `_church`) in `level.zone_capture.zones[name]`. Using one (200 per player) starts the capture: 12 s solo
  within 220 units, while 4 capture zombies go for it. When it finishes, the zone gets `player_controlled`.
  That zone's perk machines (`revive_on` etc.), random perks and box then work. Pack-a-Punch needs all six.
- Box: `magic_box_stub_update_prompt` needs the box's zone `player_controlled`, or the hint says "Power must be
  turned on". The start box `bunker_tank_chest` (-528 3832 -352) belongs to `generator_tank_trench`.
- Unitriggers only fire in active zones, so the generators past the doors need the doors open
  (`IW4L_T6_OPENALL=buy`).
- Test aids (sim/src/t6/tomb_test.rs): `IW4L_T6_GEN=<generator>[@secs]` stands beside it and uses it, again
  every 2 s until the capture starts, then logs it every 5 s. Add `IW4L_T6_GEN_BOX=1` to try that zone's box
  20 s later with 10000 points.
- Several generators: `IW4L_T6_GEN=a,b,c@30` takes them one after another, 40 s apart (with
  `IW4L_T6_OPENALL=buy IW4L_T6_GOD=1`); `IW4L_T6_GEN_PAP=1` then stands at Pack-a-Punch
  (`specialty_weapupgrade` trigger at (-6 -8 366)) with 10000 points, uses it and logs `all_zones_captured`.
- Run gen7 (2026-10-07): start bunker, tank trench, mid trench and NML right captured. NML left (farm) and church
  never start: their zones stay off (`level.zones.zone_nml_farm.is_enabled = 0`) because the debris piles
  (`zombie_debris`: farm, ruins, village_0/1) never set their `script_flag`; the doors do.
  Cause (found 2026-10-07, run deb4/deb6): not a game bug. `debris_think` checks `is_player_valid(who)`, which
  needs `isalive`; Origins' intro keeps the player a `spectator` past 5.5 s, so the forced buys at 5.5 s were
  refused (doors don't check). OPENALL=buy now starts once he is `playing` (deb8: farm, ruins, village_1 flags set,
  `debris_move` runs, 1250/2500 taken). A bought pile deletes its sibling triggers, so the buy goes by entity
  number, not by list index.
- Checked 2026-10-07 (lane runs gen1, perk1, box3): the start bunker was captured in 15 s and the capture
  zombies were electrocuted. After that Quick Revive could be bought; without it the hint was "Power must be
  turned on". After the tank trench capture the box gave srm1216_zm_mp for 950. In box5 the capture zombies reached
  the generator, the capture was contested and the player went down (the same as real play, not a bug).

## Perks, dig, robots, Panzer (checked 2026-10-07)
- Perks: `IW4L_T6_GEN_PERKS=1` (after the generators and `IW4L_T6_GEN_PAP`) buys each perk machine 12 s apart
  with 10000 points. Run perk2: Quick Revive (500), Jugger-Nog (2500), Speed Cola (3000), Stamin-Up (2000) bought;
  Mule Kick hint "Mule Kick [Cost: 4000]" but refused, which is right: Origins has a 4-perk limit
  (`vending_trigger_think`: `num_perks >= get_player_perk_purchase_limit` plays the deny sound).
- Dig (zm_tomb_dig.gsc): shovels are `p6_zm_tm_shovel` script models (4) with unitriggers; mounds are
  `p6_zm_tm_dig_mound` (radius-100 trigger, must look at it). `IW4L_T6_DIG=[secs]` (default 30) takes the nearest
  shovel, then digs the nearest mound every 10 s and logs `dig_vars` (has_shovel, n_spots_dug, n_losing_streak),
  points and the pickups that appeared. Run dig1: shovel taken, 4 mounds dug (mounds 4 -> 0); dig 1 gave the
  870mcs (`t6_wpn_shotty_870mcs_world`), digs 2-4 bonus points (`zombie_z_money_icon`). The test does not pick
  them up; since run dig3 it uses the spot again at 5 s, which takes a dug gun (dig3: `870mcs_zm_mp` in
  the player's weapons after dig 2). The gun's hint uses `getweapondisplayname` (bound 2026-10-07: the weapon table's display-name key).
- Giant robots (zm_tomb_giant_robot.gsc): soak4 (AUTOPLAY=2, 1200 s, rounds 1-10, no panics) - the intro robot
  walks at 15-45 s (origin (4584 4850 1237) -> (9329 4850 105)), then one robot walks most of each round; rounds
  4 and 8 are three-robot rounds (`flag.three_robot_round`, all three `is_walking`).
- Panzer (_zm_ai_mechz.gsc): spawned in round 8 (soak4 at 690 s, `num_mechz_spawned` 1, `next_mechz_round` 12)
  and the rounds went on to 10.
- Still unbound in that soak: `spawnvehicle` (the crystal biplane), `setturrettargetvec` (tank), `ismeleeing`,
  `showallparts`, `setlightintensity`, `setplayercollision`, `setforcenocull`; plus ones other lanes have fixed
  (`soundgetplaybacktime`, `ghostindemo`, `getstartorigin/angles`, `setentityanimrate`, `setculldist`).
  Bound since (2026-10-07): `spawnvehicle` (vehicles.rs: a script_vehicle entity with targetname/vehicletype),
  `showallparts`, `ismeleeing` (melee held), and no-ops `setlightintensity`, `setplayercollision`,
  `setforcenocull`. Still open: `setturrettargetvec`, `soundgetplaybacktime`, `ghostindemo`.

## Crafting (_zm_craftables.gsc, checked 2026-10-07)
- Stubs are `level.a_uts_craftables`; each has `.craftablespawn.craftable_name` and `.a_piecespawns`
  (`modelname`, `model`, `in_shared_inventory`, `crafted`). Origins crafts everything at 3 open tables
  (stub name `open_table`) at (146 -2981 108), (2312 729 40) and (-888 2337 -196); the shield's and drone's own
  triggers lie at x -7040, outside the map. Parts are shared pieces (radius-use unitriggers); the table is a
  box-use unitrigger with require_look_at, and holding use there runs `craftable_use_hold_think` (3 s, gives
  `zombie_builder_zm` while it holds).
- Test aid (sim/src/t6/tomb_craft.rs): `IW4L_T6_CRAFT=<craftable>[@secs]` (default 30, e.g.
  `tomb_shield_zm`), 15 steps 10 s apart: stand by a part still lying and press use (4 presses, 1.5 s apart);
  no part left -> stand at the nearest open table and hold use from 2 s to 8.5 s; crafted -> press use there
  to take it. Logs parts, tables, the hint, weapons and the carried part each step.
- Use triggers fire on a fresh press only (triggers.rs `pressed = held && !was`), and the unitrigger manager
  gives the part's or table's trigger a moment after the player arrives: a use held from before the trigger
  existed never fires. Runs craft1/craft2 held use from 1 s: parts only sometimes, and the table hold never
  started. The table's hold also needs `is_player_looking_at(trigger.origin, 0.76)`: standing at the table's
  own x/y puts the eye on the origin, so the test stands 30 along / 14 across from it.
- Run craft4 (`AUTOPLAY=1 OPENALL=buy GOD=1 CRAFT=tomb_shield_zm@30`): the 3 shield parts picked up first try
  (their spots are random each game), at the table (2312 729 40) the hold gave `zombie_builder_zm` and the hint
  turned to "Hold [use] for Zombie Shield"; the next press took it ("Took Zombie Shield",
  `tomb_shield_zm_mp` in the weapons).

## Known gaps
- `getspeedmph` is bound (vehicles.rs: the path segment's speed); the tank itself is not checked yet.
- Window boards (fixed 2026-10-06, zm-tomb): asset_t6 now captures ZBarrierDef (type 0x3B: per board
  `zombieBoardTearStateName@72` / `zombieBoardTearSubStateName@74`, taunts, reach-through, attack slots) and
  sim answers `getzbarrierpieceanimstate` etc. from the window's `type` key (Origins has 2 barrier types).
  Trace: `getzbarrierpieceanimstate(2) = zm_zbarrier_board_tear`, substate `vert_2` ->
  `getanimsubstatefromasd(zm_zbarrier_board_tear_in, spot_2_piece_vert_2) = 14`.
- Board tear needs back-to-back `animscripted` (tear in, loop, out): the `board` notetrack that sets the piece
  "opening" comes from those. `animscripted` now always restarts zm_scripted, and an anim's `end` that starts the
  next scripted anim no longer hands the zombie back to its AI (natives_ai.rs, actors.rs). Clips carry their own
  `end` notetrack (`ai_zombie_boardtear_aligned_m_3_pull`: `destroy_piece` @0.06, `end` @1.0); advance_anim used to
  add a second `end`, which woke the pull's wait at once (pull cut to 0 ms, no `destroy_piece`). Now one `end`.
- The start bunker shows a large green-white glare and a solid white rectangle (an effects/draw issue).
- Round 1 never ended (fixed 2026-10-06, zm-tomb): two causes. (1) Zombies that climbed in through a window stood
  just inside its clip forever: `walk_move` refused every ~1-unit step that still ended inside solid (only a step
  ending outside was allowed), so the scripts' failsafe killed and respawned them, round after round. Now a small
  step is allowed when 16 units on along the same heading is clear (stalls 161 -> 6 in 150 s). (2) The three giant
  robots counted as round enemies: their aitype and spawner say team `neutral`, but `animscripts/zm_init::main`
  sets `self.team = level.zombie_team` and we run it after the aitype's `main`. The aitype's team now wins.
  Origins counts enemies with `zm_tomb_get_round_enemy_array` (`getaispeciesarray(level.zombie_team, "all")`).
- Test switches that helped: `IW4L_T6_EVAL=level.zombie_total,level.a_giant_robots.0.team`, `IW4L_T6_STALL_LOG=1`,
  `IW4L_T6_ACTORS=20000`, `IW4L_T6_ACTOR_TRACE=<ent>`.

## Tank (zm_tomb_tank.gsc, checked 2026-10-07)
- `level.vh_tank` is the map's `script_vehicle` (targetname `tank`, `veh_t6_dlc_mkiv_tank`). `tank_movement` attaches
  it to `tank_start` and parks it at speed 0 by the church (441 -2674 36); `t_use` (`trig_use_tank`, brush *22,
  linked) costs 500, then `setspeedimmediate(8)` drives it along the vehicle nodes until a `tank_stop` node's
  notify. Stops alternate `village` / `bunkers`; after a stop `tank_cooldown` holds about 65 s.
- Test aid (sim/src/t6/tomb_tank.rs): `IW4L_T6_TANK=[secs]` (default 30) gives 2000 points, puts the player on the
  deck volume (`e_roof`, `vol_on_tank_watch`), presses use at `t_use` until `tank_moving`, logs every 2 s.
- Run tank2: paid 500 (2000 -> 1500), the tank drove church -> bunkers in about 70 s (node 206, 172, 205 ...,
  stop at (-332 4648 -268)), the player rode it the whole way (`riders`), cooldown cleared at 136 s and the hint
  came back. Nothing new had to be bound.
- Known: riders hold a vehicle's rider at local z 40 (Tranzit's bus floor, riders.rs `floor_z`), so on the tank
  he rides inside the hull about 100 units under the deck; the deck volume sits at local z ~140. The tank's model
  has no player collision. Not checked yet: zombies climbing on, the flamethrowers, being run over, the trigger
  box after the tank turns (brush volumes only move, they don't turn).
