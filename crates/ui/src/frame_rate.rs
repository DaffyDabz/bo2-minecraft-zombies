//! bo2zm: two of Black Ops II's Advanced settings rows: Draw FPS (the frame
//! rate in the top right corner, as `cg_drawFPS "Simple"` shows it) and Max
//! Frames Per Second (`com_maxfps`: a frame cap, only without sync every
//! frame, as BO2 greys it out with sync on).

use std::time::{Duration, Instant};

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use frame::ClientSet;

use crate::layers::{UiLayer, UiLayerVisibility};

#[derive(Component)]
struct FpsRoot;

/// The shown rate, its smoothing, and when it was last redrawn.
#[derive(Default)]
struct FpsState {
    shown: Option<u32>,
    smooth: f32,
    drawn_at: f32,
}

fn fps_counter(
    mut commands: Commands,
    settings: Option<Res<frame::GameSettings>>,
    bo2: Option<Res<crate::bo2_font::Bo2Fonts>>,
    window: Query<&Window, With<PrimaryWindow>>,
    time: Res<Time>,
    roots: Query<Entity, With<FpsRoot>>,
    mut state: Local<FpsState>,
) {
    let on = settings.as_deref().is_some_and(|s| s.draw_fps);
    let Some(bo2) = bo2.as_deref().filter(|_| on) else {
        for e in &roots {
            commands.entity(e).despawn();
        }
        state.shown = None;
        return;
    };
    let dt = time.delta_secs();
    if dt > 0.0 {
        let fps = 1.0 / dt;
        state.smooth = if state.smooth == 0.0 {
            fps
        } else {
            state.smooth + (fps - state.smooth) * 0.1
        };
    }
    let now = time.elapsed_secs();
    let value = state.smooth.round() as u32;
    if state.shown == Some(value) || (state.shown.is_some() && now - state.drawn_at < 0.25) {
        return;
    }
    state.shown = Some(value);
    state.drawn_at = now;
    for e in &roots {
        commands.entity(e).despawn();
    }
    let sh = window.single().map_or(1080.0, |w| w.height());
    let scale = (sh / 1080.0).max(0.4);
    // Green at a good rate, yellow below 60, red below 30.
    let color = match value {
        60.. => Color::srgb(0.6, 1.0, 0.6),
        30..60 => Color::srgb(1.0, 0.9, 0.4),
        _ => Color::srgb(1.0, 0.4, 0.35),
    };
    commands
        .spawn((
            FpsRoot,
            UiLayer::Hud,
            UiLayerVisibility,
            ZIndex(50),
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(12.0 * scale),
                top: Val::Px(8.0 * scale),
                ..default()
            },
        ))
        .with_children(|c| {
            bo2.spawn_line(
                c,
                "Default",
                &[(value.to_string(), color)],
                26.0 * scale,
                2.0 * scale,
            );
        });
}

/// The frame cap: each frame waits out the rest of its share of a second
/// (a sleep, then a short spin for the last millisecond).
fn frame_cap(settings: Option<Res<frame::GameSettings>>, mut last: Local<Option<Instant>>) {
    sim::client_frame_heartbeat();
    let cap = settings
        .as_deref()
        .filter(|s| !s.vsync && s.max_fps > 0)
        .map(|s| s.max_fps);
    if let (Some(cap), Some(prev)) = (cap, *last) {
        let share = Duration::from_secs_f64(1.0 / f64::from(cap));
        let spent = prev.elapsed();
        if spent < share {
            let wait = share - spent;
            if wait > Duration::from_millis(2) {
                std::thread::sleep(wait - Duration::from_millis(1));
            }
            while prev.elapsed() < share {
                std::hint::spin_loop();
            }
        }
    }
    *last = Some(Instant::now());
}

pub(crate) fn register_frame_rate_systems(app: &mut App) {
    app.add_systems(Update, fps_counter.in_set(ClientSet::Ui))
        .add_systems(Last, frame_cap);
}
