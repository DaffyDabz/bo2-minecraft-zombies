//! bo2mc: Black Ops II Zombies perks the engine itself carries out, as
//! bits of `PlayerState::perks[0]` beside Modern Warfare 2's (networked
//! with them, so the client predicts the same). The scripts give the perk
//! (`setperk("specialty_rof")`); `sim::script_player::perk_bits` sets the
//! bit.

/// Double Tap Root Beer II (`specialty_rof`): fires 33% faster, and every
/// shot fires a second bullet (`sim::combat::phase_emit`).
pub const PERK_BO2_ROF: u32 = 1 << 29;

/// Stamin-Up (`specialty_longersprint`): sprints twice as long and moves
/// 7% faster (`sim::step`).
pub const PERK_BO2_LONGERSPRINT: u32 = 1 << 30;

/// Deadshot Daiquiri (`specialty_deadshot`): aiming down the sights snaps
/// to the nearest zombie's head (`net::client::deadshot`); with it the hip
/// spread is Modern Warfare 2's Steady Aim (`PERK_BULLETACCURACY`, x0.65).
pub const PERK_BO2_DEADSHOT: u32 = 1 << 31;

/// Double Tap II's fire time: 1 / 1.33 of the gun's.
#[must_use]
pub fn rof_fire_time_ms(perks0: u32, fire_time_ms: i32) -> i32 {
    if perks0 & PERK_BO2_ROF != 0 && fire_time_ms > 1 {
        ((fire_time_ms * 100 + 66) / 133).max(1)
    } else {
        fire_time_ms
    }
}

/// Stamin-Up's sprint time (ms) and move speed scale.
pub const LONGERSPRINT_SPRINT_SCALE: i32 = 2;
pub const LONGERSPRINT_MOVE_SCALE: f32 = 1.07;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_tap_fires_a_third_faster() {
        assert_eq!(rof_fire_time_ms(PERK_BO2_ROF, 133), 100);
        assert_eq!(rof_fire_time_ms(PERK_BO2_ROF, 160), 120);
        assert_eq!(rof_fire_time_ms(0, 160), 160);
        assert_eq!(rof_fire_time_ms(PERK_BO2_ROF, 1), 1);
    }
}
