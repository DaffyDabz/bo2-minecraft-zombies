//! bo2zm: print a T6 zone's sound banks' room echoes (`SndRadverb`).
//! `t6echo <zone.ff>...`
fn main() {
    const FIELDS: [&str; 16] = [
        "smoothing", "earlyTime", "lateTime", "earlyGain", "lateGain", "returnGain", "earlyLpf",
        "lateLpf", "inputLpf", "dampLpf", "wallReflect", "dryGain", "earlySize", "lateSize",
        "diffusion", "returnHighpass",
    ];
    for zone in std::env::args().skip(1) {
        let c = match asset_t6::capture_zone(std::path::Path::new(&zone)) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{zone}: {e}");
                continue;
            }
        };
        for bank in &c.sound_banks {
            println!("{zone}: bank {} ({} echoes)", bank.name, bank.radverbs.len());
            for r in &bank.radverbs {
                let row: Vec<String> =
                    FIELDS.iter().zip(r.values).map(|(f, v)| format!("{f}={v}")).collect();
                println!("  {}: {}", r.name, row.join(" "));
            }
        }
    }
}
