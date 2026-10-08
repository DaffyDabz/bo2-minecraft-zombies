# bo2mc: Minecraft Zombies (Black Ops II zombies in a Minecraft world)

A Black Ops II Zombies round every night, Minecraft survival every day. You play the Black Ops II
character with Black Ops II guns, perks, points and HUD, in an endless Minecraft world. Built on
[IW4L](https://github.com/vladtrc/iw4L) (a Rust rewrite of the IW4 engine) by joining two forks of it:
our Black Ops II Zombies rebuild (bo2zm) and chasmlol's
[2010 Rust Rewrite Mashup](https://github.com/chasmlol/2010-rust-rewrite-mashup) (the Minecraft world,
from MinecraftOSS).

**Status:** playable, work in progress (fix list from the first playtest partly done) · **Visibility:** private for now · **Last updated:** 2026-10-05

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
- The game starts at night, in the spawn room, round 1, with Black Ops II's pistol and knife (no free grenades: buy them at the chalk).
- Every night is one zombies round. The sun stops at midnight until the last zombie of the round dies,
  then the day comes.
- Every day is Minecraft: mine, farm, place blocks, build walls around yourself. Only animals by day.
- At night Minecraft's monsters spawn too and hunt you like the zombies do: they know where you are within
  48 blocks, seen or not. Daylight burns them.
- The spawn room can't be broken. It holds the perk machines and the Mystery Box (one spot, it never moves).
  Zombies can't break its doors: they come in through two glass windows, smashing the panes one by one.
  Hold Use at a window to put the glass back (+10 points a pane), like Black Ops II's boards.
- Wall weapons are chalk drawings, each with a Minecraft sign above it giving the name and price: four inside
  the house, eight on its outside end walls (guns, the Bowie Knife, Galvaknuckles, Semtex, frag grenades, claymores).
- The vault: a second room across the road, 3,000 points to open. It holds the Nether portal, the End portal
  (twelve Eyes of Ender light it), Pack-a-Punch, and the rare wall weapons, hung on the wall itself until they
  get chalk outlines: the Blundergat (10,000), the Spork (5,000, three times the knife) and the Golden Spork
  (15,000, kills in one hit). Signs mark what comes next (Hell's Retriever, the Thunder Gun, more Mystery Boxes).
- Every 5th round is a souls round, like Nacht der Untoten's dog rounds: no zombies, just hellhounds that arrive on
  Minecraft lightning bolts and a pack of angry Minecraft wolves, in thick fog; the last one drops Max Ammo and the fog lifts.
- Swimming works like Minecraft: slower in water, you sink slowly, Jump swims up, Crouch dives; Jump and Forward at
  the surface climbs out. Stay under too long and you drown.
- Nether and End: zombie rounds keep going there; Nether zombies drop blaze rods, Eyes of Ender lead to the End.
- Zombies rise from the ground around you and break any block to reach you, natural ground too. Blocks
  have health by their Minecraft hardness: leaves and dirt go fast, wood and stone slower, iron and obsidian
  hold for a long time, bedrock never breaks. Zombies go through the weakest way in. A
  zombie that digs too long, gets stuck or falls behind rises again closer to you.
- Underground when night falls: they rise right on top of you. A warning comes at sunset.
- Going down works like Black Ops II: Quick Revive (solo) brings you back, otherwise game over and a
  new random world starts at round 1. Every game is a new world. Dying in the day ends the game too.
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
- [ ] Chalk outlines for the Blundergat and both Sporks (they hang on the wall for now).
- [ ] Hell's Retriever (the Mob of the Dead tomahawk) in the vault.
- [ ] The Thunder Gun (Black Ops 1) in the vault.
- [ ] Mob of the Dead and Origins Mystery Boxes in the vault.
- [ ] Check on screen in a full playthrough: windows, souls rounds, swimming, the Nether and the End.
- [ ] Check every first-playtest fix on screen: survival mining, hotbar scrolling, hearts/hunger, armor, brighter colours, 6-minute days.
- [ ] Check on screen: farming, snow, saplings, night monsters, block health.
- [ ] Zombies reaching a player on a one-block tower.
- [ ] A fresh-PC install test of the release zip.

## Recent changes
- 2026-10-08: playtest fixes: no invisible walls at spawn (the parked bus is solid only as its own body, so its
  doors work, and Nuketown's hidden patch walls are gone from the block world); perk machines show behind glass,
  not on top of it; the mouse stays in the game window; bread on the wall (12 for 100 points); a Minecraft
  compass at the top right points at the Mystery Box (all 32 needle positions); no leaf litter.
- 2026-10-08: mob ragdolls: a Minecraft mob that dies falls over as a ragdoll (four-legged mobs roll onto their
  side, legs out) and lies about 5 seconds before it poofs; `IW4L_BO2MC_RAGDOLL=0` turns them off. The chat
  also has /summon <mob> [x y z] and /kill @e.
- 2026-10-08: polish pass: every perk also works the Minecraft way (Speed Cola mines and places faster, Double
  Tap swings faster, Deadshot's swings crit, Stamin-Up sprints without hunger, Quick Revive heals twice as fast,
  PhD Flopper takes no fall or creeper damage, Vulture Aid loots, Electric Cherry shocks the mobs too); every wall
  chalk and buyable checked; Pack-a-Punched dual-wield guns get both clips; Minecraft's chat on T with /gamemode
  (creative flight on a double jump, spectator flies through blocks), /time, /give, /tp, /kill, /seed and /help.
- 2026-10-08: playtest fixes: the bus has its front back (the driver sits behind the windscreen, no walking
  through the hood); zombies drop rotten flesh and bones; Minecraft swords and tools hurt the zombies by their
  attack damage; Minecraft mobs hit you again; a tool's crack goes as soon as you stop mining (bullet cracks
  still fade); the house doors are double doors you walk straight through; End Game ends the game.
- 2026-10-08: the Minecraft loading picture is back (BO2's Nuketown picture had replaced it).
- 2026-10-07: the vault's Spork (5,000) next to the Golden Spork (15,000); the vault weapons hang on its wall;
  frag grenades buyable again; the Blundergat and Sporks show their name and price.
- 2026-10-07: the vault across the road (3,000 points: both portals, Pack-a-Punch, Blundergat, Golden Spork);
  a Minecraft sign with name and price over every wall weapon; chalks moved off the windows and perks;
  glass windows that smash; frag grenades on the wall.
- 2026-10-07: grenades, claymores and monkey bombs are items you buy: grenades and claymores from wall chalk
  (claymores sell again once used up), monkey bombs from the box, 3 per hit, stacking; souls-round wolves die on
  lightning when the round ends; the End portal shows its starfield.
- 2026-10-07: boarded windows into the spawn room (zombies tear boards off, you nail them back); the doors now
  hold; souls round every 5 rounds with Minecraft lightning, hellhounds and wolves; swimming; endless rounds in the Nether and End.
- 2026-10-07: a new world every game; animals by day, hunting monsters at night; farming (hoe, seeds, bone meal,
  saplings that need soil); snow breaks like Minecraft's and tools no longer place into it; blocks have their own
  health; Nuketown Zombies' current menus, zombies and weapons merged in.
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
