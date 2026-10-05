//! bo2zm: where Black Ops II keeps a HUD icon's pixels, read-only.
//! `t6icon <material>...` looks each material up in Nuketown's zones
//! (common_zm, zm_nuked, patch_zm) and prints its colour map's image: size,
//! levels, and whether its pixels sit in an image pack or in the zone.

use std::path::Path;
use std::process::ExitCode;

use asset_t6::{ImageSource, PackSet, capture_zone};

const ZONES: &str = r"<Black Ops II folder>\zone\all";

fn main() -> ExitCode {
    let names: Vec<String> = std::env::args().skip(1).collect();
    let dir = Path::new(ZONES);
    let packs = match PackSet::open_dir(dir) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("packs: {e}");
            return ExitCode::FAILURE;
        }
    };
    for zone in ["common_zm", "zm_nuked", "patch_zm"] {
        let capture = match capture_zone(&dir.join(format!("{zone}.ff"))) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{zone}: {e}");
                continue;
            }
        };
        for m in capture
            .materials
            .iter()
            .filter(|m| names.iter().any(|n| n == &m.name))
        {
            let techniques = m
                .technique_set
                .and_then(|k| capture.technique_sets.get(k.index))
                .map_or("", |t| t.name.as_str());
            println!("{zone} {} technique {techniques}", m.name);
            for t in &m.textures {
                let Some(img) = t.image.and_then(|k| capture.images.get(k.index)) else {
                    continue;
                };
                let source = match packs.locate(img) {
                    Some(ImageSource::Pack(i, _)) => format!("pack {i}"),
                    Some(ImageSource::Embedded) => {
                        let e = img.embedded.as_ref().unwrap();
                        format!(
                            "zone dxgi {} levels {} {} bytes",
                            e.dxgi_format,
                            e.level_count,
                            e.data.len()
                        )
                    }
                    Some(ImageSource::Empty) => "empty".to_owned(),
                    None => "nowhere".to_owned(),
                };
                println!(
                    "  slot {:08x} semantic {} image {} {}x{} levels {} streamed {} -> {source}",
                    t.name_hash,
                    t.semantic,
                    img.name,
                    img.width,
                    img.height,
                    img.level_count,
                    img.streamed_parts
                );
            }
        }
    }
    ExitCode::SUCCESS
}
