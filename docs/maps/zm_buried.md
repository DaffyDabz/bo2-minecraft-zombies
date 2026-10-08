# Buried (zm_buried)

Black Ops II's fourth Zombies DLC map (Vengeance). The full map plays in the
classic mode, `zclassic`, at start location `processing`; Grief and Turned
use the `street` location in the `zencounter` mode zone.

## Zones

From `zone/all`, after `code_post_gfx_zm`, `common_zm`, `patch_zm`,
`patch_ui_zm`:

- `zm_buried.ff` (53 MB): the world, its entities, the map scripts.
- `so_zclassic_zm_buried.ff` (19.5 MB): the mode zone. The zombie aitypes
  (`zm_buried_basic_01_char_*`, `_02_char_*`, `_03`), the ghosts
  (`zm_buried_ghost_female`), Leroy (`zm_buried_sloth`), their characters,
  animtrees and animstatedefs (`zm_buried_basic.asd`, `zm_buried_ghost.asd`,
  `zm_buried_sloth.asd`), the player characters, and the mode's extra map
  entities (`addonmapents` `maps/mp/so_zclassic_zm_buried.mapents`).
- `so_zencounter_zm_buried.ff`: Grief / Turned (not loaded).
- `zm_buried_patch.ff`: the patch, last.

Without the mode zone the map loads with one animstatedef and no zombie
types (measured on the first map check).

## How it starts

Solo starts it as `g_gametype zclassic`, `ui_zm_mapstartlocation
processing` (`sim::zm_gametype`, `location` in `sim/src/t6/mod.rs`).

## Power, doors, box, perks

- Power: the lever `use_elec_switch` (trigger at 645 -409 178) runs
  `zm_buried_power::electric_switch`, which sets `power_on` and unpauses the
  perks. Before it every machine says "You must turn on the Power first!".
- Doors: 13 buyable doors and debris (`zombie_door` triggers, flags such as
  `general_store_door1`, `bar_door1`, `gunshop2tunnel`). Processing, the
  tunnels and the streets are joined `always_on`.
- Test aids: `IW4L_T6_USE=<trigger targetname>[@secs]` (stand in it and press
  use; logs a level flag), `IW4L_T6_DOOR=<flag>`, `IW4L_T6_BOX=1`,
  `IW4L_T6_PERK=vending_sleight`.

## Buildables

- Tables are `level.buildable_stubs` (unitrigger use boxes, 32x100x64, must be
  looked at). Turbine, springpad, subwoofer and headchopper are pooled: which
  table builds which shuffles every game (`randomize_pooled_buildables`).
- Pieces are radius unitriggers; picking one up while holding another drops the
  held one there. Building holds use for 3 s with `zombie_builder_zm`.
- Test aid: `IW4L_T6_BUILD=<equipname>[@secs]` (e.g. `turbine`): every 12 s
  teleports onto the next piece, takes it, stands at the table and holds use;
  logs the stubs, parts, hints and weapons.
- The pooled tables shuffle at start (`swap_buildable_fields`, needs
  `worldtolocalcoords`/`localtoworldcoords`, taken from transit bc360ac): the
  same buildable lands on a different bench each game.
- The use box is 100 long (table's forward) but 32 deep (its right): stand in
  front of the table, within ~30 units, or `istouching` fails and the hold
  stops at once.
- Table models show their parts as they are added (`hidepart`/`showpart` ->
  the model's hide bits in `presence.rs`).
- 2026-10-07: turbine builds 3/3 and is taken. Trample Steam (springpad_zm)
  builds 4/4 on three different benches and is taken; the table shows each
  part as it goes on.
- Leroy's barricades: the church, jail, gun store, mansion lawn, darkwest
  nook, `jail_jugg` and `sloth_blocker_towneast` are `sloth_barricade`
  triggers that only break for Leroy (berserk). Their zones stay disabled,
  so a bench behind one (the church) has no use trigger: the unitrigger
  manager only offers stubs in active zones. `IW4L_T6_OPENALL=buy` breaks
  them too (flag set, pieces hidden by `hide_sloth_barrier`).
- 2026-10-07: head chopper builds 3/4 at the courthouse bench; its blade
  lies on a shelf (z 68) and its radius trigger runs from z 80 up, so the
  player on the floor (z 8) never touches it.

## Pack-a-Punch

- Buried's machine is a plain trigger (`specialty_weapupgrade`), not a buildable
  here (`zm_buried_buildables` has no `pap`). It waits for the power lever.
- Test: `IW4L_T6_GOD=1 IW4L_T6_USE=use_elec_switch@20 IW4L_T6_PAP=1` (power at 20 s;
  at 90 s 10000 points and use at the machine, at 100 s take the gun). The 25 s
  `bring_perk` call is Nuketown's and does nothing here. Without GOD the player
  dies standing still and the scripts restart the level.

## Ghosts (mansion)

- `_zm_ai_ghost`: a ghost round starts when a valid player is in `zone_mansion`
  (`get_current_zone`) and not touching the `ghost_round_override` trigger
  (one, at 2593 562 290, the front of the house). Ghost rooms: `ghost_zone`
  volumes (`level.ghost_rooms`), e.g. `ghost_to_maze_zone_1` at 2549 399 239.
- Test: `IW4L_T6_TP="2760 800 275 200@10"` puts the player inside (the console
  `tp` refuses in zombies). `IW4L_T6_EVAL=level.players.0.is_in_ghost_zone,
  level.zombie_ghost_round_states.is_started,level.zombie_ghost_count`.
- At 2549 399 (lands z 174) the mansion zone is occupied but `is_in_ghost_zone`
  stays 0 (likely the override trigger).
- The hang at 2760 800 275 was `geteyeapprox` unbound (sight checks got
  undefined). Bound (same as `geteye`), the ghosts spawn and play.
- `ghost_zone_spawning_think` spawns only at a spawn point of the player's
  current room that the player can NOT see (`sighttracepassed` from his eye)
  and no player is within 84. `ghost_to_maze_zone_1` has one spawn point
  (2798 839 268), so standing at 2760 800 nothing spawns.
  `ghost_to_maze_zone_2`: volume (2916 581 226), spawns (2949 1009 258),
  (2965 1215 258); zone_3 (3394 1126), (3224 1212); zone_5 (3347 840),
  (3115 978). Rooms: to_maze 1, 2, 3, 4a, 5, from_maze 1-5, start_area_drop_down.
- Test: `IW4L_T6_TP="2916 581 240 90@10"`: 2 ghosts spawn (limit 4, 1 per
  player), fly to the player and drain points (500 to 0, `zm_drain`). With
  `IW4L_T6_AUTOPLAY=2` they are shot: `ghost_death_func`, and each kill gives a
  lethal grenade back (`give_player_rewards`; no points). Shots: lane
  shots/gh11.
- Ghosts draw solid; the real ones are see-through with a glow (client fx via
  the `ghost_fx` clientfield; not drawn here).

## Leroy (sloth)
- `_zm_ai_sloth::sloth_spawning_logic` spawns him from `sloth_zombie_spawner` into
  `jail_idle` (cell under the sheriff's office, about (-1125 791 8)).
- The cell key is the `keys_zm` buildable: key piece in the jail, "table" is the cell
  door (`cell_door_trigger`, "Hold to unlock"). Unlocking sets `level.cell_open`; he
  goes to `jail_cower` and his gift trigger (the `sloth` stub) registers.
- Gifts are the `sloth` buildable's pieces: candy (candy store or toy store, it moves)
  and booze (spawns only after `jail_barricade_down`). Player and Leroy must face each
  other (dot 0.75). Candy: `eat`, then `context`, then `roam` (he walks the town).
  Booze: `berserk` (breaks the barricade you face).
- Test: `IW4L_T6_BUILD=keys_zm@15,sloth@75` (the `sloth` entry stands 48 in front of
  him). `sloth:booze@N` hands him a part from the `booze` buildable's stubs
  (`table:source`). `IW4L_T6_EVAL=level._sloth_ai.state` logs his state every 5 s.
- Booze is a managed piece (`manage_multiple_pieces(2)`), 4 spots: (946 -1509 96),
  (585 -1574 90), (787 -1589 88) and (-1016 854 45) by the jail. Candy is managed too
  (max 1).
- Open: the booze never shows. `wait_start_candy_booze(zone.pieces[1])` hides
  pieces[1] until `jail_barricade_down`; with our newest-first array order pieces is
  [candy, booze], so booze waits for a barricade only berserk (booze) can break. The
  script also has booze-before-barricade paths (`onpickup_booze` keeps
  `booze_start_origin`, `wait_respawn_booze_at_start`), which only make sense if
  pieces[1] is candy. Settle the order before touching the VM (Nuketown relies on it).
- Open: `sloth_prespawn` runs 4 times for one Leroy (`level.possible_slowgun_targets`
  has 4 entries, 4-5 `sloth` stubs at (-1123 725 16)). The engine does not run
  `spawn_funcs` itself (actors.rs only copies the spawner's fields); look at how often
  `spawn_zombie`/`add_spawn_function` run on the 2 spawners.
- After the candy, `no_hands_zm` stays in the weapon list: the real
  `_zm_buildables` only takes `zombie_builder_zm`, so that is the script, not us.

## State

See the lane journal (`<work-dir>\lanes\buried\JOURNAL.md`).
