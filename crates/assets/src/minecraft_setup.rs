//! Minecraft's own files for the Minecraft map, fetched once from Mojang's
//! official servers as a launcher fetches them: the pinned 26.3 client JAR
//! (textures, models, world generation data, loot and recipes) and, from its
//! asset index, the sounds and English text. Nothing of Mojang's ships with
//! the game. The block, item and entity catalogs MinecraftOSS's harness
//! exported ship compressed and are unpacked beside them.
//!
//! The result is laid out as a MinecraftOSS checkout is, so the world reads
//! it exactly as it reads one named by `MINECRAFTOSS_ROOT`.
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use sha1::{Digest, Sha1};

const VERSION: &str = "26.3";
/// The version's metadata, pinned by hash as Mojang's version manifest
/// lists it.
const VERSION_URL: &str =
    "https://piston-meta.mojang.com/v1/packages/bc098d111a72e9f6178801544a42099bdfbb0cf2/26.3.json";
const VERSION_SHA1: &str = "bc098d111a72e9f6178801544a42099bdfbb0cf2";
const CLIENT_SHA1: &str = "e877b6a07acd633fb3bb475002175cec036e7b87";
const OBJECTS_URL: &str = "https://resources.download.minecraft.net";
/// Parallel downloads of the asset objects.
const WORKERS: usize = 8;

const RESOURCE_PACK: &str = "resourcepacks/local/minecraft-26.3";
const DATA_PACK: &str = "datapacks/local/minecraft-26.3";
/// The JAR loot tables and recipes are read from.
pub const CLIENT_JAR: &str = "client.jar";

const CATALOGS: [(&str, &[u8]); 4] = [
    (
        "artifacts/block-state-catalog/26.3.json",
        include_bytes!("../data/minecraft/block-state-catalog-26.3.json.gz"),
    ),
    (
        "artifacts/block-entity-catalog/26.3.json",
        include_bytes!("../data/minecraft/block-entity-catalog-26.3.json.gz"),
    ),
    (
        "artifacts/entity-catalog/26.3.json",
        include_bytes!("../data/minecraft/entity-catalog-26.3.json.gz"),
    ),
    (
        "artifacts/item-catalog/26.3.json",
        include_bytes!("../data/minecraft/item-catalog-26.3.json.gz"),
    ),
];

#[derive(Clone, Debug)]
enum State {
    Idle,
    Running(String),
    Failed(String),
}

static STATE: Mutex<State> = Mutex::new(State::Idle);

fn set(state: State) {
    if let Ok(mut held) = STATE.lock() {
        *held = state;
    }
}

/// Where the files live once fetched.
pub fn cache_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_default()
        .join("iw4l-artifacts")
        .join(format!("minecraft-{VERSION}"))
}

/// The fetched files, once complete.
pub fn ready() -> Option<PathBuf> {
    let dir = cache_dir();
    dir.join(".complete").is_file().then_some(dir)
}

/// Starts fetching the files in the background, unless they are here or
/// already coming.
pub fn begin() {
    if ready().is_some() {
        return;
    }
    let Ok(mut state) = STATE.lock() else {
        return;
    };
    if matches!(*state, State::Running(_)) {
        return;
    }
    *state = State::Running("starting".into());
    drop(state);
    let spawned = std::thread::Builder::new()
        .name("minecraft-setup".into())
        .spawn(|| match run(&cache_dir()) {
            Ok(()) => {
                diag::info!(World, "Minecraft files ready in {}", cache_dir().display());
                set(State::Idle);
            }
            Err(error) => {
                diag::warn!(World, "Minecraft files could not be fetched: {error}");
                set(State::Failed(error));
            }
        });
    if let Err(error) = spawned {
        set(State::Failed(error.to_string()));
    }
}

/// Waits, up to `limit`, for a fetch under way; the files if they came.
/// Only the very first load of the Minecraft map ever waits here.
pub fn wait(limit: std::time::Duration) -> Option<PathBuf> {
    let start = std::time::Instant::now();
    while start.elapsed() < limit {
        if let Some(dir) = ready() {
            return Some(dir);
        }
        if !STATE.lock().is_ok_and(|s| matches!(*s, State::Running(_))) {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    ready()
}

/// What the fetch is doing, for a map load that has to wait for it.
pub fn status() -> String {
    match STATE.lock().map(|s| s.clone()) {
        Ok(State::Running(step)) => format!("Minecraft's files are still downloading from Mojang ({step}); try again shortly"),
        Ok(State::Failed(error)) => format!("Minecraft's files could not be downloaded from Mojang: {error}"),
        _ => "Minecraft's files are not downloaded yet".into(),
    }
}

fn step(text: impl Into<String>) {
    let text = text.into();
    diag::info!(World, "Minecraft setup: {text}");
    set(State::Running(text));
}

fn run(dir: &Path) -> Result<(), String> {
    let staging = dir.with_file_name(format!("minecraft-{VERSION}.partial"));
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;

    step("version metadata");
    let version_path = staging.join("version.json");
    fetch(VERSION_URL, &version_path, Some(VERSION_SHA1))?;
    let version: serde_json::Value = read_json(&version_path)?;
    let client_url = version["downloads"]["client"]["url"].as_str().ok_or("version has no client download")?;
    let index_url = version["assetIndex"]["url"].as_str().ok_or("version has no asset index")?;
    let index_sha1 = version["assetIndex"]["sha1"].as_str().ok_or("asset index has no hash")?;

    step("client JAR");
    let jar = staging.join(CLIENT_JAR);
    fetch(client_url, &jar, Some(CLIENT_SHA1))?;

    step("unpacking the client JAR");
    unpack_jar(&jar, &staging)?;

    step("asset index");
    let index_path = staging.join("asset-index.json");
    fetch(index_url, &index_path, Some(index_sha1))?;
    let index: serde_json::Value = read_json(&index_path)?;
    let objects = index["objects"].as_object().ok_or("asset index has no objects")?;
    // Sounds and English text: what the world plays and names. Music,
    // records and the other languages are left on Mojang's servers.
    let assets = staging.join(RESOURCE_PACK).join("assets");
    let mut wanted = Vec::new();
    for (key, object) in objects {
        let keep = key == "minecraft/sounds.json"
            || key == "minecraft/lang/en_us.json"
            || key.starts_with(TITLE_PANORAMA)
            || (key.starts_with("minecraft/sounds/")
                && !key.starts_with("minecraft/sounds/music/")
                && !key.starts_with("minecraft/sounds/records/"));
        let Some(hash) = object["hash"].as_str().filter(|_| keep) else {
            continue;
        };
        let path = assets.join(key);
        if sha1_of(&path).as_deref() != Some(hash) {
            wanted.push((hash.to_owned(), path));
        }
    }
    fetch_objects(&staging, &wanted)?;
    for (hash, path) in &wanted {
        if sha1_of(path).as_deref() != Some(hash.as_str()) {
            return Err(format!("{} did not match Mojang's hash", path.display()));
        }
    }

    step("catalogs");
    std::fs::write(
        staging.join(RESOURCE_PACK).join("pack.mcmeta"),
        r#"{"pack":{"description":"Minecraft 26.3, fetched from Mojang","min_format":[97,1],"max_format":[97,1]}}"#,
    )
    .map_err(|e| e.to_string())?;
    for (path, gz) in CATALOGS {
        let mut json = Vec::new();
        flate2::read::GzDecoder::new(gz).read_to_end(&mut json).map_err(|e| e.to_string())?;
        let out = staging.join(path);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(out, json).map_err(|e| e.to_string())?;
    }

    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&staging, dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(".complete"), VERSION).map_err(|e| e.to_string())?;
    Ok(())
}

/// The JAR's `assets/` and pack icon into the resource pack, its `data/`
/// into the data pack.
fn unpack_jar(jar: &Path, staging: &Path) -> Result<(), String> {
    let file = std::fs::File::open(jar).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        if entry.is_dir() {
            continue;
        }
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let out = if name.starts_with("assets") || name == Path::new("pack.png") {
            staging.join(RESOURCE_PACK).join(&name)
        } else if name.starts_with("data") {
            staging.join(DATA_PACK).join(&name)
        } else {
            continue;
        };
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        std::fs::write(&out, bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The asset objects, split over a few `curl` processes each fetching its
/// share over one connection.
fn fetch_objects(staging: &Path, wanted: &[(String, PathBuf)]) -> Result<(), String> {
    if wanted.is_empty() {
        return Ok(());
    }
    step(format!("{} sound and text files", wanted.len()));
    let mut children = Vec::new();
    for (worker, share) in wanted.chunks(wanted.len().div_ceil(WORKERS)).enumerate() {
        let list = staging.join(format!("objects-{worker}.curl"));
        let mut text = String::new();
        for (hash, path) in share {
            let out = path.to_string_lossy().replace('\\', "/");
            text.push_str(&format!("url = \"{OBJECTS_URL}/{}/{hash}\"\noutput = \"{out}\"\n", &hash[..2]));
        }
        std::fs::File::create(&list)
            .and_then(|mut f| f.write_all(text.as_bytes()))
            .map_err(|e| e.to_string())?;
        let child = curl()
            .args(["--fail", "--silent", "--show-error", "--location", "--retry", "3", "--create-dirs", "--config"])
            .arg(&list)
            .spawn()
            .map_err(|e| format!("could not run curl: {e}"))?;
        children.push((child, list));
    }
    let mut failed = None;
    for (mut child, list) in children {
        let status = child.wait().map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(list);
        if !status.success() {
            failed = Some(format!("curl exited with {status}"));
        }
    }
    failed.map_or(Ok(()), Err)
}

/// bo2mc: the title screen's panorama faces (the JAR holds 1x1 stand-ins;
/// the real ones are asset objects).
const TITLE_PANORAMA: &str = "minecraft/textures/gui/title/background/panorama_";

/// bo2mc: Minecraft's own title art for the front end's MINECRAFT map
/// card and loading screen, from the fetched files: panorama faces `faces`
/// (a cube face each, 1024 x 1024) and the MINECRAFT logo
/// (`gui/title/minecraft.png`), as PNG bytes. A cache made before the
/// panorama was fetched gets its faces now (Mojang's asset objects, by the
/// hashes in its asset index; a few hundred KB each). None without the
/// files (the first run fetches them) or offline.
pub fn title_art(faces: &[u32]) -> Option<(Vec<Vec<u8>>, Vec<u8>)> {
    let dir = ready()?;
    let assets = dir.join(RESOURCE_PACK).join("assets");
    let logo = std::fs::read(assets.join("minecraft/textures/gui/title/minecraft.png")).ok()?;
    let index: Option<serde_json::Value> = read_json(&dir.join("asset-index.json")).ok();
    let mut out = Vec::new();
    for face in faces {
        let key = format!("{TITLE_PANORAMA}{face}.png");
        let path = assets.join(&key);
        let hash = index
            .as_ref()
            .and_then(|i| i["objects"][&key]["hash"].as_str().map(str::to_owned));
        if let Some(hash) = &hash
            && sha1_of(&path).as_deref() != Some(hash.as_str())
        {
            let url = format!("{OBJECTS_URL}/{}/{hash}", &hash[..2]);
            if let Err(e) = fetch(&url, &path, Some(hash)) {
                diag::warn!(World, "bo2mc: Minecraft title art {key}: {e}");
                return None;
            }
        }
        out.push(std::fs::read(&path).ok()?);
    }
    Some((out, logo))
}

/// bo2mc: one of the fetched resource pack's files
/// (`minecraft/textures/block/grass_block_top.png`), once the files are in.
pub fn pack_file(key: &str) -> Option<Vec<u8>> {
    std::fs::read(ready()?.join(RESOURCE_PACK).join("assets").join(key)).ok()
}

/// One file, kept if it is already here with the right hash.
fn fetch(url: &str, path: &Path, sha1: Option<&str>) -> Result<(), String> {
    if sha1.is_some() && sha1_of(path).as_deref() == sha1 {
        return Ok(());
    }
    let status = curl()
        .args(["--fail", "--silent", "--show-error", "--location", "--retry", "3", "--create-dirs", "--output"])
        .arg(path)
        .arg(url)
        .status()
        .map_err(|e| format!("could not run curl: {e}"))?;
    if !status.success() {
        return Err(format!("downloading {url}: curl exited with {status}"));
    }
    match sha1 {
        Some(expected) if sha1_of(path).as_deref() != Some(expected) => Err(format!("{url} did not match its hash")),
        _ => Ok(()),
    }
}

fn curl() -> std::process::Command {
    let mut command = std::process::Command::new("curl");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashing up behind the game.
        command.creation_flags(0x0800_0000);
    }
    command
}

fn sha1_of(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let digest = Sha1::digest(&bytes);
    Some(digest.iter().map(|b| format!("{b:02x}")).collect())
}

fn read_json(path: &Path) -> Result<serde_json::Value, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}
