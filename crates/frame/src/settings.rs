use bevy::prelude::Resource;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayResolution {
    pub width: u32,
    pub height: u32,
}

impl DisplayResolution {
    pub const HD: Self = Self {
        width: 1280,
        height: 720,
    };

    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

impl core::fmt::Display for DisplayResolution {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct GameSettings {
    pub resolution: DisplayResolution,
    pub fullscreen: bool,
    pub vsync: bool,
    pub fov: f32,
    pub master_volume: f32,
    /// bo2zm: Black Ops II's Sound sliders, each over the master: music,
    /// effects, voices, cinematics (0..1).
    pub music_volume: f32,
    pub sfx_volume: f32,
    pub voice_volume: f32,
    pub cinematic_volume: f32,
    pub brightness: f32,
    pub shadows: bool,
    pub depth_of_field: bool,
    pub bloom: bool,
    pub sensitivity: f32,
    pub invert_mouse: bool,
    pub player_name: String,

    pub pad_layout: u8,
    pub pad_stick_layout: u8,
    pub pad_sensitivity_preset: u8,
    pub pad_custom_sensitivity: f32,
    pub pad_ads_sensitivity: f32,
    pub pad_invert: bool,
    pub pad_curve: u8,
    pub pad_acceleration: bool,
    pub pad_aim_assist: u8,
    pub pad_prompts: u8,
    pub pad_vibration: bool,
    pub pad_deadzone_left: f32,
    pub pad_deadzone_right: f32,

    /// bo2zm: `bo2_quality=fast` (Black Ops II maps: half-size textures,
    /// single-layer ground, props hidden sooner); `full` by default.
    pub bo2_fast: bool,

    /// bo2zm: more of Black Ops II's Settings rows: mature content (heads
    /// and limbs come off), how many corpses stay (3, 5, 10, 16), the fps
    /// counter, the frame cap (0 = none; only without sync), FXAA, the
    /// anti-aliasing row (1 = off), texture filtering (0 low .. 2 high) and
    /// texture quality (-1 automatic, 3 low .. 0 extra; low = `bo2_fast`).
    pub mature: bool,
    pub corpses: u8,
    pub draw_fps: bool,
    pub max_fps: u16,
    pub fxaa: bool,
    pub aa_samples: u8,
    pub tex_filter: u8,
    pub tex_quality: i8,

    pub revision: u64,
}

impl Default for GameSettings {
    fn default() -> Self {
        Self {
            resolution: DisplayResolution::HD,
            fullscreen: false,
            vsync: true,
            fov: Self::FOV_DEFAULT,
            master_volume: 1.0,
            music_volume: 1.0,
            sfx_volume: 1.0,
            voice_volume: 1.0,
            cinematic_volume: 1.0,
            brightness: 0.0,
            shadows: true,
            depth_of_field: true,
            bloom: true,
            sensitivity: 5.0,
            invert_mouse: false,
            player_name: "Player".to_owned(),
            pad_layout: 0,
            pad_stick_layout: 0,
            pad_sensitivity_preset: 0,
            pad_custom_sensitivity: 1.0,
            pad_ads_sensitivity: 1.0,
            pad_invert: false,
            pad_curve: 0,
            pad_acceleration: true,
            pad_aim_assist: 0,
            pad_prompts: 0,
            pad_vibration: true,
            pad_deadzone_left: 0.12,
            pad_deadzone_right: 0.12,
            bo2_fast: false,
            mature: true,
            corpses: 16,
            draw_fps: false,
            max_fps: 0,
            fxaa: false,
            aa_samples: 1,
            tex_filter: 2,
            tex_quality: -1,
            revision: 0,
        }
    }
}

impl GameSettings {
    pub const FOV_DEFAULT: f32 = 65.0;
    pub const FOV_MIN: f32 = 65.0;
    pub const FOV_MAX: f32 = 120.0;
    pub const PAD_SENSITIVITY_PRESETS: [f32; 10] =
        [0.6, 1.0, 1.4, 1.8, 2.0, 2.2, 2.6, 3.0, 3.5, 4.0];

    pub fn pad_look_sensitivity(&self) -> f32 {
        self.pad_sensitivity_preset
            .checked_sub(1)
            .and_then(|index| Self::PAD_SENSITIVITY_PRESETS.get(usize::from(index)))
            .copied()
            .unwrap_or(self.pad_custom_sensitivity)
    }

    pub const PAD_LAYOUT_CUSTOM: u8 = 255;

    pub fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn sanitize(&mut self) {
        self.resolution.width = self.resolution.width.clamp(640, 7680);
        self.resolution.height = self.resolution.height.clamp(480, 4320);
        self.fov = if self.fov.is_finite() {
            self.fov.clamp(Self::FOV_MIN, Self::FOV_MAX)
        } else {
            Self::FOV_DEFAULT
        };
        self.brightness = if self.brightness.is_finite() {
            self.brightness.clamp(-0.2, 0.2)
        } else {
            0.0
        };
        self.master_volume = self.master_volume.clamp(0.0, 1.0);
        for v in [
            &mut self.music_volume,
            &mut self.sfx_volume,
            &mut self.voice_volume,
            &mut self.cinematic_volume,
        ] {
            *v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 1.0 };
        }
        self.sensitivity = self.sensitivity.clamp(0.1, 30.0);
        if self.pad_layout != Self::PAD_LAYOUT_CUSTOM {
            self.pad_layout = self.pad_layout.min(4);
        }
        self.pad_stick_layout = self.pad_stick_layout.min(3);
        self.pad_curve = self.pad_curve.min(2);
        self.pad_aim_assist = 0;
        self.pad_prompts = self.pad_prompts.min(3);
        let finite = |v: f32, lo: f32, hi: f32, default: f32| {
            if v.is_finite() {
                v.clamp(lo, hi)
            } else {
                default
            }
        };
        self.pad_sensitivity_preset = self.pad_sensitivity_preset.min(10);
        self.pad_custom_sensitivity = finite(self.pad_custom_sensitivity, 0.1, 5.0, 1.0);
        self.pad_ads_sensitivity = finite(self.pad_ads_sensitivity, 0.5, 1.5, 1.0);
        if ![3, 5, 10, 16].contains(&self.corpses) {
            self.corpses = 16;
        }
        self.max_fps = self.max_fps.min(1000);
        if ![1, 2, 4, 8, 16, 17, 18].contains(&self.aa_samples) {
            self.aa_samples = 1;
        }
        self.tex_filter = self.tex_filter.min(2);
        self.tex_quality = self.tex_quality.clamp(-1, 3);
        self.pad_deadzone_left = finite(self.pad_deadzone_left, 0.0, 0.4, 0.12);
        self.pad_deadzone_right = finite(self.pad_deadzone_right, 0.0, 0.4, 0.12);
        self.player_name = self.player_name.trim().chars().take(16).collect();
        if self.player_name.is_empty() {
            self.player_name = "Player".to_owned();
        }
    }
}
