//! bo2mc: Vulture Aid's sight. With Vulture Aid, Black Ops II shows the
//! perk machines and the Mystery Box through walls (its client script
//! glows each in the perk's colour). Here the server lists them for his
//! client (`bo2mc_vulture`: "x y z material;..." in map units), and each
//! draws as its perk's icon over the scene where it stands, through
//! anything in between; hidden when it is close (he sees it then) or
//! behind him.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use frame::ClientSet;
use net::{LocalPresentClient, PresentedSnapshot};

use crate::layers::{UiLayer, UiLayerVisibility};

#[derive(Component)]
struct VultureRoot;

#[derive(Component)]
struct VultureMark(usize);

/// Closer than this (map units) a mark hides.
const NEAR: f32 = 160.0;
/// Icon size at 1080 lines.
const SIZE: f32 = 40.0;

fn parse(s: &str) -> Vec<([f32; 3], String)> {
    s.split(';')
        .filter_map(|e| {
            let mut it = e.split_whitespace();
            let x = it.next()?.parse().ok()?;
            let y = it.next()?.parse().ok()?;
            let z = it.next()?.parse().ok()?;
            Some(([x, y, z], it.next()?.to_owned()))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn vulture_marks(
    mut commands: Commands,
    presented: Option<Res<PresentedSnapshot>>,
    local: Option<Res<LocalPresentClient>>,
    icons: Option<Res<assets::T6HudIcons>>,
    mut images: ResMut<Assets<Image>>,
    window: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    roots: Query<Entity, With<VultureRoot>>,
    mut marks: Query<(&VultureMark, &mut Node, &mut Visibility)>,
    mut handles: Local<Vec<(String, Handle<Image>)>>,
    mut shown: Local<String>,
) {
    let value = match (presented.as_deref(), local.as_deref()) {
        (Some(p), Some(l)) => p
            .snapshot()
            .and_then(|snap| snap.meta.script_dvars(l.0).string("bo2mc_vulture").map(str::to_owned)),
        _ => None,
    }
    .unwrap_or_default();
    if value.is_empty() {
        for e in &roots {
            commands.entity(e).despawn();
        }
        shown.clear();
        return;
    }
    if handles.is_empty()
        && let Some(icons) = icons.as_deref()
    {
        for (name, image) in &icons.0 {
            handles.push((name.clone(), images.add((**image).clone())));
        }
    }
    let list = parse(&value);
    let scale = window.single().map_or(1.0, |w| (w.height() / 1080.0).max(0.4));
    if *shown != value || roots.is_empty() {
        *shown = value.clone();
        for e in &roots {
            commands.entity(e).despawn();
        }
        let handle = |m: &str| handles.iter().find(|(n, _)| n == m).map(|(_, h)| h.clone());
        commands
            .spawn((
                VultureRoot,
                UiLayer::Hud,
                UiLayerVisibility,
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
            ))
            .with_children(|p| {
                for (i, (_, material)) in list.iter().enumerate() {
                    let Some(h) = handle(material) else { continue };
                    p.spawn((
                        VultureMark(i),
                        ImageNode::new(h).with_color(Color::srgba(1.0, 1.0, 1.0, 0.75)),
                        Node {
                            position_type: PositionType::Absolute,
                            width: Val::Px(SIZE * scale),
                            height: Val::Px(SIZE * scale),
                            ..default()
                        },
                        Visibility::Hidden,
                    ));
                }
            });
        return;
    }
    let Some((camera, transform)) = cameras.iter().find(|(c, _)| c.is_active) else {
        return;
    };
    let eye = transform.translation();
    let forward = *transform.forward();
    let half = SIZE * scale * 0.5;
    for (mark, mut node, mut vis) in &mut marks {
        let Some((at, _)) = list.get(mark.0) else { continue };
        let world = Vec3::from_array(*at);
        let ahead = (world - eye).dot(forward);
        let place = (ahead > 0.0 && world.distance(eye) > NEAR)
            .then(|| camera.world_to_viewport(transform, world).ok())
            .flatten();
        match place {
            Some(p) => {
                node.left = Val::Px(p.x - half);
                node.top = Val::Px(p.y - half);
                *vis = Visibility::Inherited;
            }
            None => *vis = Visibility::Hidden,
        }
    }
}

pub(crate) fn register(app: &mut App) {
    app.add_systems(Update, vulture_marks.in_set(ClientSet::Ui));
}

#[cfg(test)]
mod tests {
    #[test]
    fn marks_parse() {
        let l = super::parse("1 2 3 specialty_vulture_zombies;-4.5 0 60 x;bad");
        assert_eq!(l.len(), 2);
        assert_eq!(l[1].0, [-4.5, 0.0, 60.0]);
    }
}
