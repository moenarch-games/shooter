//! The arena kit (release `shooter-arena`, `scripts/fetch-robot-kit.sh` puts it in
//! `target/arena-kit`): the simulation's checked-in collision table (`shooter_sim::arena_kit`,
//! written by `scripts/gen-arena-kit.py`) is the pinned kit's descriptor, the kit decodes for
//! every module the table names, the browser's tar holds the same files, and every module of a
//! kit-built map is drawn inside the placement box its collision is derived in.

use serde_json::Value as Json;
use shooter_client::arena_art::transform;
use shooter_client::robots::kit::{Bundle, Plan, pinned_manifest, pinned_tar, sha256};
use shooter_client::robots::load;
use shooter_sim::MapId;
use shooter_sim::arena_kit::{Module, Proxy, Shape, modules::KIT_MANIFEST_SHA256};
use shooter_sim::physics::RampRise;

fn read(path: &std::path::Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|error| {
        panic!(
            "{}: {error} (fetch the kits with scripts/fetch-robot-kit.sh)",
            path.display()
        )
    })
}

/// The pinned kit's descriptor (`arena-descriptor`, attached to `arena.floor`).
fn descriptor() -> Json {
    let directory = load::directory(Bundle::Arena);
    let manifest = read(&directory.join("manifest.json"));
    assert_eq!(sha256(&manifest), pinned_manifest(Bundle::Arena));
    let manifest: Json = serde_json::from_slice(&manifest).unwrap();
    let attachment = manifest["assets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|asset| asset["key"] == "arena.floor" && asset["variant"] == "default")
        .flat_map(|asset| asset["attachments"].as_array().unwrap())
        .find(|attachment| attachment["name"] == "arena-descriptor")
        .expect("the descriptor attachment");
    let bytes = read(&directory.join(attachment["path"].as_str().unwrap()));
    assert_eq!(sha256(&bytes), attachment["sha256"].as_str().unwrap());
    serde_json::from_slice(&bytes).unwrap()
}

/// Metres to whole centimetres, failing on a fraction.
fn centimetres(value: &Json, scale: f64) -> i32 {
    let metres = value.as_f64().unwrap() * scale;
    let rounded = metres.round();
    assert!((metres - rounded).abs() < 1e-6, "{value} is not whole");
    #[allow(clippy::cast_possible_truncation)]
    let whole = rounded as i32;
    whole
}

fn triple(value: &Json, scale: f64) -> [i32; 3] {
    let axes = value.as_array().unwrap();
    [0, 1, 2].map(|axis| centimetres(&axes[axis], scale))
}

fn proxies(list: &Json) -> Vec<Proxy> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|proxy| Proxy {
            name: Box::leak(proxy["name"].as_str().unwrap().to_owned().into_boxed_str()),
            center: triple(&proxy["center"], 100.0),
            half: triple(&proxy["size"], 50.0),
            shape: match (proxy["shape"].as_str(), proxy["tag"].as_str()) {
                (Some("ramp"), _) => Shape::Ramp(match proxy["rises"].as_str().unwrap() {
                    "+X" => RampRise::PosX,
                    "-X" => RampRise::NegX,
                    "+Z" => RampRise::PosZ,
                    "-Z" => RampRise::NegZ,
                    other => panic!("rises {other}"),
                }),
                (_, Some("walkable")) => Shape::Walkable,
                (_, Some("blocked")) => Shape::Blocked,
                (_, Some("trigger")) => Shape::Trigger,
                other => panic!("proxy {other:?}"),
            },
        })
        .collect()
}

#[test]
fn the_collision_table_is_the_pinned_kits_descriptor() {
    assert_eq!(KIT_MANIFEST_SHA256, pinned_manifest(Bundle::Arena));
    let descriptor = descriptor();
    assert_eq!(descriptor["schema"], "arena-kit-descriptor-v1");
    assert_eq!(descriptor["units"], "metre");
    let assets = descriptor["assets"].as_array().unwrap();
    let keys: Vec<&str> = assets.iter().map(|a| a["key"].as_str().unwrap()).collect();
    let table: Vec<&str> = Module::ALL.iter().map(|module| module.key()).collect();
    assert_eq!(
        keys, table,
        "the table's modules, in the descriptor's order"
    );
    for (asset, module) in assets.iter().zip(Module::ALL) {
        let key = module.key();
        assert_eq!(
            module.placement_box(),
            [
                triple(&asset["box"]["min"], 100.0),
                triple(&asset["box"]["max"], 100.0)
            ],
            "{key} box"
        );
        assert_eq!(
            module.proxies(),
            proxies(&asset["variants"]["default"]["collisionProxies"]),
            "{key} proxies"
        );
        let broken = asset["variants"]
            .get("broken")
            .map(|broken| proxies(&broken["collisionProxies"]));
        assert_eq!(
            module.broken_proxies().map(<[Proxy]>::to_vec),
            broken,
            "{key} broken proxies"
        );
    }
    // The kit's jump pad is Shooter's 1.6 m pad.
    let pad = Module::JumpPad.proxies()[0];
    assert_eq!(
        (pad.shape, pad.half[0], pad.half[2]),
        (Shape::Trigger, 80, 80)
    );
}

#[test]
fn the_arena_kit_decodes_every_module_and_the_browser_tar_holds_it() {
    let kit = load::read_arena().unwrap();
    for module in Module::ALL {
        let intact = kit
            .entry(module.key(), "default")
            .unwrap_or_else(|| panic!("{}", module.key()));
        assert!(!kit.entries[intact].mesh.positions.is_empty());
        assert_eq!(
            kit.entry(module.key(), "broken").is_some(),
            module.broken_proxies().is_some(),
            "{}",
            module.key()
        );
    }
    // One material for every module, as the kit promises.
    assert_eq!(kit.materials.len(), 1);
    // The door's two leaves are baked into one mesh.
    let door = &kit.entries[kit.entry("arena.door", "default").unwrap()].mesh;
    let xs = door.positions.iter().map(|p| p.x);
    let (low, high) = xs.fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
    assert!(low < -0.9 && high > 0.9, "{low} {high}");

    // What the browser does with `arena-kit/kit.tar`, natively.
    let directory = load::directory(Bundle::Arena);
    let tar = read(&directory.join("kit.tar"));
    assert!(tar.len() < load::MAX_TAR_BYTES);
    assert_eq!(sha256(&tar), pinned_tar(Bundle::Arena));
    let mut entries = load::untar(&tar).unwrap();
    let plan = Plan::of(Bundle::Arena, &entries.remove("manifest.json").unwrap()).unwrap();
    let (mut total, mut files) = (0, load::Files::new());
    load::take(&plan, &mut entries, &mut total, &mut files).unwrap();
    let from_tar = load::finish(&[plan], &files).unwrap();
    assert_eq!(from_tar.entries.len(), kit.entries.len());
}

/// Every module of every kit-built map is drawn inside its placement box in the arena: the box
/// its collision pieces are derived in. What a fighter sees is where it collides.
#[test]
fn kit_built_maps_draw_each_module_inside_its_placement_box() {
    let kit = load::read_arena().unwrap();
    let mut checked = 0;
    for map in MapId::ALL {
        for placement in &map.arena().placements {
            let entry = kit
                .entry(placement.module.key(), placement.variant.bundle())
                .unwrap();
            let [min, max] = placement.bounds();
            #[allow(clippy::cast_precision_loss)]
            let (min, max) = (
                [min.x, min.y, min.z].map(|v| v as f32 * 0.01 - 0.011),
                [max.x, max.y, max.z].map(|v| v as f32 * 0.01 + 0.011),
            );
            let placed = transform(placement);
            for position in &kit.entries[entry].mesh.positions {
                let p = placed.transform_point3(*position).to_array();
                assert!(
                    (0..3).all(|axis| min[axis] <= p[axis] && p[axis] <= max[axis]),
                    "{map}: {placement:?} draws a vertex at {p:?} outside {min:?}..{max:?}"
                );
            }
            checked += 1;
        }
    }
    assert!(checked > 100);
}
