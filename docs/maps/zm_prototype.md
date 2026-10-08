# Nacht der Untoten (`zm_prototype`, Zombies Declassified)

Owned by the declassified lane (branch `zm-declassified`). The map comes from
Zombies Declassified BETA1, a port of Black Ops II's cancelled DLC5 (the Black
Ops 1 maps rebuilt in BO2's own zone format). Its files stay in their own
read-only folder, never copied into the BO2 install.

## Where the files are (read-only)

`<work-dir>\declassified\ZombiesDeclassified_BETA1`:

- `storage/t6/zone/zm_<map>.ff`, `en_zm_<map>.ff` (also copied under
  `storage/t6/mods/dlc5/zone/`)
- `steam/zone/all/zm_<map>.ipak`, `mod_load.ipak` (image packs)
- `steam/sound/zmb_waw_<map>.*.sab*`, `zmb_blops_<map>.*`, `dlc5_load.all.sab*`
- `storage/t6/mods/dlc5/{mod,mod_load,mod_patch}.ff` (the mod's weapons, menus)
- `storage/t6/raw/scripts/zm/*.gsc` (mod add-on scripts as SOURCE, not
  compiled; the author's Plutonium compiled them at load)
- `storage/t6/mods/dlc5/console_zm.log*`: the author's own game logs. Load
  order there: the usual `*_zm` zones, `dlc*_load_zm`, then `mod`,
  `mod_load`, `mod_patch`, `zm_<map>`, `en_zm_<map>`. Gametype `zclassic`,
  `ui_zm_mapstartlocation ""`.

## How we read them

`IW4L_T6_EXTRA=<work-dir>/declassified/ZombiesDeclassified_BETA1` (`;`-separated
list): `asset_transport::t6_extra`. Zone lookup, image packs and sound banks
search the install first, then these folders. A map zone found there still
loads the install's common zones (`install_zone_dir`).

The map zones carry a `shaderOverflowBuffer` memory block (streamMem, 835,488
bytes) in the streamer-reserve block. That block is reserved, not read from
the file: the next asset follows the block's name at once. `fastfile_t6`
now types it Runtime; all 318 zones (stock BO2 + Declassified) still walk
to the exact end (`t6zone --walk`, 2026-10-06).

Load order (`assets::lane::t6_m2::zone_paths`): the install's
`code_post_gfx_zm`, `common_zm`, `patch_zm`, `patch_ui_zm`, then for a map
zone outside the install the mod's `mod`, `mod_load`, `mod_patch` (found
in the extra folder), then the map. `mod_patch` carries 134 weapons (the
BO1 guns), `mod_load` the string tables (`zm/mapstable.csv`...) and the
English strings. There is no `so_zclassic_<map>.ff`: the map zone holds its
own aitype and characters. Game type `zclassic`, location `""`.

`asset_transport::game_main_for_zone`: a zone with no `main/` above it (the
extra folder) uses the install's game tree (iwds, sounds).

Test run: `bash <work-dir>/lanes/declassified/dzrun.sh NAME SECONDS "cmds" [MAP]`
(env passes through: `IW4L_T6_AUTOPLAY=2` for the rounds test player,
`IW4L_WINDOW_HIDDEN=1 IW4L_INPUT_UNFOCUSED=1` so screenshots don't come
out 1x1 when the desktop minimizes the window). `survey.sh SECONDS maps...`
runs several maps and writes `runs/survey.txt`. dzrun.sh runs share one
build folder's logs: run them one after another, never two at once.

## The mod's fixes (GSC source we cannot compile)

`storage/t6/raw/scripts/zm/zzz_*.gsc` (57 files) are the pack authors' fixes,
compiled by Plutonium at load. We have no GSC source compiler, so the ones a
map needs are redone in Rust in `crates/sim/src/t6/declassified.rs`, each
named after its file:

- `zzz_zm_roundfix`: the leaked maps set `level._round_start_func`, so two
  `round_think` drivers ran and rounds went 1, 3, 5. Done: the field is
  set to a no-op (`declassified_round_start_noop`, logged once as a missing
  function) as soon as the map sets it. All leaked maps but zm_theater.
- `zzz_zm_location`: Nacht's start location is `default` (its Mule Kick
  struct is `zclassic_perks_default`). Done in `sim::t6::location`.
- `zzz_cabinet`: the sniper cabinet's trigger (`weapon_cabinet_use`, ent78,
  brush *10 at 584 882 201) has no handler in any zone. Done in Rust:
  hint ZOMBIE_CABINET_OPEN_1500; 1500 opens the doors (ent76 +120, ent77
  "right" -120 yaw, not solid), creak `zmb_small_wood_door`, gives
  dsr50_zm through `_zm_weapons::weapon_give`, points through
  `_zm_score::minus_to_player_score`; then sells the gun again (1500) or
  ammo (750: half, as a wall buy; the mod's `get_ammo_cost` says 30 for
  the DSR, a box gun). The hook is one line in `triggers::dispatch`.
  Test aid `IW4L_T6_DZCABINET=1` (stand at it, use at 7 s and 9 s, logs).
- `zzz_protonames`: SMR / Executioner wall-buy hints. The mod's strings
  (`ZM_PROTO_WB_SMR`, `ZM_PROTO_WB_EXEC`) live in mod_load.ff; the lane
  loader now reads strings from every zone where no language zone has
  the key, and the retail keys ZOMBIE_WEAPON_SARITCH / JUDGE are given the
  mod's text on Nacht and Der Riese.
- `zzz_zm_doorcreak`: the first-room door body gets script_sound
  `zmb_small_wood_door` (retail's `zmb_door_slide_open` is only in
  Nuketown's bank, so the door opened silently). Code in
  `cabinet_setup`; built-checked, not yet heard in the game.
- Not done yet: `zzz_protoround` (round 1 waits for the intro black
  screen), `zzz_zm_barrelsound`, `zzz_zm_monkeymodel`
  (monkey bombs invisible).

Nacht's zones: `start_zone` (2 use triggers), `upstairs_zone` (4) and
`box_zone` (3, the box) behind the two debris piles. The box only shows
its hint once its debris is bought (as in the real game).
Test aid: `IW4L_T6_DZZONES=<s>` logs the zones and their use triggers.

## State

See the lane journal `<work-dir>\lanes\declassified\JOURNAL.md`.
