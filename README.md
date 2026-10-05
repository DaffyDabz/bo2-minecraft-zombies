# bo2mc: Minecraft Zombies (Black Ops II zombies in a Minecraft world)

A Black Ops II Zombies round every night, Minecraft survival every day. You play the Black Ops II
character with Black Ops II guns, perks, points and HUD, in an endless Minecraft world. Built on
[IW4L](https://github.com/vladtrc/iw4L) (a Rust rewrite of the IW4 engine) by joining two forks of it:
our Black Ops II Zombies rebuild (bo2zm) and chasmlol's
[2010 Rust Rewrite Mashup](https://github.com/chasmlol/2010-rust-rewrite-mashup) (the Minecraft world,
from MinecraftOSS).

**Status:** playable, work in progress (fix list from the first playtest partly done) · **Visibility:** public · **Last updated:** 2026-10-05

Hobby/modding project. Nothing from Activision or Mojang is in this repository: Black Ops II's files
come from your own copy of the game, Minecraft's files are fetched from Mojang's servers on first run.

## Contents
- [How it plays](#how-it-plays)
- [Download and play](#download-and-play)
- [Requirements](#requirements)
- [Install](#install)
- [Run / Play](#run--play)
- [Configuration](#configuration)
- [How it works](#how-it-works)
- [Coming soon](#coming-soon)
- [Recent changes](#recent-changes)
- [Credits and license](#credits-and-license)

## Download and play
1. Download `MinecraftZombies-2026-10-05.zip` from this repository's **Releases** page.
2. Unzip it anywhere (for example Documents).
3. Double-click `Setup.bat`. It finds Black Ops II in your Steam libraries, writes the settings file (`.env`) and puts
   a **Minecraft Zombies** shortcut on your desktop. If it can't find the game it asks for the Black Ops II folder.
4. Open **Minecraft Zombies** > ONLINE > SOLO > pick **MINECRAFT** or **NUKETOWN** on the globe > START MATCH.

You need your own Black Ops II (PC, Steam) with Zombies. The zip holds only this mod (`iw4l.exe`, the loading picture,
the setup script); no Black Ops II or Minecraft files. Tested on the author's PC only, not yet on a fresh PC.

## How it plays
- The game starts at night, in the spawn room, round 1, with Black Ops II's pistol, knife and grenades.
- Every night is one zombies round. The sun stops at midnight until the last zombie of the round dies,
  then the day comes.
- Every day is Minecraft: mine, place blocks, build walls around yourself. No Minecraft monsters.
- The spawn room can't be broken. It holds the perk machines, the Mystery Box (one spot, it never moves)
  and Pack-a-Punch. Its two doors are the only part zombies can break.
- Zombies rise from the ground around you and break any block to reach you, natural ground too. A
  zombie that digs too long, gets stuck or falls behind rises again closer to you.
- Underground when night falls: they rise right on top of you. A warning comes at sunset.
- Going down works like Black Ops II: Quick Revive (solo) brings you back, otherwise game over and the
  same world starts fresh at round 1. Dying in the day ends the game too.
- Carpenter gives you wood (planks and doors).
- Survival items: Black Ops II guns, grenades and the knife are inventory items; you start with wooden tools; hearts and
  hunger replace Black Ops II health (Juggernog adds golden hearts); armor, furnace and chest work; ammo station in the room.
- Black Ops II's own menus: the globe shows MINECRAFT next to NUKETOWN, so both maps are one click apart; Exit Game
  returns to the menus.

## Requirements
- Windows 10/11, a GPU with DirectX 12 (or Vulkan).
- Call of Duty: Black Ops II (PC, Steam) with Zombies installed: the game reads its zone files.
- Internet once, the first time (Minecraft's files, about 125 MB, from Mojang).
- To build: Rust (stable, MSVC), about 30 GB free for the build folder.

## Install
From source (the zip above is the easy way):
1. Build: `cargo build -p launcher --profile play` (set `CARGO_TARGET_DIR` to a roomy drive).
2. Make a play folder and copy `target/play/iw4l.exe` into it.
3. In the play folder, create `.env`:
   ```
   IW4L_GAMES="<your Steam library>/steamapps/common/Call of Duty Black Ops II"
   IW4L_BO2MC=1
   IW4L_SKATE=off
   IW4L_MINECRAFT_SEED=20261005
   WGPU_BACKEND=dx12
   ```

## Run / Play
From the play folder: `iw4l.exe frontend t6:zm_nuked` opens Black Ops II's own main menu (ONLINE > SOLO >
map > Survival > START MATCH). `iw4l.exe map t6:zm_nuked` skips the menu. Use key (F) opens and closes doors;
number keys pick hotbar items (blocks, doors); right click places, left click mines; E opens the inventory.
Settings, caches, logs and demos go to `iw4l-artifacts\` next to `iw4l.exe`.

## Configuration
- `IW4L_MINECRAFT_SEED`: the world (same seed = same world every restart).
- `IW4L_MINECRAFT_TIME`: start time of day (default nightfall, 13000).
- `IW4L_BO2MC_DAY_SPEED`: day clock multiplier (testing).
- Settings (resolution, volume, sensitivity) live in the play folder's `iw4l-artifacts/settings.cfg`.

## How it works
- The game loads Black Ops II's Nuketown Zombies for its guns, zombies, perks, box, HUD and scripts, hides
  Nuketown's world, and stands it on a Minecraft world with block collision (the mashup's trick, with a
  Black Ops II map instead of an MW2 one).
- `crates/sim/src/bo2mc.rs` is the bridge between the Minecraft world (blocks, day clock, room, spawn spots)
  and the Black Ops II rules (zombies, rounds, perks, points).
- Zombies plan over the blocks with A* (walk, climb one block, drop, dig through blocks), claw doors and
  blocks, and rise again closer when stuck or far.
- Rounds follow the day clock: a round waits for nightfall, the clock waits for the round.

## Coming soon
- [ ] All 12 Black Ops II perks in the spawn room (built on branch `mc-perks`, being merged).
- [ ] Check every first-playtest fix on screen: survival mining, hotbar scrolling, hearts/hunger, armor, brighter colours, 6-minute days.
- [ ] Zombies reaching a player on a one-block tower.
- [ ] A fresh-PC install test of the release zip.

## Recent changes
- 2026-10-05: release zip with one-click setup (finds Black Ops II, makes the desktop shortcut).
- 2026-10-05: Black Ops II's front end with MINECRAFT next to NUKETOWN on the globe; new loading screen; Exit Game returns to the menus.
- 2026-10-05: first-playtest fixes: doors you fit through, items for guns/grenades/knife, stacks, ammo station, real ammo, hearts and hunger, armor, furnace and chest.
- 2026-10-05: first playable build: Minecraft world under Black Ops II zombies, spawn room, doors, digging
  zombies, night rounds, Carpenter wood.

## Credits and license
Apache-2.0 (see `LICENSE`), as IW4L and the mashup it is built from.
- **IW4L** by vladtrc and contributors (including Bulat Fatykh): the Rust engine everything runs on, https://github.com/vladtrc/iw4L
- **2010 Rust Rewrite Mashup** by chasmlol: the Minecraft world inside IW4L (block collision, terrain, day cycle), https://github.com/chasmlol/2010-rust-rewrite-mashup
- **MinecraftOSS**: the Minecraft engine crates (world generation from a seed), vendored by the mashup in `third_party/minecraftoss`.
- **Black Ops II Zombies rebuild (bo2zm)** and this mod by DaffyDabz.
- Loading screen art: supplied by the author.
- Call of Duty: Black Ops II belongs to Activision and Treyarch; Minecraft is a trademark of Mojang Studios / Microsoft.
  Fan project, not affiliated with or endorsed by any of them. You need your own copy of Black Ops II; Minecraft's
  data files are downloaded from Mojang's own servers on first run.
