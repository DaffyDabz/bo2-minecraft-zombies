//! bo2zm M4: the engine around Black Ops II's UI scripts, shared by the
//! game and the headless harness (`t6lua`): the scripts by name and
//! `require`, the `Engine` / `UIExpression` / `Dvar` tables the scripts
//! ask (answered from the values the host keeps: dvars, game type
//! settings, visibility bits, localized text, text widths), the LUI root
//! (`LUI.UIRoot`, sized like the engine's: 720 units high, centred), and
//! the frame step that advances animations and sends their
//! `transition_complete_<name>` events.
//!
//! A field the scripts ask that the host has no answer for is a function
//! that returns nothing, counted in `asked` (what to bind next).

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use crate::lui::{self, Rect};
use crate::value::{Table, TableRef, UserData, Value};
use crate::vm::{LuaError, Vm};

/// How wide a line of text is: (text, font file, height in units) ->
/// width in units.
pub type MeasureFn = dyn Fn(&str, &str, f32) -> f32;

/// The values the scripts read from the engine. The game writes these as
/// its state changes; the scripts read them when they run.
#[derive(Default)]
pub struct EngineValues {
    /// Dvars by name (`UIExpression.DvarString` / `DvarInt` / `DvarBool` /
    /// `DvarFloat`, `Dvar.<name>:get()`).
    pub dvars: HashMap<String, String>,
    /// `Engine.GetGametypeSetting(name)`.
    pub settings: HashMap<String, f32>,
    /// The visibility bits that are set (`CoD.BIT_*` numbers).
    pub bits: HashSet<i32>,
    /// bo2zm: his last input was a pad (`Engine.LastInput_Gamepad`): BO2's
    /// prompts show the pad's button pictures, not his keys.
    pub gamepad: bool,
    /// Localized text by key (`Engine.Localize`), upper-case keys.
    pub localize: HashMap<String, String>,
    /// String tables by name (`mp/zombiemode.csv`): rows of cells
    /// (`UIExpression.TableLookup`).
    pub tables: HashMap<String, Vec<Vec<String>>>,
    /// The local player's team number (`CoD.TEAM_ALLIES`).
    pub team: i32,
    /// The scoreboard's rows: name, then the columns' values.
    pub players: Vec<Vec<String>>,
    /// Each scoreboard row's client number.
    pub clients: Vec<i32>,
    /// The scoreboard's column names (`Engine.GetScoreBoardColumnName`).
    pub columns: Vec<String>,
    /// The safe area's size in root units (the root's own size).
    pub safe_area: (f32, f32),
    /// Each bound command's keys, as shown (`+actionslot 4` -> `4`).
    pub binds: HashMap<String, Vec<String>>,
    /// The session's modes (`CoD.SESSIONMODE_*` numbers): an offline game.
    pub session_modes: Vec<f32>,
    /// An enum dvar's choices (`Dvar.<name>:getDomainEnumStrings()`).
    pub enums: HashMap<String, Vec<String>>,
    /// His profile settings (`Engine.SetProfileVar`, `UIExpression.Profile*`)
    /// and hardware profile (`Engine.*HardwareProfileValue*`).
    pub profile: HashMap<String, String>,
    /// bo2zm M4: his player stats by path (`Engine.GetPlayerStats(c)`'s
    /// `cacLoadouts.resetWarningDisplayed`), each 0 until set.
    pub stats: HashMap<String, Value>,
    /// bo2zm M4: the maps this game plays (`Engine.GetMaps`), load names.
    pub maps: Vec<String>,
    /// bo2zm M4: the front end's game modes (`Engine.GameModeSetMode`).
    pub game_modes: Vec<f32>,
    /// bo2zm M4: the playlist picked (`Engine.SetPlaylistID`), 0 = none.
    pub playlist: usize,
    /// bo2zm M4: these menus are the front end (not in a game).
    pub front_end: bool,
    /// bo2zm M4: his name (his settings' player name), the lobby's card.
    pub player_name: String,
}

/// What the scripts asked the engine to do (the host's owner carries it
/// out): a menu answer for the server, a console command, a sound.
#[derive(Clone, Debug, PartialEq)]
pub enum EngineCall {
    /// `Engine.SendMenuResponse(controller, menu, response)`.
    MenuResponse(String, String),
    /// `Engine.Exec(controller, command)`.
    Exec(String),
    /// `Engine.PlaySound(alias)`.
    PlaySound(String),
    /// `Engine.BindCommand(controller, command, index)`: bind the next key
    /// he presses (then send `key_bound`).
    BindCommand(String, usize),
    /// `Engine.BlurWorld(controller, amount)`: blur the world behind the
    /// menus (0 = none).
    BlurWorld(f32),
    /// `Engine.PlayMenuMusic(alias)`: the menus' music (the front end's
    /// `mus_mp_frontend`).
    PlayMusic(String),
}

/// One element as drawn: its rectangle in root units (0,0 = the root's top
/// left), alpha including its parents', and what it shows.
#[derive(Clone, Debug)]
pub struct Drawn {
    pub id: usize,
    pub kind: &'static str,
    pub rect: Rect,
    pub alpha: f32,
    pub rgb: [f32; 3],
    pub material: Option<String>,
    pub text: Option<String>,
    pub font: Option<String>,
    pub alignment: i32,
    /// Horizontal anchors (left, right): where unaligned text sits.
    pub anchors: (bool, bool),
    pub z_rot: f32,
    /// A dashes bar's (count, lit) and units per dash.
    pub dashes: (i32, i32),
    pub dash_pitch: f32,
    /// bo2zm M4: the material's shader vectors (`setShaderVector`).
    pub shader: [[f32; 4]; 4],
    /// bo2zm M4: it blurs what is behind it (`setBlur`).
    pub blur: bool,
    /// bo2zm M4: the box its clipping ancestors (`setUseStencil`) keep it
    /// in, root units; None = not clipped.
    pub clip: Option<Rect>,
}

pub struct Host {
    pub vm: Vm,
    pub values: Rc<RefCell<EngineValues>>,
    /// Engine fields the scripts asked that nothing answers, with counts.
    pub asked: Rc<RefCell<BTreeMap<String, usize>>>,
    /// Modules `require` found no script for.
    pub missing: Rc<RefCell<Vec<String>>>,
    /// Engine calls since the owner last took them (`take_calls`).
    calls: Rc<RefCell<Vec<EngineCall>>>,
    /// The engine's roots: `UIRoot0` (the player's) and `UIRootFull`
    /// (the whole screen), drawn in that order.
    roots: Vec<Value>,
    /// Root size in units (720 high; the width follows the screen).
    root_size: (f32, f32),
    pub errors: Vec<String>,
    /// Steps one entry into the scripts may take.
    pub step_budget: u64,
    /// Text width in root units (font, height): GetTextDimensions and
    /// tight text.
    measure: Rc<MeasureFn>,
}

/// A script name as a lookup key: lower case, no separators, no
/// extension (`ui_mp/t6/hud.lua`, `ui_mp_t6_hud.lua` and
/// `ui_mp__t6__hud.lua` are one key).
fn script_key(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let stem = lower.strip_suffix(".lua").unwrap_or(&lower);
    stem.chars()
        .filter(|c| !matches!(c, '/' | '\\' | '_' | '.'))
        .collect()
}

fn arg(a: &[Value], i: usize) -> Value {
    a.get(i).cloned().unwrap_or(Value::Nil)
}

impl Host {
    /// A VM with the standard library, LUI's natives and the engine tables,
    /// and `scripts` ((rawfile name, bytes); a later name wins) behind
    /// `require`. Nothing runs yet (`load_base`).
    #[allow(clippy::too_many_lines)]
    pub fn new(scripts: Vec<(String, Vec<u8>)>, measure: Box<MeasureFn>) -> Host {
        let measure: Rc<MeasureFn> = Rc::from(measure);
        let measure_out = measure.clone();
        let mut vm = Vm::new();
        crate::stdlib::open(&mut vm);
        let asked: Rc<RefCell<BTreeMap<String, usize>>> = Rc::default();
        let values: Rc<RefCell<EngineValues>> = Rc::default();
        let missing: Rc<RefCell<Vec<String>>> = Rc::default();
        // What the menus ask of the engine, queued for the owner.
        let calls: Rc<RefCell<Vec<EngineCall>>> = Rc::default();
        for name in ["Engine", "UIExpression"] {
            let v = stub_table(&mut vm, name, asked.clone());
            vm.set_global(name, v);
        }
        lui::install(&mut vm);

        // Dvar.<name>: :get() reads the host's dvar, :set(v) writes it.
        {
            let dvars = Table::new_ref();
            let meta = Table::new_ref();
            let vals = values.clone();
            let index = vm.native("dvar_index", move |vm, a| {
                let name = arg(&a, 1).to_string();
                let d = Table::new_ref();
                let (v1, v2) = (vals.clone(), vals.clone());
                let n1 = name.clone();
                let get = vm.native("dvar_get", move |_, _| {
                    let v = dvar_get(&v1.borrow().dvars, &n1);
                    Ok(
                        v.map(|s| s.parse::<f32>().map_or_else(|_| Value::str(&s), Value::Num))
                            .into_iter()
                            .collect(),
                    )
                });
                let set = vm.native("dvar_set", move |_, a| {
                    dvar_set(&mut v2.borrow_mut().dvars, &name, arg(&a, 1).to_string());
                    Ok(vec![])
                });
                // getDomainEnumStrings(): an enum dvar's choices
                // (`enums`, e.g. r_mode's resolutions).
                let v3 = vals.clone();
                let n3 = arg(&a, 1).to_string();
                let domain = vm.native("dvar_domain", move |_, _| {
                    let t = Table::new_ref();
                    for (i, c) in v3.borrow().enums.get(&n3).into_iter().flatten().enumerate() {
                        t.borrow_mut()
                            .set(Value::Num((i + 1) as f32), Value::str(c));
                    }
                    Ok(vec![Value::Table(t)])
                });
                d.borrow_mut().set_str("get", get);
                d.borrow_mut().set_str("set", set);
                d.borrow_mut().set_str("getDomainEnumStrings", domain);
                Ok(vec![Value::Table(d)])
            });
            meta.borrow_mut().set_str("__index", index);
            dvars.borrow_mut().meta = Some(meta);
            vm.set_global("Dvar", Value::Table(dvars));
        }

        // UIExpression: dvars and visibility bits.
        let ui = vm.global("UIExpression");
        let dvar = |vals: &Rc<RefCell<EngineValues>>, a: &[Value]| {
            dvar_get(&vals.borrow().dvars, &arg(a, 1).to_string()).unwrap_or_default()
        };
        let v = values.clone();
        let f = vm.native("DvarString", move |_, a| {
            Ok(vec![Value::str(&dvar(&v, &a))])
        });
        set_field(&ui, "DvarString", f);
        for name in ["DvarInt", "DvarFloat"] {
            let v = values.clone();
            let f = vm.native(name, move |_, a| {
                Ok(vec![Value::Num(
                    dvar(&v, &a).trim().parse::<f32>().unwrap_or(0.0),
                )])
            });
            set_field(&ui, name, f);
        }
        let v = values.clone();
        let f = vm.native("DvarBool", move |_, a| {
            let s = dvar(&v, &a);
            let on =
                matches!(s.trim(), "1" | "true") || s.trim().parse::<f32>().is_ok_and(|n| n != 0.0);
            Ok(vec![Value::Num(if on { 1.0 } else { 0.0 })])
        });
        set_field(&ui, "DvarBool", f);
        let v = values.clone();
        let f = vm.native("IsVisibilityBitSet", move |_, a| {
            let bit = arg(&a, 1).as_num().map_or(-1, |n| n as i32);
            Ok(vec![Value::Num(if v.borrow().bits.contains(&bit) {
                1.0
            } else {
                0.0
            })])
        });
        set_field(&ui, "IsVisibilityBitSet", f);
        // ToUpper(controller, text).
        let f = vm.native("ToUpper", |_, a| {
            let text = if a.len() >= 2 { arg(&a, 1) } else { arg(&a, 0) };
            Ok(vec![Value::str(&text.to_string().to_uppercase())])
        });
        set_field(&ui, "ToUpper", f);
        // A zombies game on PC (the multiplayer exe runs zombies).
        let f = vm.native("GetCurrentPlatform", |_, _| Ok(vec![Value::str("pc")]));
        set_field(&ui, "GetCurrentPlatform", f);
        let f = vm.native("GetCurrentExe", |_, _| Ok(vec![Value::str("multiplayer")]));
        set_field(&ui, "GetCurrentExe", f);
        let f = vm.native("SessionMode_IsZombiesGame", |_, _| {
            Ok(vec![Value::Num(1.0)])
        });
        set_field(&ui, "SessionMode_IsZombiesGame", f);
        // TableLookup(controller, table, column, value[, column, value...],
        // return column): the first row whose cells match every pair
        // (numbers as their text). `maps/<x>` (CoD.mapsTable) is the zombies
        // game's `zm/<x>`.
        let v = values.clone();
        let f = vm.native("TableLookup", move |_, a| {
            let table = arg(&a, 1).to_string();
            let rest: Vec<Value> = a.iter().skip(2).cloned().collect();
            let Some((ret, pairs)) = rest.split_last() else {
                return Ok(vec![Value::str("")]);
            };
            let ret = ret.as_num().map_or(0, |n| n as usize);
            let pairs: Vec<(usize, String)> = pairs
                .chunks_exact(2)
                .map(|p| (p[0].as_num().map_or(0, |n| n as usize), p[1].to_string()))
                .collect();
            let v = v.borrow();
            let rows = v.tables.get(&table).or_else(|| {
                table
                    .strip_prefix("maps/")
                    .and_then(|x| v.tables.get(&format!("zm/{x}")))
            });
            let hit = rows
                .and_then(|rows| {
                    rows.iter().find(|r| {
                        pairs
                            .iter()
                            .all(|(c, want)| r.get(*c).is_some_and(|x| x == want))
                    })
                })
                .and_then(|r| r.get(ret).cloned())
                .unwrap_or_default();
            Ok(vec![Value::str(&hit)])
        });
        set_field(&ui, "TableLookup", f);
        // The maps table a zombies game reads (CoD.mapsTable).
        let f = vm.native("GetCurrentMapTableName", |_, _| {
            Ok(vec![Value::str("zm/mapstable.csv")])
        });
        set_field(&ui, "GetCurrentMapTableName", f);

        // Engine: the safe area, settings, localized text.
        let engine = vm.global("Engine");
        // The safe area: the whole root (a PC screen has no overscan), in
        // root units. ForController: its edges around the centre;
        // GetUserSafeArea: its width and height.
        let v = values.clone();
        let f = vm.native("GetUserSafeAreaForController", move |_, _| {
            let (w, h) = v.borrow().safe_area;
            Ok(vec![
                Value::Num(-w / 2.0),
                Value::Num(-h / 2.0),
                Value::Num(w / 2.0),
                Value::Num(h / 2.0),
            ])
        });
        set_field(&engine, "GetUserSafeAreaForController", f);
        let v = values.clone();
        let f = vm.native("GetUserSafeArea", move |_, _| {
            let (w, h) = v.borrow().safe_area;
            Ok(vec![Value::Num(w), Value::Num(h)])
        });
        set_field(&engine, "GetUserSafeArea", f);
        let v = values.clone();
        let f = vm.native("GetGametypeSetting", move |_, a| {
            Ok(vec![Value::Num(
                v.borrow()
                    .settings
                    .get(&arg(&a, 0).to_string())
                    .copied()
                    .unwrap_or(0.0),
            )])
        });
        set_field(&engine, "GetGametypeSetting", f);
        // SetGametypeSetting(name, value): Custom Games' options.
        let v = values.clone();
        let f = vm.native("SetGametypeSetting", move |_, a| {
            if let Some(x) = arg(&a, 1).as_num() {
                v.borrow_mut().settings.insert(arg(&a, 0).to_string(), x);
            }
            Ok(vec![])
        });
        set_field(&engine, "SetGametypeSetting", f);
        let v = values.clone();
        let f = vm.native("Localize", move |_, a| {
            let key = arg(&a, 0).to_string();
            let mut text = v
                .borrow()
                .localize
                .get(&key.to_ascii_uppercase())
                .cloned()
                .unwrap_or(key);
            // "&&1".. take the arguments.
            for (i, x) in a.iter().enumerate().skip(1) {
                text = text.replace(&format!("&&{i}"), &x.to_string());
            }
            Ok(vec![Value::str(&text)])
        });
        set_field(&engine, "Localize", f);
        let falsy = vm.native("false", |_, _| Ok(vec![Value::Bool(false)]));
        for name in ["IsSplitscreen", "IsDemoShoutcaster", "IsInGame"] {
            set_field(&engine, name, falsy.clone());
        }
        // SessionModeIsMode(mode): the session's modes (`session_modes`).
        let v = values.clone();
        let f = vm.native("SessionModeIsMode", move |_, a| {
            let m = arg(&a, 0).as_num().unwrap_or(f32::NAN);
            // (A Zombies session always: CoD.SESSIONMODE_ZOMBIES, 4.)
            Ok(vec![Value::Bool(
                m == 4.0 || v.borrow().session_modes.contains(&m),
            )])
        });
        set_field(&engine, "SessionModeIsMode", f);
        // bo2zm M4: the front end's game modes (CoD.GAMEMODE_*: public
        // match 0 - Solo is one -, private match 1, ...) and session modes
        // (CoD.SESSIONMODE_*: offline 0, system link 1, online 2, private 3),
        // as its menus set and ask them.
        let v = values.clone();
        let f = vm.native("GameModeIsMode", move |_, a| {
            let m = arg(&a, 0).as_num().unwrap_or(f32::NAN);
            Ok(vec![Value::Bool(v.borrow().game_modes.contains(&m))])
        });
        set_field(&engine, "GameModeIsMode", f);
        let v = values.clone();
        let f = vm.native("GameModeSetMode", move |_, a| {
            let m = arg(&a, 0).as_num().unwrap_or(f32::NAN);
            let on = a.len() < 2 || arg(&a, 1).truthy();
            let mut v = v.borrow_mut();
            v.game_modes.retain(|x| *x != m);
            if on {
                v.game_modes.push(m);
            }
            Ok(vec![])
        });
        set_field(&engine, "GameModeSetMode", f);
        let v = values.clone();
        let f = vm.native("GameModeResetModes", move |_, _| {
            v.borrow_mut().game_modes.clear();
            Ok(vec![])
        });
        set_field(&engine, "GameModeResetModes", f);
        for (name, mode) in [
            ("SessionModeSetOffline", 0.0),
            ("SessionModeSetSystemLink", 1.0),
            ("SessionModeSetOnlineGame", 2.0),
            ("SessionModeSetPrivate", 3.0),
        ] {
            let v = values.clone();
            let f = vm.native(name, move |_, a| {
                let on = a.is_empty() || arg(&a, 0).truthy();
                let mut v = v.borrow_mut();
                v.session_modes.retain(|x| *x != mode);
                if on {
                    v.session_modes.push(mode);
                }
                Ok(vec![])
            });
            set_field(&engine, name, f);
        }
        let v = values.clone();
        let f = vm.native("SessionModeResetModes", move |_, _| {
            v.borrow_mut().session_modes.clear();
            Ok(vec![])
        });
        set_field(&engine, "SessionModeResetModes", f);
        let zero = vm.native("zero", |_, _| Ok(vec![Value::Num(0.0)]));
        for name in ["GetPlayerCount", "GetAspectRatio"] {
            set_field(&engine, name, zero.clone());
        }
        set_field(&engine, "PartyConnectingToDedicated", falsy.clone());
        // One local player (the front end's per-controller loops).
        // Offline, solo: nobody is signed in to an online service.
        for name in [
            "AnySignedInToLive",
            "IsSignedInToLive",
            "IsSignedInToDemonware",
        ] {
            set_field(&engine, name, falsy.clone());
            set_field(&ui, name, falsy.clone());
        }
        let one_pad = vm.native("one_controller", |_, _| Ok(vec![Value::Num(1.0)]));
        for name in [
            "GetMaxControllerCount",
            "GetMaxLocalControllers",
            "GetUsedControllerCount",
        ] {
            set_field(&engine, name, one_pad.clone());
            set_field(&ui, name, one_pad.clone());
        }
        // ONLINE's checks (the PC front end's online service): the
        // connection is up, nothing is banned, the Steam ticket and the
        // service's fetches are done, every rating allowed.
        let truthy = vm.native("true", |_, _| Ok(vec![Value::Bool(true)]));
        set_field(&engine, "CheckNetConnection", truthy.clone());
        for name in ["IsFeatureBanned", "IsVacBanned", "WaitingForSteamTicket"] {
            set_field(&engine, name, falsy.clone());
        }
        for name in ["IsDemonwareFetchingDone", "IsContentRatingAllowed"] {
            set_field(&ui, name, one_pad.clone());
        }
        set_field(&ui, "IsAnyControllerMPRestricted", zero.clone());
        // His party (`xstartprivateparty`): he hosts it and is alone in it.
        for name in [
            "PrivatePartyHost",
            "InPrivateParty",
            "PrivatePartyHostInLobby",
            "AloneInPartyIgnoreSplitscreen",
        ] {
            set_field(&ui, name, one_pad.clone());
        }
        for name in [
            "PartyGetMemberCount",
            "PartyGetPlayerCount",
            "PartyGetTeamMemberCount",
        ] {
            set_field(&engine, name, one_pad.clone());
        }
        // The party waits for him to start (the solo lobby's Start Match);
        // a Custom Games host is ready, and Start Match starts it.
        set_field(&engine, "PartyIsWaiting", truthy.clone());
        set_field(&engine, "PartyHostIsReadyToStart", truthy.clone());
        let c = calls.clone();
        let f = vm.native("PartyHostToggleStart", move |_, _| {
            c.borrow_mut().push(EngineCall::Exec("xpartygo".to_owned()));
            Ok(vec![])
        });
        set_field(&engine, "PartyHostToggleStart", f);
        // His player stats: any path answers (0 until set); the class-reset
        // warnings were seen long ago (a new profile would show them once).
        {
            let mut v = values.borrow_mut();
            for path in [
                "cacLoadouts.resetWarningDisplayed",
                "cacLoadouts.classWarningDisplayed",
            ] {
                v.stats.insert(path.to_owned(), Value::Num(1.0));
            }
        }
        // The maps the globe offers (`Engine.GetMaps`): the ones this game
        // plays, by load name (`maps`, e.g. zm_nuked); the globe reads the
        // rest (name, place on the globe) from the maps table.
        let v = values.clone();
        let f = vm.native("GetMaps", move |_, _| {
            let t = Table::new_ref();
            for (i, name) in v.borrow().maps.iter().enumerate() {
                let m = Table::new_ref();
                m.borrow_mut().set_str("loadName", Value::str(name));
                m.borrow_mut().set_str("filter", Value::str(""));
                t.borrow_mut()
                    .set(Value::Num((i + 1) as f32), Value::Table(m));
            }
            Ok(vec![Value::Table(t)])
        });
        set_field(&engine, "GetMaps", f);
        let v = values.clone();
        let f = vm.native("GetPlayersInLobby", move |_, _| {
            Ok(vec![lobby_members(&v.borrow())])
        });
        set_field(&engine, "GetPlayersInLobby", f);
        // No friends online (offline, one PC): the joinable lists are empty.
        let empty = vm.native("empty_list", |_, _| {
            Ok(vec![Value::Table(Table::new_ref())])
        });
        for name in [
            "GetTitleFriendsOfAllLocalPlayers",
            "GetFriendsOfAllLocalPlayers",
            "GetRecentPlayers",
        ] {
            set_field(&engine, name, empty.clone());
        }
        // A map's start locations and a start location's game modes, from
        // zm/gametypestable.csv: rows `5,index,map,ref,name,desc,picture`
        // and `6,index,map,location,mode,dlc` (Nuketown: `nuked`,
        // `zstandard`); game mode names from rows `0,mode,name`.
        let v = values.clone();
        let f = vm.native("GetStartLocsZombie", move |_, a| {
            let map = arg(&a, 0).to_string();
            let v = v.borrow();
            let t = Table::new_ref();
            let rows = v
                .tables
                .get("zm/gametypestable.csv")
                .cloned()
                .unwrap_or_default();
            let cell = |r: &Vec<String>, i: usize| r.get(i).cloned().unwrap_or_default();
            let mut n = 0.0;
            for r in rows
                .iter()
                .filter(|r| cell(r, 0) == "5" && cell(r, 2) == map)
            {
                let e = Table::new_ref();
                let name = cell(r, 4);
                let shown = v
                    .localize
                    .get(&name.to_ascii_uppercase())
                    .cloned()
                    .unwrap_or(name);
                e.borrow_mut().set_str("ref", Value::str(&cell(r, 3)));
                e.borrow_mut().set_str("name", Value::str(&shown));
                e.borrow_mut()
                    .set_str("index", Value::Num(cell(r, 1).parse().unwrap_or(0.0)));
                e.borrow_mut().set_str("playerCount", Value::Num(0.0));
                n += 1.0;
                t.borrow_mut().set(Value::Num(n), Value::Table(e));
            }
            Ok(vec![Value::Table(t)])
        });
        set_field(&engine, "GetStartLocsZombie", f);
        let v = values.clone();
        let f = vm.native("GetGamemodesZombie", move |_, a| {
            let (map, loc) = (arg(&a, 0).to_string(), arg(&a, 1).to_string());
            let v = v.borrow();
            let t = Table::new_ref();
            let rows = v
                .tables
                .get("zm/gametypestable.csv")
                .cloned()
                .unwrap_or_default();
            let cell = |r: &Vec<String>, i: usize| r.get(i).cloned().unwrap_or_default();
            let mut n = 0.0;
            for r in rows
                .iter()
                .filter(|r| cell(r, 0) == "6" && cell(r, 2) == map && cell(r, 3) == loc)
            {
                let mode = cell(r, 4);
                let key = rows
                    .iter()
                    .find(|g| cell(g, 0) == "0" && cell(g, 1) == mode)
                    .map(|g| cell(g, 2))
                    .unwrap_or_default();
                let shown = v
                    .localize
                    .get(&key.to_ascii_uppercase())
                    .cloned()
                    .unwrap_or(key);
                let e = Table::new_ref();
                e.borrow_mut().set_str("ref", Value::str(&mode));
                e.borrow_mut().set_str("name", Value::str(&shown));
                e.borrow_mut()
                    .set_str("index", Value::Num(cell(r, 1).parse().unwrap_or(0.0)));
                e.borrow_mut().set_str("playerCount", Value::Num(0.0));
                n += 1.0;
                t.borrow_mut().set(Value::Num(n), Value::Table(e));
            }
            Ok(vec![Value::Table(t)])
        });
        set_field(&engine, "GetGamemodesZombie", f);
        // His copy holds every map pack's content the menus ask about.
        set_field(&engine, "HasDLCContent", truthy.clone());
        // The playlists (BO2 downloads them; built here from the maps this
        // game plays and the gametypes table): each map once per filter
        // (`playermatch`, `solomatch`: a super category, its load name
        // `_solo` for Solo), its start locations (categories, `nuked_solo`)
        // and their game modes (playlists, by id).
        let v = values.clone();
        let f = vm.native("GetPlaylistSuperCategories", move |_, _| {
            let rows = playlist_rows(&v.borrow());
            let t = Table::new_ref();
            let mut seen = Vec::new();
            for r in &rows {
                if seen.contains(&r.super_index) {
                    continue;
                }
                seen.push(r.super_index);
                let e = Table::new_ref();
                let solo = if r.filter == "solomatch" { "_solo" } else { "" };
                e.borrow_mut()
                    .set_str("loadName", Value::str(&format!("{}{solo}", r.map)));
                e.borrow_mut().set_str("filter", Value::str(r.filter));
                e.borrow_mut()
                    .set_str("index", Value::Num(r.super_index as f32));
                e.borrow_mut().set_str("playerCount", Value::Num(0.0));
                t.borrow_mut()
                    .set(Value::Num(seen.len() as f32), Value::Table(e));
            }
            Ok(vec![Value::Table(t)])
        });
        set_field(&engine, "GetPlaylistSuperCategories", f);
        let v = values.clone();
        let f = vm.native("GetPlaylistSuperCategoryID", move |_, a| {
            let id = a.iter().rev().find_map(Value::as_num).unwrap_or(0.0) as usize;
            let rows = playlist_rows(&v.borrow());
            Ok(vec![
                rows.iter()
                    .find(|r| r.id == id)
                    .map_or(Value::Nil, |r| Value::Num(r.super_index as f32)),
            ])
        });
        set_field(&engine, "GetPlaylistSuperCategoryID", f);
        let v = values.clone();
        let f = vm.native("GetPlaylistCategories", move |_, a| {
            let sup = a.iter().rev().find_map(Value::as_num).unwrap_or(0.0) as usize;
            let vb = v.borrow();
            let rows = playlist_rows(&vb);
            let t = Table::new_ref();
            let mut cats: Vec<(String, TableRef)> = Vec::new();
            for r in rows.iter().filter(|r| r.super_index == sup) {
                let solo = if r.filter == "solomatch" { "_solo" } else { "" };
                let cref = format!("{}{solo}", r.loc);
                let cat = match cats.iter().find(|(c, _)| *c == cref) {
                    Some((_, c)) => c.clone(),
                    None => {
                        let c = Table::new_ref();
                        c.borrow_mut().set_str("ref", Value::str(&cref));
                        c.borrow_mut().set_str("name", Value::str(&r.loc_name));
                        c.borrow_mut().set_str("filter", Value::str(r.filter));
                        c.borrow_mut()
                            .set_str("index", Value::Num((cats.len() + 1) as f32));
                        c.borrow_mut().set_str("playerCount", Value::Num(0.0));
                        c.borrow_mut()
                            .set_str("playlists", Value::Table(Table::new_ref()));
                        cats.push((cref.clone(), c.clone()));
                        t.borrow_mut()
                            .set(Value::Num(cats.len() as f32), Value::Table(c.clone()));
                        c
                    }
                };
                let list = cat.borrow().get_str("playlists");
                if let Value::Table(list) = list {
                    let p = Table::new_ref();
                    p.borrow_mut().set_str("index", Value::Num(r.id as f32));
                    p.borrow_mut().set_str("ref", Value::str(&r.mode));
                    p.borrow_mut().set_str("name", Value::str(&r.mode_name));
                    // bo2mc: its description (the start location's: Solo's
                    // mode list showed "nil" under SURVIVAL).
                    p.borrow_mut()
                        .set_str("description", Value::str(&r.loc_desc));
                    p.borrow_mut().set_str("playerCount", Value::Num(0.0));
                    let n = list.borrow().len() + 1;
                    list.borrow_mut().set(Value::Num(n as f32), Value::Table(p));
                }
            }
            Ok(vec![Value::Table(t)])
        });
        set_field(&engine, "GetPlaylistCategories", f);
        // The playlist picked (`Engine.SetPlaylistID`) and its facts.
        let v = values.clone();
        let f = vm.native("GetPlaylistID", move |_, _| {
            Ok(vec![Value::Num(v.borrow().playlist as f32)])
        });
        set_field(&engine, "GetPlaylistID", f.clone());
        set_field(&ui, "GetPlaylistID", f);
        let v = values.clone();
        let f = vm.native("SetPlaylistID", move |_, a| {
            let id = a.iter().rev().find_map(|x| {
                x.as_num()
                    .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
            });
            if let Some(id) = id {
                let mut vb = v.borrow_mut();
                vb.playlist = id as usize;
                // bo2mc: as BO2's party does, the playlist's map, start
                // location and game mode become the ui dvars (the loading
                // screen's picture is named from them; Start Match loads
                // the map they name).
                let rows = playlist_rows(&vb);
                if let Some(r) = rows.iter().find(|r| r.id == id as usize) {
                    let (map, loc, mode) = (r.map.clone(), r.loc.clone(), r.mode.clone());
                    dvar_set(&mut vb.dvars, "ui_mapname", map);
                    dvar_set(&mut vb.dvars, "ui_zm_mapstartlocation", loc);
                    dvar_set(&mut vb.dvars, "ui_gametype", mode);
                }
            }
            Ok(vec![])
        });
        set_field(&engine, "SetPlaylistID", f);
        for (name, field) in [
            ("GetPlaylistGameType", 0usize),
            ("GetPlaylistCategoryFilter", 1),
            ("GetPlaylistCategoryName", 2),
            ("GetPlaylistName", 3),
        ] {
            let v = values.clone();
            let f = vm.native(name, move |_, a| {
                let vb = v.borrow();
                let id = a
                    .iter()
                    .rev()
                    .find_map(Value::as_num)
                    .map_or(vb.playlist, |n| n as usize);
                let rows = playlist_rows(&vb);
                let Some(r) = rows.iter().find(|r| r.id == id) else {
                    return Ok(vec![Value::str("")]);
                };
                let out = match field {
                    0 => r.mode.clone(),
                    1 => r.filter.to_owned(),
                    2 => r.loc_name.clone(),
                    _ => r.mode_name.clone(),
                };
                Ok(vec![Value::str(&out)])
            });
            set_field(&engine, name, f.clone());
            set_field(&ui, name, f);
        }
        let stats = stat_node(&mut vm, values.clone(), String::new());
        let f = vm.native("GetPlayerStats", move |_, _| Ok(vec![stats.clone()]));
        set_field(&engine, "GetPlayerStats", f);
        let v = values.clone();
        let f = vm.native("LastInput_Gamepad", move |_, _| {
            Ok(vec![Value::Bool(v.borrow().gamepad)])
        });
        set_field(&engine, "LastInput_Gamepad", f);
        let nothing = vm.native("nothing", |_, _| Ok(vec![]));
        // Engine.SetDvar(name, value) (the front end's ui_mapname, ...).
        let v = values.clone();
        let f = vm.native("SetDvar", move |_, a| {
            dvar_set(
                &mut v.borrow_mut().dvars,
                &arg(&a, 0).to_string(),
                arg(&a, 1).to_string(),
            );
            Ok(vec![])
        });
        set_field(&engine, "SetDvar", f);
        for name in ["SetForceMouseRootFull", "StopEditingPresetClass"] {
            set_field(&engine, name, nothing.clone());
        }
        let c = calls.clone();
        let f = vm.native("PlayMenuMusic", move |_, a| {
            c.borrow_mut()
                .push(EngineCall::PlayMusic(arg(&a, 0).to_string()));
            Ok(vec![])
        });
        set_field(&engine, "PlayMenuMusic", f);
        let c = calls.clone();
        let f = vm.native("SendMenuResponse", move |_, a| {
            c.borrow_mut().push(EngineCall::MenuResponse(
                arg(&a, 1).to_string(),
                arg(&a, 2).to_string(),
            ));
            Ok(vec![])
        });
        set_field(&engine, "SendMenuResponse", f);
        let c = calls.clone();
        let f = vm.native("Exec", move |_, a| {
            // Exec(controller, command) or Exec(command).
            let cmd = a
                .iter()
                .rev()
                .find_map(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default();
            c.borrow_mut().push(EngineCall::Exec(cmd));
            Ok(vec![])
        });
        set_field(&engine, "Exec", f.clone());
        // ExecNow: the same, done at once (the solo lobby's Start Match:
        // `xpartyready 1`).
        set_field(&engine, "ExecNow", f);
        let c = calls.clone();
        let f = vm.native("PlaySound", move |_, a| {
            c.borrow_mut()
                .push(EngineCall::PlaySound(arg(&a, 0).to_string()));
            Ok(vec![])
        });
        set_field(&engine, "PlaySound", f);
        // A solo game pauses (one local player, no one else in the game).
        let f = vm.native("CanPauseZombiesGame", |_, _| Ok(vec![Value::Bool(true)]));
        set_field(&engine, "CanPauseZombiesGame", f);
        let c = calls.clone();
        let f = vm.native("BlurWorld", move |_, a| {
            let amount = match a.get(1) {
                Some(Value::Num(n)) => *n,
                _ => 0.0,
            };
            c.borrow_mut().push(EngineCall::BlurWorld(amount));
            Ok(vec![])
        });
        set_field(&engine, "BlurWorld", f);
        for name in [
            "IsMigrating",
            "LockInput",
            "SetUIActive",
            "ProbationCheckIfPenalizedForQuit",
        ] {
            set_field(
                &engine,
                name,
                if name == "IsMigrating" || name == "ProbationCheckIfPenalizedForQuit" {
                    falsy.clone()
                } else {
                    nothing.clone()
                },
            );
        }
        let one = vm.native("one", |_, _| Ok(vec![Value::Num(1.0)]));
        // In a game (the HUD) or not (the front end).
        let v = values.clone();
        let f = vm.native("IsInGame", move |_, _| {
            Ok(vec![Value::Num(if v.borrow().front_end {
                0.0
            } else {
                1.0
            })])
        });
        set_field(&ui, "IsInGame", f);
        // His keyboard and mouse are controller 0 (menus pass on input only
        // from a controller in use).
        set_field(&ui, "IsControllerBeingUsed", one.clone());
        // One local player.
        set_field(&ui, "SplitscreenNum", one.clone());
        set_field(&engine, "PartyGetPlayerCount", one.clone());
        set_field(&engine, "GetClientNum", zero.clone());
        set_field(&ui, "IsGuest", zero.clone());
        // The scripts' debug output (a release build prints nothing).
        vm.set_global("DebugPrint", nothing.clone());
        for name in ["IsDemoPlaying", "SessionMode_IsOnlineGame"] {
            set_field(&ui, name, zero.clone());
        }
        // The scoreboard: one team (CoD.TEAM_ALLIES, set by `load_base`'s
        // caller through `players`), its players.
        let v = values.clone();
        let f = vm.native("GetTeamPositions", move |_, _| {
            let t = Table::new_ref();
            let row = Table::new_ref();
            let v = v.borrow();
            // The team's score: its players' first column (score).
            let score: f32 = v
                .players
                .iter()
                .filter_map(|r| r.get(1)?.parse::<f32>().ok())
                .sum();
            row.borrow_mut().set_str("team", Value::Num(v.team as f32));
            row.borrow_mut().set_str("score", Value::Num(score));
            t.borrow_mut().set(Value::Num(1.0), Value::Table(row));
            Ok(vec![Value::Table(t)])
        });
        set_field(&engine, "GetTeamPositions", f);
        let v = values.clone();
        let f = vm.native("GetMatchScoreboardClientCount", move |_, _| {
            Ok(vec![Value::Num(v.borrow().players.len() as f32)])
        });
        set_field(&engine, "GetMatchScoreboardClientCount", f);
        // GetMatchScoreboardIndexAndClientNumForTeam(i, team, sort): row i
        // and its client number.
        let v = values.clone();
        let f = vm.native("GetMatchScoreboardIndexAndClientNumForTeam", move |_, a| {
            let i = arg(&a, 0).as_num().map_or(0, |n| n as usize);
            let c = v.borrow().clients.get(i).copied().unwrap_or(i as i32);
            Ok(vec![Value::Num(i as f32), Value::Num(c as f32)])
        });
        set_field(&engine, "GetMatchScoreboardIndexAndClientNumForTeam", f);
        let v = values.clone();
        let f = vm.native("GetFullGamertagForScoreboardIndex", move |_, a| {
            let i = arg(&a, 0).as_num().map_or(0, |n| n as usize);
            let name = v
                .borrow()
                .players
                .get(i)
                .and_then(|r| r.first().cloned())
                .unwrap_or_default();
            Ok(vec![Value::str(&name)])
        });
        set_field(&engine, "GetFullGamertagForScoreboardIndex", f);
        let v = values.clone();
        let f = vm.native("GetScoreboardColumnForScoreboardIndex", move |_, a| {
            let i = arg(&a, 0).as_num().map_or(0, |n| n as usize);
            let col = arg(&a, 1).as_num().map_or(0, |n| n as usize);
            let cell = v
                .borrow()
                .players
                .get(i)
                .and_then(|r| r.get(col + 1).cloned())
                .unwrap_or_default();
            Ok(vec![Value::str(&cell)])
        });
        set_field(&engine, "GetScoreboardColumnForScoreboardIndex", f);
        // A local game: no ping (full bars), nobody muted.
        set_field(&engine, "GetPingForScoreboardIndex", zero.clone());
        set_field(&engine, "IsPlayerMuteToggled", falsy.clone());
        // Nuketown's teams (teamset_cdc): the allies CDC, the axis CIA; the
        // other team numbers (the lobby lists every team) have none.
        let f = vm.native("GetFactionForTeam", |_, a| {
            Ok(match arg(&a, 0).as_num().map(|n| n as i32) {
                Some(1) => vec![Value::str("cdc")],
                Some(2) => vec![Value::str("cia")],
                _ => vec![Value::str("")],
            })
        });
        set_field(&engine, "GetFactionForTeam", f);
        // A faction's colour (r, g, b) from zm/factiontable.csv (name, team,
        // red, green, blue 0-255; the CDC gold 224 178 46).
        let v = values.clone();
        let f = vm.native("GetFactionColor", move |_, a| {
            let want = arg(&a, 0).to_string();
            let v = v.borrow();
            let rgb = v
                .tables
                .get("zm/factiontable.csv")
                .and_then(|rows| rows.iter().find(|r| r.first().is_some_and(|c| *c == want)))
                .map(|r| {
                    [2, 3, 4].map(|i| {
                        r.get(i).and_then(|c| c.parse::<f32>().ok()).unwrap_or(0.0) / 255.0
                    })
                })
                .unwrap_or([0.0, 0.502, 1.0]);
            Ok(rgb.into_iter().map(Value::Num).collect())
        });
        set_field(&engine, "GetFactionColor", f);
        let v = values.clone();
        let f = vm.native("GetScoreBoardColumnName", move |_, a| {
            let i = arg(&a, 1).as_num().map_or(0, |n| n as usize);
            Ok(vec![Value::str(
                v.borrow().columns.get(i).map_or("", String::as_str),
            )])
        });
        set_field(&engine, "GetScoreBoardColumnName", f);
        // Weapons (the HUD passes the weapon the engine named in
        // `hud_update_weapon`: here its name).
        let f = vm.native("IsWeaponType", |_, a| {
            let w = arg(&a, 0).to_string();
            let hit = match arg(&a, 1).to_string().as_str() {
                "melee" => ["knife", "bowie", "tazer", "fists", "sickle"]
                    .iter()
                    .any(|k| w.contains(k)),
                "grenade" => ["grenade", "monkey", "claymore", "emp"]
                    .iter()
                    .any(|k| w.contains(k)),
                _ => false,
            };
            Ok(vec![Value::Bool(hit)])
        });
        set_field(&engine, "IsWeaponType", f);
        set_field(&engine, "IsOverheatWeapon", falsy.clone());
        set_field(&engine, "IsShoutcaster", falsy.clone());
        set_field(&engine, "ForceHUDRefresh", nothing.clone());
        set_field(&engine, "GetActiveLocalClientsCount", one.clone());
        // The key a command is bound to (the string argument naming it).
        // (controller, command[, index]): the index-th key (0 first).
        for (table, name) in [
            (&engine, "GetKeyBindingLocalizedString"),
            (&ui, "KeyBinding"),
        ] {
            let v = values.clone();
            let f = vm.native(name, move |_, a| {
                let at = a.iter().position(|x| x.as_str().is_some());
                let cmd = at.and_then(|i| a[i].as_str()).unwrap_or("").to_owned();
                let index = at
                    .and_then(|i| a.get(i + 1))
                    .and_then(Value::as_num)
                    .map_or(0, |n| n as usize);
                let key = v
                    .borrow()
                    .binds
                    .get(&cmd)
                    .and_then(|keys| keys.get(index).cloned())
                    .unwrap_or_default();
                Ok(vec![Value::str(&key.to_uppercase())])
            });
            set_field(table, name, f);
        }
        // Profile and hardware-profile settings: kept as text by name.
        for (name, arg_at) in [("SetProfileVar", 1usize), ("SetHardwareProfileValue", 0)] {
            let v = values.clone();
            let f = vm.native(name, move |_, a| {
                let key = arg(&a, arg_at).to_string();
                v.borrow_mut()
                    .profile
                    .insert(key, arg(&a, arg_at + 1).to_string());
                Ok(vec![])
            });
            set_field(&engine, name, f);
        }
        let v = values.clone();
        let f = vm.native("GetHardwareProfileValueAsString", move |_, a| {
            let key = arg(&a, 0).to_string();
            let vb = v.borrow();
            Ok(vec![Value::str(
                vb.profile
                    .get(&key)
                    .or_else(|| vb.dvars.get(&key))
                    .map_or("0", String::as_str),
            )])
        });
        set_field(&engine, "GetHardwareProfileValueAsString", f);
        set_field(&engine, "SyncHardwareProfileWithDvars", nothing.clone());
        for (name, number) in [
            ("ProfileValueAsString", false),
            ("ProfileInt", true),
            ("ProfileFloat", true),
            ("ProfileBool", true),
        ] {
            let v = values.clone();
            let f = vm.native(name, move |_, a| {
                let key = arg(&a, 1).to_string();
                // An unset profile value reads 0, as BO2's profile defaults
                // it (the map picker's `unlock_crumbs_zm` bits).
                let s = v
                    .borrow()
                    .profile
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| "0".to_owned());
                Ok(vec![if number {
                    Value::Num(s.trim().parse().unwrap_or(0.0))
                } else {
                    Value::str(&s)
                }])
            });
            set_field(&ui, name, f);
        }
        // BindCommand(controller, command, index): the next key he presses.
        let c = calls.clone();
        let f = vm.native("BindCommand", move |_, a| {
            let cmd = arg(&a, 1).to_string();
            c.borrow_mut().push(EngineCall::BindCommand(
                cmd,
                arg(&a, 2).as_num().map_or(0, |n| n as usize),
            ));
            Ok(vec![])
        });
        set_field(&engine, "BindCommand", f);

        // GetTextDimensions(text, font, height) -> left, top, right, bottom.
        let f = vm.native("GetTextDimensions", move |_, a| {
            let text = arg(&a, 0).to_string();
            let font = match arg(&a, 1) {
                Value::User(u) => font_name(&u).unwrap_or_default(),
                other => other.to_string(),
            };
            let h = arg(&a, 2).as_num().unwrap_or(0.0);
            Ok(vec![
                Value::Num(0.0),
                Value::Num(0.0),
                Value::Num(measure(&text, &font, h)),
                Value::Num(h),
            ])
        });
        vm.set_global("GetTextDimensions", f);

        // require(module): the script named like it under ui/ or ui_mp/.
        let mut by_key: HashMap<String, Rc<[u8]>> = HashMap::new();
        for (name, bytes) in scripts {
            if name.to_ascii_lowercase().ends_with(".lua") {
                by_key.insert(script_key(&name), Rc::from(bytes));
            }
        }
        let loaded = Table::new_ref();
        let miss = missing.clone();
        let require = vm.native("require", move |vm, a| {
            let module = arg(&a, 0).to_string();
            let hit = loaded.borrow().get_str(&module);
            if hit.truthy() {
                return Ok(vec![hit]);
            }
            let stem = script_key(&module);
            let Some(bytes) = ["ui", "uimp"]
                .iter()
                .find_map(|p| by_key.get(&format!("{p}{stem}")))
                .cloned()
            else {
                // The engine's require skips a script the zones lack (the
                // zombies HUD names multiplayer-only ones).
                miss.borrow_mut().push(module);
                return Ok(vec![]);
            };
            loaded.borrow_mut().set_str(&module, Value::Bool(true));
            let chunk =
                crate::parse(&bytes).map_err(|e| LuaError::new(format!("{module}: {e}")))?;
            let f = vm.load(chunk.main);
            let r = vm.call(f, vec![])?;
            let v = r
                .into_iter()
                .next()
                .filter(Value::truthy)
                .unwrap_or(Value::Bool(true));
            loaded.borrow_mut().set_str(&module, v.clone());
            Ok(vec![v])
        });
        vm.set_global("require", require);

        // The fonts' resolution (`fonts/720/...`), read while CoD's base
        // loads.
        values
            .borrow_mut()
            .dvars
            .insert("r_fontResolution".to_owned(), "720".to_owned());
        values.borrow_mut().safe_area = (1280.0, 720.0);
        Host {
            vm,
            values,
            asked,
            missing,
            calls,
            roots: Vec::new(),
            root_size: (1280.0, 720.0),
            errors: Vec::new(),
            step_budget: 20_000_000,
            measure: measure_out,
        }
    }

    /// Call into the scripts with a fresh step budget; an error is kept in
    /// `errors` (the scripts go on, as the engine's do).
    pub fn call(&mut self, f: Value, args: Vec<Value>) -> Option<Vec<Value>> {
        self.vm.steps = 0;
        self.vm.step_limit = self.step_budget;
        match self.vm.call(f, args) {
            Ok(r) => Some(r),
            Err(e) => {
                if self.errors.len() < 200 {
                    self.errors.push(e.to_string());
                }
                None
            }
        }
    }

    pub fn require(&mut self, module: &str) -> Option<Value> {
        let r = self.vm.global("require");
        self.call(r, vec![Value::str(module)])
            .map(|v| v.into_iter().next().unwrap_or(Value::Nil))
    }

    /// LUI's core and CoD's base, as the engine loads them first.
    pub fn load_base(&mut self) {
        let metas: Vec<(&str, Option<TableRef>)> = ["Engine", "UIExpression"]
            .into_iter()
            .map(|n| {
                (
                    n,
                    self.vm
                        .global(n)
                        .table()
                        .and_then(|t| t.borrow().meta.clone()),
                )
            })
            .collect();
        for base in ["LUI.LUI", "T6.CoDBase"] {
            self.require(base);
        }
        // The engine's element functions on LUI's element class too (the
        // scripts call `LUI.UIElement.removeElement(self, child)`).
        let class = self.field("LUI", "UIElement");
        if let Some(class) = class.table() {
            let natives = lui::natives();
            let natives = natives.borrow();
            let mut key = Value::Nil;
            while let Some((k, v)) = natives.next(&key) {
                if matches!(class.borrow().get(&k), Value::Nil) {
                    class.borrow_mut().set(k.clone(), v);
                }
                key = k;
            }
        }
        // CoD's base answers a missing engine field with CoD.NullFunction;
        // ours does the same and counts it.
        for (n, meta) in metas {
            if let Some(t) = self.vm.global(n).table() {
                t.borrow_mut().meta = meta;
            }
        }
    }

    /// Set a function on an engine table (`Engine`, `UIExpression`, ...).
    pub fn bind(
        &mut self,
        table: &str,
        name: &'static str,
        f: impl Fn(&mut Vm, Vec<Value>) -> Result<Vec<Value>, LuaError> + 'static,
    ) {
        let t = self.vm.global(table);
        let v = self.vm.native(name, f);
        set_field(&t, name, v);
    }

    /// A material, as the scripts' `RegisterMaterial(name)` makes it.
    pub fn material(&mut self, name: &str) -> Value {
        let f = self.vm.global("RegisterMaterial");
        self.call(f, vec![Value::str(name)])
            .and_then(|r| r.into_iter().next())
            .unwrap_or(Value::Nil)
    }

    /// A field of a global table (`CoD.BIT_HUD_VISIBLE`).
    /// Set a field of a global table (`CoD.PlaylistCategoryFilter`).
    pub fn set_global_field(&mut self, global: &str, name: &str, value: Value) {
        let g = self.vm.global(global);
        set_field(&g, name, value);
    }

    pub fn field(&mut self, global: &str, name: &str) -> Value {
        let g = self.vm.global(global);
        self.vm.index(&g, &Value::str(name)).unwrap_or(Value::Nil)
    }

    /// The player's root (`LUI.UIRoot.new("UIRoot0")`, with `UIRootFull`
    /// beside it), made on first use and sized for a screen of this aspect
    /// (width / height).
    pub fn root(&mut self, aspect: f32) -> Option<Value> {
        let w = 720.0 * aspect.max(0.5);
        if self.roots.is_empty() {
            let ui_root = self.field("LUI", "UIRoot");
            let new = self.vm.index(&ui_root, &Value::str("new")).ok()?;
            for name in ["UIRoot0", "UIRootFull"] {
                let root = self
                    .call(new.clone(), vec![Value::str(name)])?
                    .into_iter()
                    .next()?;
                lui::add_root(lui::element(&root)?);
                self.roots.push(root);
            }
            self.root_size = (0.0, 0.0);
        }
        if (self.root_size.0 - w).abs() > 0.5 {
            self.root_size = (w, 720.0);
            self.values.borrow_mut().safe_area = (w, 720.0);
            lui::set_root_rect([0.0, 0.0, w, 720.0]);
            for root in self.roots.clone() {
                self.event(
                    &root,
                    "resize",
                    &[
                        ("width", Value::Num(w)),
                        ("height", Value::Num(720.0)),
                        ("unitsToPixels", Value::Num(1.0)),
                    ],
                );
            }
        }
        self.roots.first().cloned()
    }

    /// Send `name` with `fields` to an element (`processEvent`).
    pub fn event(&mut self, target: &Value, name: &str, fields: &[(&str, Value)]) {
        let t = Table::new_ref();
        t.borrow_mut().set_str("name", Value::str(name));
        t.borrow_mut().set_str("controller", Value::Num(0.0));
        for (k, v) in fields {
            t.borrow_mut().set_str(k, v.clone());
        }
        let pe = self
            .vm
            .index(target, &Value::str("processEvent"))
            .unwrap_or(Value::Nil);
        self.call(pe, vec![target.clone(), Value::Table(t)]);
    }

    /// Send an event to the root (and so to every menu on it).
    pub fn root_event(&mut self, name: &str, fields: &[(&str, Value)]) {
        if let Some(root) = self.roots.first().cloned() {
            self.event(&root, name, fields);
        }
    }

    /// The mouse, as the engine sends it to the root: `name` is
    /// mousemove / mousedown / mouseup, (x, y) in root units, `button`
    /// left / right.
    pub fn mouse(&mut self, name: &str, x: f32, y: f32, button: &str) {
        self.root_event(
            name,
            &[
                ("rootName", Value::str("UIRoot0")),
                ("x", Value::Num(x)),
                ("y", Value::Num(y)),
                ("button", Value::str(button)),
                // His mouse is controller 0's (the menus a controller owns
                // take only its input).
                ("controller", Value::Num(0.0)),
            ],
        );
    }

    /// A pad button (or the key that stands for it): primary, secondary,
    /// start, up, down, left, right, ...
    pub fn button(&mut self, button: &str, down: bool) {
        self.root_event(
            "gamepad_button",
            &[
                ("button", Value::str(button)),
                ("down", Value::Bool(down)),
                ("qualifier", Value::str("keyboard")),
                // His keys and pad are controller 0 (the lobby, opened for
                // a controller, takes only that one's buttons).
                ("controller", Value::Num(0.0)),
            ],
        );
    }

    /// The engine calls the scripts made since the last time.
    /// bo2mc: the map the menus picked (load name): the playlist's map,
    /// else `ui_mapname`.
    pub fn picked_map(&self) -> Option<String> {
        let v = self.values.borrow();
        playlist_rows(&v)
            .into_iter()
            .find(|r| r.id == v.playlist)
            .map(|r| r.map)
            .or_else(|| {
                v.dvars
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("ui_mapname"))
                    .map(|(_, m)| m.clone())
            })
    }

    pub fn take_calls(&mut self) -> Vec<EngineCall> {
        std::mem::take(&mut *self.calls.borrow_mut())
    }

    /// bo2zm M4 (debugging aid): an element's parent chain (id, name, rect).
    pub fn chain_of(&self, id: usize) -> Vec<(usize, String, Option<lui::Rect>)> {
        self.roots
            .iter()
            .filter_map(lui::element)
            .map(|r| lui::chain_of(&r, id))
            .find(|c| !c.is_empty())
            .unwrap_or_default()
    }

    /// bo2zm M4 (debugging aid): every open menu and its input state.
    pub fn menu_report(&self) -> Vec<String> {
        self.roots
            .iter()
            .filter_map(lui::element)
            .flat_map(|r| lui::menu_report(&r))
            .collect()
    }

    /// The menus open over the HUD (the root's children other than the HUD
    /// menu: `Menu.class`, popups).
    pub fn open_menus(&self) -> Vec<String> {
        let Some(root) = self.roots.first().and_then(lui::element) else {
            return Vec::new();
        };
        lui::child_ids(&root)
            .into_iter()
            .filter(|id| id.starts_with("Menu.") && id != "Menu.HUD")
            .collect()
    }

    /// The engine ids of a menu's elements (its `id`, e.g.
    /// `Menu.Scoreboard`), wherever it was added.
    pub fn menu_element_ids(&self, id: &str) -> Option<std::collections::HashSet<usize>> {
        self.roots
            .iter()
            .filter_map(lui::element)
            .find_map(|r| lui::ids_under_named(&r, id))
    }

    /// Open a menu on the root (the root's `addmenu`), as the engine does.
    pub fn open_menu(&mut self, menu: &str) {
        self.root_event("addmenu", &[("menu", Value::str(menu))]);
    }

    /// One frame at `now_ms`: start the animations the scripts wrote, move
    /// every animation on, and send each one that ended its
    /// `transition_complete_<name>` (the handlers may start more, so this
    /// repeats a few times).
    pub fn frame(&mut self, now_ms: f64) {
        // Streamed pictures the scripts set are ready (resident here).
        for el in lui::take_streamed_ready() {
            self.event(&Value::User(el), "streamed_image_ready", &[]);
        }
        for _ in 0..4 {
            lui::commit_pending();
            let done = lui::tick(now_ms);
            if done.is_empty() {
                break;
            }
            for (el, anim, interrupted, late) in done {
                // BO2ZM_LUI_TRANSLOG=1: every animation's end (debugging aid).
                if std::env::var_os("BO2ZM_LUI_TRANSLOG").is_some() {
                    let id = lui::with_id(&el);
                    eprintln!(
                        "bo2zm lui transition #{id} {anim} interrupted={interrupted} at {now_ms:.0}"
                    );
                }
                let target = Value::User(el);
                // `interrupted` only when a new animation cut it short (the
                // globe's handlers test `event.interrupted == nil`).
                let mut fields = vec![("lateness", Value::Num(late as f32))];
                if interrupted {
                    fields.push(("interrupted", Value::Bool(true)));
                }
                self.event(&target, &format!("transition_complete_{anim}"), &fields);
            }
        }
    }

    /// LUI's clock (the last frame's time).
    pub fn now_ms(&self) -> f64 {
        lui::now_ms()
    }

    /// The element with this id, as a value (debugging aid).
    pub fn element_by_id(&self, id: usize) -> Option<Value> {
        self.roots
            .iter()
            .filter_map(lui::element)
            .find_map(|r| lui::find(&r, id))
            .map(Value::User)
    }

    /// The element trees under the roots (debugging aid).
    pub fn tree(&self) -> Vec<String> {
        let mut out = Vec::new();
        for r in self.roots.iter().filter_map(lui::element) {
            lui::tree(&r, 0, &mut out);
        }
        out
    }

    /// Every element in drawing order, in root units.
    pub fn drawn(&self) -> Vec<Drawn> {
        let (w, h) = self.root_size;
        self.roots
            .iter()
            .filter_map(lui::element)
            .flat_map(|root| lui::layout_clipped(&root, [0.0, 0.0, w, h], &*self.measure))
            .map(|(u, rect, alpha, clip)| {
                let mut d = drawn_of(&u, rect, alpha);
                d.clip = clip;
                d
            })
            .collect()
    }
}

fn drawn_of(u: &Rc<UserData>, rect: Rect, alpha: f32) -> Drawn {
    let d = u.data.borrow();
    let e = d.downcast_ref::<lui::Element>().expect("element");
    let s = &e.state;
    Drawn {
        id: e.id,
        kind: e.kind,
        rect,
        alpha,
        rgb: [s.red, s.green, s.blue],
        material: s.material.clone(),
        text: s.text.clone(),
        font: s.font.clone(),
        alignment: s.alignment,
        anchors: (s.left_anchor, s.right_anchor),
        z_rot: s.z_rot,
        dashes: e.dashes,
        dash_pitch: e.dash_pitch,
        shader: s.shader,
        blur: e.blur,
        clip: None,
    }
}

fn font_name(u: &UserData) -> Option<String> {
    if u.kind != "font" {
        return None;
    }
    u.data.borrow().downcast_ref::<String>().cloned()
}

/// A dvar by name, any case (CoD's dvar names ignore case: the lobby asks
/// `ui_gameType`, the engine keeps `ui_gametype`).
fn dvar_get(dvars: &HashMap<String, String>, name: &str) -> Option<String> {
    dvars
        .get(name)
        .or_else(|| {
            dvars
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v)
        })
        .cloned()
}

/// Set a dvar, keeping the spelling it already has.
fn dvar_set(dvars: &mut HashMap<String, String>, name: &str, value: String) {
    let key = dvars
        .keys()
        .find(|k| k.eq_ignore_ascii_case(name))
        .cloned()
        .unwrap_or_else(|| name.to_owned());
    dvars.insert(key, value);
}

fn set_field(t: &Value, name: &str, v: Value) {
    if let Value::Table(t) = t {
        t.borrow_mut().set_str(name, v);
    }
}

/// bo2zm M4: the lobby's members (`Engine.GetPlayersInLobby`, the lobby
/// updates' `members`): him alone, host of his own party, ready.
pub fn lobby_members(v: &EngineValues) -> Value {
    let m = Table::new_ref();
    {
        let mut t = m.borrow_mut();
        let name = if v.player_name.is_empty() {
            "Player"
        } else {
            v.player_name.as_str()
        };
        t.set_str("name", Value::str(name));
        t.set_str("gamertag", Value::str(name));
        t.set_str("clantag", Value::str(""));
        t.set_str("tagprefix", Value::str(""));
        // Rank 1's icon (mp/rankicontable_zm.csv, the menus' column).
        t.set_str("rankIcon", Value::str("menu_zm_rank_1"));
        t.set_str("xuid", Value::str("1"));
        t.set_str("playerXuid", Value::str("1"));
        t.set_str("clientNum", Value::Num(0.0));
        t.set_str("controller", Value::Num(0.0));
        t.set_str("team", Value::Num(v.team as f32));
        t.set_str("rank", Value::Num(0.0));
        t.set_str("prestige", Value::Num(0.0));
        t.set_str("score", Value::Num(0.0));
        t.set_str("isLocal", Value::Bool(true));
        t.set_str("isHost", Value::Bool(true));
        t.set_str("isGuest", Value::Bool(false));
        t.set_str("isInParty", Value::Bool(true));
        t.set_str("isReady", Value::Bool(true));
        t.set_str("subparty", Value::Num(0.0));
        t.set_str("leagueTeamID", Value::Num(0.0));
    }
    let list = Table::new_ref();
    list.borrow_mut().set(Value::Num(1.0), Value::Table(m));
    Value::Table(list)
}

/// bo2zm M4: one playlist: its id, its map's super category (one per map
/// and filter), the map, the filter (`playermatch` / `solomatch`), the
/// start location and game mode (refs and shown names).
struct PlaylistRow {
    id: usize,
    super_index: usize,
    map: String,
    filter: &'static str,
    loc: String,
    loc_name: String,
    /// bo2mc: the start location's description key.
    loc_desc: String,
    mode: String,
    mode_name: String,
}

/// The playlists the menus offer: every start location and game mode the
/// gametypes table gives the maps this game plays, for Public Match and
/// for Solo (ids from 1 in that order).
fn playlist_rows(v: &EngineValues) -> Vec<PlaylistRow> {
    let rows = v
        .tables
        .get("zm/gametypestable.csv")
        .cloned()
        .unwrap_or_default();
    let cell = |r: &Vec<String>, i: usize| r.get(i).cloned().unwrap_or_default();
    let shown = |k: String| {
        v.localize
            .get(&k.to_ascii_uppercase())
            .cloned()
            .unwrap_or(k)
    };
    let mut out = Vec::new();
    let mut super_index = 0;
    for filter in ["playermatch", "solomatch"] {
        for map in &v.maps {
            super_index += 1;
            for loc in rows
                .iter()
                .filter(|r| cell(r, 0) == "5" && cell(r, 2) == *map)
            {
                let loc_ref = cell(loc, 3);
                for m in rows
                    .iter()
                    .filter(|r| cell(r, 0) == "6" && cell(r, 2) == *map && cell(r, 3) == loc_ref)
                {
                    let mode = cell(m, 4);
                    let mode_key = rows
                        .iter()
                        .find(|g| cell(g, 0) == "0" && cell(g, 1) == mode)
                        .map(|g| cell(g, 2))
                        .unwrap_or_default();
                    out.push(PlaylistRow {
                        id: out.len() + 1,
                        super_index,
                        map: map.clone(),
                        filter,
                        loc: loc_ref.clone(),
                        loc_name: shown(cell(loc, 4)),
                        loc_desc: cell(loc, 5),
                        mode,
                        mode_name: shown(mode_key),
                    });
                }
            }
        }
    }
    out
}

/// bo2zm M4: one node of BO2's player stats (`stats.cacLoadouts.
/// resetWarningDisplayed:get()`): any field is another node (made once and
/// kept), `:get()` reads its value (0 until set), `:set(v)` stores it.
fn stat_node(vm: &mut Vm, values: Rc<RefCell<EngineValues>>, path: String) -> Value {
    let t: TableRef = Table::new_ref();
    let meta = Table::new_ref();
    let index = vm.native("stat_index", move |vm, a| {
        let key = arg(&a, 1);
        let name = key.to_string();
        let p = path.clone();
        let out = match name.as_str() {
            "get" => {
                let v = values.clone();
                vm.native("stat_get", move |_, _| {
                    Ok(vec![
                        v.borrow().stats.get(&p).cloned().unwrap_or(Value::Num(0.0)),
                    ])
                })
            }
            "set" => {
                let v = values.clone();
                vm.native("stat_set", move |_, a| {
                    v.borrow_mut().stats.insert(p.clone(), arg(&a, 1));
                    Ok(vec![])
                })
            }
            _ => {
                let child = stat_node(
                    vm,
                    values.clone(),
                    if p.is_empty() {
                        name
                    } else {
                        format!("{p}.{name}")
                    },
                );
                if let Value::Table(me) = arg(&a, 0) {
                    me.borrow_mut().set(key, child.clone());
                }
                child
            }
        };
        Ok(vec![out])
    });
    meta.borrow_mut().set_str("__index", index);
    t.borrow_mut().meta = Some(meta);
    Value::Table(t)
}

/// A table whose missing fields are do-nothing functions, counted.
fn stub_table(
    vm: &mut Vm,
    name: &'static str,
    asked: Rc<RefCell<BTreeMap<String, usize>>>,
) -> Value {
    let t: TableRef = Table::new_ref();
    let meta = Table::new_ref();
    let index = vm.native("stub_index", move |vm, a| {
        let key = arg(&a, 1).to_string();
        *asked
            .borrow_mut()
            .entry(format!("{name}.{key}"))
            .or_insert(0) += 1;
        Ok(vec![vm.native("stub_fn", |_, _| Ok(vec![]))])
    });
    meta.borrow_mut().set_str("__index", index);
    t.borrow_mut().meta = Some(meta);
    Value::Table(t)
}

#[cfg(test)]
mod tests {
    use super::script_key;

    #[test]
    fn script_names_meet_modules() {
        assert_eq!(
            script_key("ui_mp/t6/hud.lua"),
            script_key("ui_mp_t6_hud.lua")
        );
        assert_eq!(
            script_key("ui_mp__t6__zombie__basezombie.lua"),
            format!("uimp{}", script_key("T6.Zombie.BaseZombie"))
        );
    }
}

#[cfg(test)]
mod host_tests {
    use super::Host;
    use crate::value::Value;

    #[test]
    fn unanswered_engine_fields_are_counted() {
        let mut h = Host::new(Vec::new(), Box::new(|_, _, _| 0.0));
        let e = h.vm.global("Engine");
        let f = h.vm.index(&e, &Value::str("SomeUnboundField")).unwrap();
        assert!(matches!(f, Value::Native(_)));
        assert_eq!(h.asked.borrow().get("Engine.SomeUnboundField"), Some(&1));
    }
}
