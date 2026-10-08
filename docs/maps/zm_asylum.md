# Verrückt (`zm_asylum`, Zombies Declassified)

Owned by the declassified lane (branch `zm-declassified`). Files, load
order and the mod's fix scripts: see `zm_prototype.md` (same pack, same
loader). Run: `dzrun.sh NAME SECS "cmds" zm_asylum`.

## What works (2026-10-07)

- Loads, 9 script zones; rounds 1 -> 4 in 5 min with the test player
  (one round driver: `zzz_zm_roundfix` covers it), points, kills.
- Windows: 3 wall windows name `zmcore_prototype_wallbarrier`, a type
  this map's zone does not hold (it has `zmcore_asylum_wallbarrier`).
  `zbarrier::def_for` now takes the zone's one type of the same kind
  (last name part), so zombies get their tear anims there.
- Doors: all 13 buy triggers bought (`IW4L_T6_OPENALL=buy`) enable all 9
  zones (west/west2/north downstairs, north/north2/south/south2/kitchen/
  power upstairs).
- Power: switch trigger `use_master_switch` at (-302 -225 275), hint
  "Hold [E] to turn on the Power"; using it sets flag `power_on`.
- Perks after power: Jugger-Nog (`vending_jugg`, 2500) and Speed Cola
  (`vending_sleight`, 3000) bought, icons on the HUD. Also in the zone:
  revive, doubletap, three_gun (Mule Kick), tombstone.
- Mystery box at (-605 -203 226): opens for 950 and offers a weapon.

- Soak: 20 min with the test player (`asysoak`), round 10, 30070 points,
  0 panics, clean exit.
- Electric traps (map's own `electric_trap_think`): buy triggers
  `gas_access` (south (240 -394 274), north (298 470 280)), 1000 after
  power; damage brushes `trigger_multiple` pf218_auto242 / 246 with
  spawnflags 1 (AI axis). Cycle: "The trap is active." ~25 s, "The trap is
  recharging.", then "Hold [E] to activate the trap [Cost: 1000]". Buying
  one sets both switches active (the north use after the south one costs
  nothing). Touch triggers now fire for zombies when their spawnflags take
  AI (`triggers::actor_touches`), so `zombie_elec_death` runs on them.
  A player inside without Jugger-Nog is killed at once (the map's
  `player_elec_damage`); with `IW4L_T6_GOD` he is shocked every frame.

## Test aids

- `IW4L_T6_DZUSE=<trigger targetname>[,...]` (`declassified.rs`): from
  `IW4L_T6_DZUSE_AT` ms (15000; the player spawns late here), 5 s apart,
  the player stands in that use trigger, presses use, and his place,
  points, perks, hint and `power_on` are logged. Combine with
  `IW4L_T6_PERK=vending_jugg` (perk test at 42 s) and `IW4L_T6_OPENALL=buy`.
  `name#k` takes the k-th trigger of that name (`gas_access#1` = north).
  Trap test: OPENALL=buy (zombies can't reach the upstairs switch through
  closed doors), START_ROUND=3, DZUSE_AT=75000, FTRACE=zombie_elec_death.

## Not done yet

- The map is very dark in our shots (the mod's `zzz_zm_visioncycle` has a
  brightness ladder; check against a reference before changing anything).
- Door sounds (`zzz_zm_asylumdoor`: door bodies get `zmb_small_wood_door`),
  Double Tap / Quick Revive / Mule Kick buys, box weapon take, a trap kill
  seen on screen (zombie death fx), many zombies through a trap.
