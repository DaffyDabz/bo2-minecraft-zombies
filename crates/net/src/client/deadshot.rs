//! bo2mc: Deadshot Daiquiri's aim. In Black Ops II aiming down the sights
//! with Deadshot snaps the view to a zombie's head; here, on the press of
//! aim, the view turns to the head nearest the crosshair (within 20
//! degrees, seen) at 120 degrees a second for at most half a second, as
//! the pad's auto-aim does (`pad_aim`). Mouse movement that frame wins.

use std::sync::Mutex;

/// How far from the crosshair a head may be (degrees, at 65 fov).
const REGION: f32 = 20.0;
/// Turn speed (degrees a second) and how long the snap may take.
const SPEED: f32 = 120.0;
const TIME: f32 = 0.5;

struct State {
    was_ads: bool,
    /// The head being turned to and the time left.
    goal: Option<([f32; 3], f32)>,
}

static STATE: Mutex<State> = Mutex::new(State { was_ads: false, goal: None });

fn angles_to(eye: [f32; 3], point: [f32; 3]) -> [f32; 2] {
    let d = [point[0] - eye[0], point[1] - eye[1], point[2] - eye[2]];
    let flat = (d[0] * d[0] + d[1] * d[1]).sqrt();
    [-d[2].atan2(flat).to_degrees(), d[1].atan2(d[0]).to_degrees()]
}

fn angle_delta(to: f32, from: f32) -> f32 {
    (to - from + 540.0).rem_euclid(360.0) - 180.0
}

/// One frame: `heads` = the seen zombies' heads, `angles` = pitch, yaw.
/// Returns the view turn (pitch, yaw degrees) to add this frame.
pub fn frame(eye: [f32; 3], angles: [f32; 2], heads: &[[f32; 3]], ads: bool, fov_scale: f32, dt: f32) -> [f32; 2] {
    let mut st = STATE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if ads && !st.was_ads {
        let region = REGION * fov_scale.clamp(0.2, 2.0);
        st.goal = heads
            .iter()
            .map(|h| {
                let a = angles_to(eye, *h);
                let off = angle_delta(a[0], angles[0]).hypot(angle_delta(a[1], angles[1]));
                (*h, off)
            })
            .filter(|(_, off)| *off <= region)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(h, off)| {
                diag::info!(Fpv, "bo2mc deadshot: aiming turns to a zombie head {off:.1} degrees off ({} seen)", heads.len());
                (h, TIME)
            });
    }
    st.was_ads = ads;
    if !ads {
        st.goal = None;
    }
    let Some((head, left)) = st.goal else {
        return [0.0; 2];
    };
    let goal = angles_to(eye, head);
    let pitch = angle_delta(goal[0], angles[0]);
    let yaw = angle_delta(goal[1], angles[1]);
    let length = pitch.hypot(yaw);
    let step = (SPEED * dt).min(length);
    st.goal = (length > 0.25 && left > dt).then_some((head, left - dt));
    if length > 1e-4 {
        [pitch / length * step, yaw / length * step]
    } else {
        [0.0; 2]
    }
}
