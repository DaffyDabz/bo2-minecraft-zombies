# zm_transit (Tranzit / Green Run; Bus Depot, Town, Farm survival)

Notes of the transit map lane (branch zm-transit). Journal: <work-dir>\lanes\transit\JOURNAL.md.

## How to run it
- `<work-dir>/lanes/transit/tools/trun.sh NAME SECONDS "console cmds"` runs `iw4l.exe map t6:zm_transit` from
  <work-dir>/target-transit/play through the game slot; logs and shots go to <work-dir>/lanes/transit/runs/.
  `IW4L_T6_AUTOPLAY=1` plays it (5 s status lines: round, points, zombies alive).
- Script reading: the map's compiled scripts are dumped in <work-dir>/lanes/transit/gscdump (t6census
  `T6CENSUS_DUMPALL`); `python <work-dir>/lanes/transit/tools/t6dec.py <dump> <function>` lists a function.
- Map entities: `T6CLIP_ENTS=<out.txt> t6clip.exe <zone.ff>` writes a zone's entity string.

## What the map needs (measured)
- Game type: since 10-06 (zm-buried/zm-tomb's zclassic start, taken in c3adb47) Solo starts Tranzit as
  `zclassic` + `transit` = Green Run (`zm_transit_classic`) and loads `so_zclassic_zm_transit.ff`: zombies
  spawn, rounds advance. Bus Depot survival is `zstandard` + `transit`; Town and Farm `zstandard` +
  `town` / `farm` (not reachable from the menu yet).
- Zombie spawners are NOT in zm_transit.ff: its entity string has no `actor_*` at all (Nuketown's has 5).
  They come with the mode zone `so_<group>_zm_transit.ff` (group: zsurvival for zstandard, zclassic,
  zencounter for grief): so_zsurvival holds 1 addonmapents, 32 scripts, 138 xmodels, 490 xanims.
  Without that zone `_zm::round_spawning` dies on `random(level.zombie_spawners)` (empty list).
- Streaming areas: zm_transit_gump_{busstation,diner,farm,town,powerstation,labs,tunnel,forest,forest2,
  cornfield,bridge}.ff; not loaded yet.
- `delete_bus_pieces` (survival) hides the bus's zbarrier windows via `getzbarrierarray()` (added in zm-transit).
- Script models missing from the catalog: p6_zm_core_reactor_top, p6_door_metal_no_decal_left (likely in
  a gump or mode zone).

## The bus (zm_transit_bus.gsc)
- `the_bus` is a `script_vehicle` (model veh_t6_civ_bus_zombie). `busopeningscene` drives it along
  `BUS_OPENING` (12 nodes, 3,180 units) to the depot stop, then `follow_path(BUS_START)`: a LOOP of 278
  `info_vehicle_node`s (80,140 units) back to BUS_START. Stops (script_noteworthy): depot, diner, farm,
  power, town; ambush points: tunnel, forest, cornfield, power2town, bridge (`tools/vpath.py <node>`
  lists a chain from ents.txt).
- The script drives it: `setspeed(mph, accel, decel)` / `setspeedimmediate`, polls `getspeed()`,
  waits `reached_node` (node) per node and re-sends the node's script_noteworthy as `noteworthy`;
  `busschedulethink` stops at a destination only when a player is in the nearby zones, else drives on.
  It leaves the depot only once door flag OnPriDoorYar or OnPriDoorYar2 is set (a depot door bought),
  then waits randomintrange(40,180) + 10 s.
- Engine side: crates/sim/src/t6/vehicles.rs (speed easing, looping chains, `reached_node`,
  `localtoworldcoords`/`worldtolocalcoords`, `setvehmaxspeed`). Test env: `IW4L_T6_VEHLOG=1` logs each
  node passed; `IW4L_T6_CHASECAM="back up"` holds player 0 behind the bus looking at it.
- Riding (sim/src/t6/riders.rs): `setmovingplatformenabled(1)` marks a platform (the bus, its clips
  and triggers). Each tick, after vehicles and links move, a player standing in a platform's box (as
  it stood last tick) is moved and turned with the root of its links; `getmoverent()` returns that
  root (busupdateplayers counts riders by it); `isonladder` reads the ladder pm flag. The bus model
  has no player collision of its own here (script models are solid only for actors, collision_* and
  the box): since 10-07 crates/sim/src/t6/transit.rs gives it 22 turned boxes (presence blockers,
  also in his client's prediction) from the model's collision surfaces (`T6PROPS_MODEL=veh_t6_civ_bus
  t6props.exe zm_transit.ff`): floor top z 40, walls' inner faces y +-69, back x -89, front x 375,
  roof z 144..152 with the hatch hole x 196..255 y +-29, door wells (steps 16/30) and gaps at x 33..85
  and 309..361 on the right (-y), which the door brushes *221/*219 (`bus_door_blocker`, solid while
  closed) fill. The doorway leaves him 22 units: his middle must be at x 48..70 / 324..346.
  A player between its walls is also held at its floor, bus-local z 40 (`IW4L_T6_BUSFLOOR`). Boxes (bus-local): bus (-96 -91 0)..(449 88 172); door blockers *219/*221
  (-26 -6 -59)..(25 6 59); plow *228 (-29 -65 -38)..(29 65 38); hatch *230 (-28 -28 -3)..(27 28 2).
  Test: `IW4L_T6_GOD=1 IW4L_T6_RIDE=1 IW4L_T6_RIDELOG=1` puts player 0 in the bus at 15 s
  (`IW4L_T6_RIDE="x y z"` = bus-local spot, default 0 0 80) and logs platforms and riders.
  `IW4L_T6_RIDE_YAW=<deg>` faces him that way from the bus's front, `IW4L_T6_RIDE_AT=<s>` places
  him then; RIDELOG logs his bus-local place every 0.5 s. `IW4L_T6_USE=bus_door_trigger@<s>
  IW4L_T6_USE_STAY=1` presses use where he stands (the bus doors toggle). The bus's opening drive
  ends at 23..30 s (varies run to run); he cannot move until it ends.
  Still to do: the roof (ladder at the back, the hatch); client prediction does not carry riders (20 Hz steps on
  screen); `nodesarelinked`, `getanimlength`, `hidepart`, `ghostindemo`.
- The bus IS drawn (veh_t6_civ_bus_zombie is in zm_transit.ff and in the script-model catalog, all its
  surfaces draw). Tranzit's fog hides it from ~700 units: test it with `IW4L_T6_CHASECAM="400 120"`.
  Its lights and cow catcher models live in so_zclassic_zm_transit.ff.
- Prison's getgroundent (cherry-picked) maps the IW4 ground entity number straight onto script entity
  numbers; that is wrong for movers shown through presences (bus): needs the presence number -> ent map.
- Test aids: `IW4L_T6_PRESLOG=<model part>` logs each script model as it is first shown (hidden,
  in the catalog); the render logs once per model any surfaces it leaves out
  (`t6 model <name>: n of m surfaces drawn`); `T6CENSUS_XMODELS=<part>` lists a zone's models.
- Window shots need tools/keepup.ps1 (prison lane's): a minimized test window shoots 1x1 black.

## Box, perks, buildables (2026-10-07)
- 12 buildable tables (`level.buildable_stubs`): turbine, busladder, bushatch, cattlecatcher at the depot;
  riotshield_zm, dinerhatch at the diner; jetgun_zm, pap, sq_common in town/the labs; turret (farm),
  electric_trap and powerswitch at the power station.
- Pickups and tables are unitriggers: `_zm_unitrigger::main` only offers the stubs of ACTIVE zones
  (`level.active_zone_names`), so a test that teleports far must enable the zones first. The autoplay
  `open_all_zones` now also calls `_zm_zonemgr::enable_zone` on every key of `level.zones` (50 zones).
- Measured: turbine built from its 3 parts and taken (equip_turbine_zm_mp), r17; Quick Revive bought at
  the depot (vending_revive, 500), r19; the box (first spot: the diner, -4838 -7751) sold an M1216 for
  950, r20. Where many stubs are near (power station) the table's prompt can take seconds to reach him.
- Tests: `IW4L_T6_BUILD=<equipname>@<s>` (buried's), `IW4L_T6_BOX=1`, `IW4L_T6_PERK=vending_revive`,
  with `IW4L_T6_GOD=1`. Plain autoplay (pistol only) reaches round 2 and dies there (ammo runs out).
- Power switch (power station): with `IW4L_T6_OPENALL=1 IW4L_T6_BUILD=powerswitch@20` all 3 parts (lever,
  body, hand) go in and the table leaves `level.buildable_stubs` (built), r23. Not yet checked: flipping it
  (power_on), perks/PaP with power. The build test now presses only once the prompt shows (pickups and the
  4.3 s table hold), since the unitrigger can reach him seconds late.

## Power, Juggernog, round 1 stall (2026-10-07)
- Power: after the switch is built (`powerswitch_buildable_trigger_power`, zm_transit_power::electricswitch)
  a use flips it; the reactor rises for 30 s, then `power_on` is set. Measured (p1): `IW4L_T6_OPENALL=1
  IW4L_T6_GOD=1 IW4L_T6_BUILD=powerswitch@20 IW4L_T6_USE=powerswitch_buildable_trigger_power@110` ->
  power_on false at +3 s, true at +35 s, hint "Hold [use] to turn off the Power".
- Juggernog (`vending_jugg`, town 1046 -1560) sold after power: `IW4L_T6_PERK=vending_jugg@160`, 10000 -> 7500,
  perks ["specialty_armorvest"].
- Test aids: `IW4L_T6_BUILD` takes a comma list (`powerswitch@20,pap@200`); `IW4L_T6_PERK=<m>@<s>`;
  `IW4L_T6_PAP=@<s>` (machine already there, no Nuketown bring_perk); use test logs its flag again at +35 s;
  `IW4L_T6_AUTOPLAY_SEEK=1` walks to the nearest zombie when none is in shooting range.
- Round 1 never ends under autoplay (soak1, w1, w2): 2-3 actors stay alive from ~100 s. w2 at 75 s:
  Avogadro idle in his chamber at the power station (12208 7582 -768, zm_chamber_idle) and a zombie
  standing idle in the depot (-6398 5118 -56, zm_idle, path 0/0, goal 100 units away). zm_transit_classic
  spawns 4 sleeping (inert) zombies in the depot at round start (structs `inert_location`, zone_pri);
  they count for the round (get_round_enemy_array) and wake only on touch (64), sprint within 600 or a
  shot within 2400 (player.lastfiretime). Not yet known: whether Avogadro is skipped by
  ignore_enemy_count in the real game, and why the depot one stands without a path.
- Console `wait Ns` overran in this slice (a 150 s run still going at 455 s game time); use TLIMIT.

## Round 1 stall solved, 20 min soak, Pack-a-Punch (2026-10-07)
- The round 1 stall was one depot sleeper (inert zombie) left asleep: it counts for the round and wakes only
  on touch (64), a sprint within 600 or a shot within 2400 (`player.lastfiretime`, set on `weapon_fired`,
  works). Avogadro is not the cause (`ignore_enemy_count` 1). The autoplay player had nothing in sight, so
  never fired, and its straight walk never reached the sleeper. A real player shoots or walks up.
  Autoplay with `IW4L_T6_AUTOPLAY_SEEK=1` now fires a shot every 3 s when none is in sight and one is
  within 2400.
- Far sleepers are recycled by the map itself (zm_transit_distance_tracking: >1500 away and unseen ->
  deleted, `zombie_total++`).
- soak2 (`IW4L_T6_AUTOPLAY=2 IW4L_T6_AUTOPLAY_SEEK=1 IW4L_T6_GOD=1`, 1260 s): round 12 at 1200 s,
  36610 points, 0 panics, 0 script errors; only `ghostindemo` unbound (highrise 09d211f has it, but that
  commit needs highrise's own files, cherry-pick conflicts).
- Pack-a-Punch (pap1): `IW4L_T6_OPENALL=1 IW4L_T6_GOD=1 IW4L_T6_BUILD=powerswitch@20,pap@200
  IW4L_T6_USE=powerswitch_buildable_trigger_power@110 IW4L_T6_PAP=@340`: table (2252 -206 -248), parts
  table, body (1262 535 -304), battery; built; then m1911 -> m1911_upgraded for 5000 points.
  `calcweaponoptions` now returns 0.
- Debug aids: `IW4L_T6_ACTOR_FIELDS=is_inert,ignore_enemy_count,...` (each actor's script fields with
  `IW4L_T6_ACTORS`), `IW4L_T6_EVAL` takes `player.<field>`; the autoplay status line shows his place
  and the nearest zombie's distance.
