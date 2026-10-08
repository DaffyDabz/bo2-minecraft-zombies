//! Tranzit (zm_transit): the bus is solid to players.
//!
//! The bus is a script vehicle (`veh_t6_civ_bus_zombie`) made a moving
//! platform by its scripts. Script models do not block player movement
//! here, so its body is kept as turned boxes (presence's blockers) taken
//! from the model's collision surfaces (t6props `T6PROPS_MODEL`): the
//! walls' inner faces at y +-69, the doors' panels at x 37..80 and
//! 310..353 on its right (-y), the roof at 144..152 with the hatch at
//! x 196..255, y +-29. The door gaps and the hatch stay open here: their
//! own brush models (`bus_door_blocker`, the hatch) close them while the
//! scripts keep them solid.

/// The aisle floor's top in the bus's own space (its origin is at the road).
pub(crate) const BUS_FLOOR: f32 = 40.0;

/// The inside of the bus (forward x, left y) between its walls.
pub(crate) const BUS_INSIDE: ([f32; 2], [f32; 2]) = ([-89.0, -69.0], [375.0, 69.0]);

type Box3 = ([f32; 3], [f32; 3]);

const BUS: &[Box3] = &[
    // Under the floor, but for the door wells.
    ([-96.0, -76.0, 0.0], [33.0, 76.0, BUS_FLOOR]),
    ([85.0, -76.0, 0.0], [309.0, 76.0, BUS_FLOOR]),
    ([361.0, -76.0, 0.0], [383.0, 76.0, BUS_FLOOR]),
    // The door wells: the aisle, then two steps down to the road.
    ([33.0, -50.0, 0.0], [85.0, 76.0, BUS_FLOOR]),
    ([33.0, -63.0, 0.0], [85.0, -50.0, 30.0]),
    ([33.0, -76.0, 0.0], [85.0, -63.0, 16.0]),
    ([309.0, -50.0, 0.0], [361.0, 76.0, BUS_FLOOR]),
    ([309.0, -63.0, 0.0], [361.0, -50.0, 30.0]),
    ([309.0, -76.0, 0.0], [361.0, -63.0, 16.0]),
    // The hood in front.
    ([383.0, -74.0, 0.0], [449.0, 74.0, 53.0]),
    // Left wall; right wall around the doors, and over them.
    ([-96.0, 69.0, BUS_FLOOR], [383.0, 76.0, 152.0]),
    ([-96.0, -76.0, BUS_FLOOR], [33.0, -69.0, 152.0]),
    ([85.0, -76.0, BUS_FLOOR], [309.0, -69.0, 152.0]),
    ([361.0, -76.0, BUS_FLOOR], [383.0, -69.0, 152.0]),
    ([33.0, -76.0, 120.0], [85.0, -69.0, 152.0]),
    ([309.0, -76.0, 120.0], [361.0, -69.0, 152.0]),
    // Back and front.
    ([-96.0, -69.0, BUS_FLOOR], [-89.0, 69.0, 152.0]),
    ([375.0, -69.0, BUS_FLOOR], [383.0, 69.0, 152.0]),
    // Roof around the hatch.
    ([-96.0, -76.0, 144.0], [196.0, 76.0, 152.0]),
    ([255.0, -76.0, 144.0], [383.0, 76.0, 152.0]),
    ([196.0, 29.0, 144.0], [255.0, 76.0, 152.0]),
    ([196.0, -76.0, 144.0], [255.0, -29.0, 152.0]),
];

/// A platform model's solid boxes in its own space (forward, left, up).
pub(crate) fn platform_boxes(model: &str) -> &'static [Box3] {
    match model {
        "veh_t6_civ_bus_zombie" => BUS,
        _ => &[],
    }
}
