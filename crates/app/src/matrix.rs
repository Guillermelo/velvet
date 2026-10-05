use super::*;

const FLUID_TIME_RATE: f64 = 0.09375;
const FLUID_PITCH: f32 = 10.0;
const FLUID_SCALE: Vec2 = Vec2::new(3.2, 2.4);
const FLUID_BAND_WIDTH: f32 = 0.006;
const FLUID_BAND_SOFTNESS: f32 = 0.024;
const FLUID_COLOR_INTENSITY: f32 = 0.85;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum MatrixPreset {
    CrossingWaves,
    #[default]
    FluidGrid,
}
impl MatrixPreset {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::CrossingWaves => "Crossing Waves",
            Self::FluidGrid => "Fluid Grid",
        }
    }
}

#[derive(Clone)]
pub(super) struct MatrixView {
    pub(super) origin: Pos2,
    pub(super) scale_x: f32,
    pub(super) row_height: f32,
    pub(super) track_heights: Vec<f32>,
}
impl Default for MatrixView {
    fn default() -> Self {
        Self {
            origin: Pos2::ZERO,
            scale_x: 1.0,
            row_height: 81.0,
            track_heights: Vec::new(),
        }
    }
}
impl MatrixView {
    fn columns(&self, rect: Rect, pitch: f32, arrangement: bool) -> (f32, f32, i32, i32) {
        let (pitch, offset) = if arrangement {
            (
                22.0 / musical_grid_divisions(self.scale_x * 22.0) as f32,
                0.0,
            )
        } else {
            (pitch, 0.5)
        };
        let first = (((rect.left() - self.origin.x) / self.scale_x) / pitch - offset).ceil() as i32;
        let last =
            (((rect.right() - self.origin.x) / self.scale_x) / pitch - offset).floor() as i32;
        (pitch, offset, first, last)
    }
    fn screen_y(&self, world_y: f32) -> f32 {
        if world_y < 0.0 {
            return self.origin.y + world_y;
        }
        let mut remaining = world_y;
        let mut screen = self.origin.y;
        for &height in &self.track_heights {
            if remaining < self.row_height {
                return screen + remaining * height / self.row_height;
            }
            remaining -= self.row_height;
            screen += height;
        }
        screen + remaining
    }
    fn world_y(&self, screen_y: f32) -> f32 {
        let mut remaining = screen_y - self.origin.y;
        if remaining < 0.0 {
            return remaining;
        }
        let mut world = 0.0;
        for &height in &self.track_heights {
            if remaining < height {
                return world + remaining * self.row_height / height;
            }
            remaining -= height;
            world += self.row_height;
        }
        world + remaining
    }
}

#[derive(Default)]
struct Particle {
    offset: Vec2,
    velocity: Vec2,
}
impl Particle {
    fn step(&mut self, base: Pos2, pointer: Option<Pos2>, dt: f32) -> f32 {
        let (target, light) = pointer
            .map(|pointer| {
                let delta = base - pointer;
                let distance = delta.length();
                let light = (-distance * distance / (130.0 * 130.0)).exp();
                (delta / distance.max(1.0) * (22.0 * light), light)
            })
            .unwrap_or((Vec2::ZERO, 0.0));
        self.velocity += ((target - self.offset) * 110.0 - self.velocity * 18.0) * dt;
        self.offset += self.velocity * dt;
        light
    }
}
#[derive(Default)]
pub(super) struct ParticleMatrix {
    pub(super) preset: MatrixPreset,
    pub(super) view: Option<MatrixView>,
    field_size: Option<Vec2>,
    particles: Vec<Particle>,
    columns: usize,
    dot_texture: Option<egui::TextureHandle>,
    bloom_texture: Option<egui::TextureHandle>,
    field_time: f64,
    last_time: Option<f64>,
    tempo_speed: Option<f64>,
    hover_pos: Option<Pos2>,
    hover_glow: f32,
    white_wave_opacity: f32,
}
impl ParticleMatrix {
    pub(super) fn paint(
        &mut self,
        ctx: &egui::Context,
        rect: Rect,
        bpm: f64,
        playback_seconds: Option<f64>,
    ) -> Vec<egui::Shape> {
        let now = ctx.input(|i| i.time);
        let dt = self
            .last_time
            .map_or(0.0, |last| (now - last).clamp(0.0, 0.1));
        self.last_time = Some(now);
        if self.preset == MatrixPreset::FluidGrid {
            // Tempo has a gentle influence; changing BPM never jumps the field's phase.
            let target_speed = (bpm / 120.0).powf(0.35).clamp(0.65, 1.5);
            let speed = self.tempo_speed.get_or_insert(target_speed);
            *speed += (target_speed - *speed) * (1.0 - (-dt * 4.0).exp());
            self.field_time += dt * FLUID_TIME_RATE * *speed;
            self.white_wave_opacity = first_beat_envelope(bpm, playback_seconds);
            let pointer = ctx.input(|i| i.pointer.hover_pos().filter(|p| rect.contains(*p)));
            if let Some(pointer) = pointer {
                self.hover_pos = Some(pointer);
            }
            let target_glow = if pointer.is_some() { 1.0 } else { 0.0 };
            self.hover_glow += (target_glow - self.hover_glow) * (1.0 - (-dt as f32 * 12.0).exp());
            if self.hover_glow < 0.001 && pointer.is_none() {
                self.hover_pos = None;
            }
            self.paint_fluid(ctx, rect)
        } else {
            self.paint_crossing_waves(ctx, rect)
        }
    }

    fn paint_fluid(&mut self, ctx: &egui::Context, rect: Rect) -> Vec<egui::Shape> {
        const PITCH: f32 = FLUID_PITCH;
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return Vec::new();
        }
        let view = self.view.clone().unwrap_or(MatrixView {
            origin: rect.min,
            ..Default::default()
        });
        let field_size = *self.field_size.get_or_insert(rect.size());
        let (pitch_x, offset_x, first_column, last_column) =
            view.columns(rect, PITCH, self.view.is_some());
        let first_row = (view.world_y(rect.top()) / PITCH - 0.5).ceil() as i32;
        let last_row = (view.world_y(rect.bottom()) / PITCH - 0.5).floor() as i32;
        let columns = (last_column - first_column + 1).max(0) as usize;
        let rows = (last_row - first_row + 1).max(0) as usize;
        if columns == 0 || rows == 0 {
            return Vec::new();
        }
        let texture = self.dot_texture.get_or_insert_with(|| dot_texture(ctx));
        let bloom = self.bloom_texture.get_or_insert_with(|| {
            let mut image = egui::ColorImage::new([32, 32], Color32::TRANSPARENT);
            for y in 0..32 {
                for x in 0..32 {
                    let r = Vec2::new(x as f32 + 0.5 - 16.0, y as f32 + 0.5 - 16.0).length() / 16.0;
                    let alpha =
                        ((-r * r * 6.0).exp() * (1.0 - smoothstep(0.7, 1.0, r)) * 255.0) as u8;
                    image[(x, y)] = Color32::from_white_alpha(alpha);
                }
            }
            ctx.load_texture("matrix_bloom", image, egui::TextureOptions::LINEAR)
        });
        let mut mesh = egui::Mesh::with_texture(texture.id());
        let mut glow = egui::Mesh::with_texture(bloom.id());
        mesh.vertices.reserve(columns * rows * 4);
        mesh.indices.reserve(columns * rows * 6);
        for row in first_row..=last_row {
            let world_y = (row as f32 + 0.5) * PITCH;
            let screen_y = view.screen_y(world_y);
            for column in first_column..=last_column {
                let world_x = (column as f32 + offset_x) * pitch_x;
                let base = Pos2::new(view.origin.x + world_x * view.scale_x, screen_y);
                let u = world_x / field_size.x;
                let v = world_y / field_size.y;
                let (pink, cyan) = fluid_field(u, v, self.field_time as f32);
                let strength = pink.max(cyan);
                let hover = self.hover_pos.map_or(0.0, |pointer| {
                    (-(base - pointer).length_sq() / (90.0 * 90.0)).exp() * self.hover_glow
                });
                let background = 1.0 - strength;
                let wave = if self.white_wave_opacity > 0.0 {
                    let (a, b) = fluid_field(u + 2.7, v + 1.9, self.field_time as f32);
                    a.max(b) * self.white_wave_opacity
                } else {
                    0.0
                };
                let radius = 0.60
                    + strength * 1.05
                    + strength.powi(5) * 0.40
                    + hover * (0.08 + background * 0.06);
                let rgb = fluid_color(pink, cyan).map(|channel| {
                    mix(
                        channel as f32,
                        250.0,
                        (hover * (0.10 + background * 0.15) + wave * 0.75).min(1.0),
                    ) as u8
                });
                let alpha = (0.15
                    + strength * 0.60
                    + strength.powi(4) * 0.15
                    + hover * (0.085 + background * 0.095)
                    + wave * 0.22)
                    .min(0.96);
                let color =
                    Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], (alpha * 255.0) as u8);
                let bloom_strength = smoothstep(0.92, 1.0, strength);
                if bloom_strength > 0.0 {
                    dot(
                        &mut glow,
                        base,
                        radius * 2.0,
                        Color32::from_rgba_unmultiplied(
                            rgb[0],
                            rgb[1],
                            rgb[2],
                            (bloom_strength * 10.0) as u8,
                        ),
                    );
                }
                dot(&mut mesh, base, radius, color);
            }
        }
        vec![egui::Shape::mesh(glow), egui::Shape::mesh(mesh)]
    }

    fn paint_crossing_waves(&mut self, ctx: &egui::Context, rect: Rect) -> Vec<egui::Shape> {
        let pitch = 7.0;
        if !rect.is_positive() {
            return Vec::new();
        }
        let arrangement = self.view.is_some();
        let view = self.view.clone().unwrap_or(MatrixView {
            origin: rect.min,
            ..Default::default()
        });
        let field_size = *self.field_size.get_or_insert(rect.size());
        let (pitch_x, offset_x, first_column, last_column) = view.columns(rect, pitch, arrangement);
        let first_row = (view.world_y(rect.top()) / pitch - 0.5).ceil() as i32;
        let last_row = (view.world_y(rect.bottom()) / pitch - 0.5).floor() as i32;
        let columns = (last_column - first_column + 1).max(0) as usize;
        let rows = (last_row - first_row + 1).max(0) as usize;
        if columns == 0 || rows == 0 {
            return Vec::new();
        }
        if self.columns != columns || self.particles.len() != columns * rows {
            self.columns = columns;
            self.particles = (0..columns * rows).map(|_| Particle::default()).collect();
        }
        let (pointer, time, dt) = ctx.input(|i| {
            (
                i.pointer.hover_pos().filter(|p| rect.contains(*p)),
                i.time as f32,
                i.stable_dt.clamp(0.0, 1.0 / 30.0),
            )
        });
        let texture = self.dot_texture.get_or_insert_with(|| dot_texture(ctx));
        let mut mesh = egui::Mesh::with_texture(texture.id());
        mesh.vertices.reserve(self.particles.len() * 4);
        mesh.indices.reserve(self.particles.len() * 6);
        for (i, particle) in self.particles.iter_mut().enumerate() {
            let world_x = (first_column as f32 + (i % columns) as f32 + offset_x) * pitch_x;
            let world_y = (first_row as f32 + (i / columns) as f32 + 0.5) * pitch;
            let base = Pos2::new(
                view.origin.x + world_x * view.scale_x,
                view.screen_y(world_y),
            );
            let u = world_x / field_size.x;
            let v = world_y / field_size.y;
            let cyan_path = 0.77 - 0.40 * u + 0.065 * (u * 8.0 - time * 1.1).sin();
            let rose_path = 0.39 + 0.43 * u + 0.060 * (u * 7.0 + time * 0.95).cos();
            let width = 0.045 + 0.012 * (u * 5.0 + time * 0.65).sin();
            let cyan = (-(v - cyan_path).powi(2) / (width * width)).exp();
            let rose = (-(v - rose_path).powi(2) / (width * width)).exp();
            // Max, rather than a sum, keeps the crossing light and open.
            let strength = cyan.max(rose);
            let total = (cyan + rose).max(0.0001);
            let overlap = cyan.min(rose);
            let light = particle.step(base, pointer, dt);
            let color_channel = |neutral: f32, blue: f32, pink: f32, violet: f32| {
                let mixed = (blue * cyan + pink * rose) / total;
                let wave = mixed * (1.0 - overlap) + violet * overlap;
                let lit = neutral + (wave - neutral) * strength;
                (lit + (blue - lit) * light * 0.35) as u8
            };
            let alpha = (46.0 + strength * 152.0 + light * 54.0).min(235.0) as u8;
            let radius =
                (0.28 + strength * (0.52 + 0.10 * (u * 12.0 - time * 0.9).sin()) + light * 0.15)
                    .clamp(0.28, 1.1);
            let color = Color32::from_rgba_unmultiplied(
                color_channel(205.0, 56.0, 243.0, 158.0),
                color_channel(205.0, 189.0, 134.0, 102.0),
                color_channel(205.0, 248.0, 161.0, 246.0),
                alpha,
            );
            dot(
                &mut mesh,
                base + if arrangement {
                    Vec2::ZERO
                } else {
                    particle.offset
                },
                radius,
                color,
            );
        }
        let mut shapes = vec![egui::Shape::mesh(mesh)];
        for x in [0.05, 0.36, 0.67, 0.95] {
            let world_x = if arrangement {
                (field_size.x * x / pitch_x).round() * pitch_x
            } else {
                field_size.x * x
            };
            let center = Pos2::new(
                view.origin.x + world_x * view.scale_x,
                view.screen_y(field_size.y * 0.57),
            );
            shapes.push(egui::Shape::line_segment(
                [center - Vec2::new(4.0, 0.0), center + Vec2::new(4.0, 0.0)],
                Stroke::new(0.5_f32, ROSE.gamma_multiply(0.35)),
            ));
            shapes.push(egui::Shape::line_segment(
                [center - Vec2::new(0.0, 4.0), center + Vec2::new(0.0, 4.0)],
                Stroke::new(0.5_f32, CYAN.gamma_multiply(0.35)),
            ));
        }
        shapes
    }
}
fn first_beat_envelope(bpm: f64, playback_seconds: Option<f64>) -> f32 {
    playback_seconds.map_or(0.0, |seconds| {
        let duration = 0.35_f64.min(60.0 / bpm * 0.85) as f32;
        let age = ((seconds * bpm / 60.0).rem_euclid(4.0) * 60.0 / bpm) as f32;
        smoothstep(0.0, duration * 0.12, age) * (1.0 - smoothstep(duration * 0.25, duration, age))
    })
}
fn dot_texture(ctx: &egui::Context) -> egui::TextureHandle {
    let mut image = egui::ColorImage::new([16, 16], Color32::TRANSPARENT);
    for y in 0..16 {
        for x in 0..16 {
            let distance = Vec2::new(x as f32 + 0.5 - 8.0, y as f32 + 0.5 - 8.0).length();
            image[(x, y)] =
                Color32::from_white_alpha(((8.0 - distance).clamp(0.0, 1.0) * 255.0) as u8);
        }
    }
    ctx.load_texture("matrix_dot", image, egui::TextureOptions::LINEAR)
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
fn smoothstep(low: f32, high: f32, value: f32) -> f32 {
    let t = ((value - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// Fixed lattice values with quintic interpolation, never per-dot/per-frame randomness.
fn noise(x: f32, y: f32, z: f32) -> f32 {
    let lattice = [x.floor() as i32, y.floor() as i32, z.floor() as i32];
    let fade = |v: f32| {
        let t = v - v.floor();
        t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
    };
    let value = |dx: i32, dy: i32, dz: i32| {
        let mut h = ((lattice[0] + dx) as u32).wrapping_mul(0x8da6b343)
            ^ ((lattice[1] + dy) as u32).wrapping_mul(0xd8163841)
            ^ ((lattice[2] + dz) as u32).wrapping_mul(0xcb1ab31f);
        h = (h ^ (h >> 16)).wrapping_mul(0x7feb352d);
        h = (h ^ (h >> 15)).wrapping_mul(0x846ca68b);
        (h ^ (h >> 16)) as f32 / u32::MAX as f32
    };
    let plane = |dz| {
        mix(
            mix(value(0, 0, dz), value(1, 0, dz), fade(x)),
            mix(value(0, 1, dz), value(1, 1, dz), fade(x)),
            fade(y),
        )
    };
    mix(plane(0), plane(1), fade(z))
}

fn fluid_field(u: f32, v: f32, time: f32) -> (f32, f32) {
    let time = time + 0.8;
    let warp = Vec2::new(
        noise(u * 1.2, v * 0.9, time * 0.18 + 4.7) - 0.5,
        noise(u * 1.2 + 41.3, v * 0.9 + 17.1, time * 0.15 + 8.2) - 0.5,
    );
    // Warp the sampling domain, never the rigid dot positions.
    let x = u * FLUID_SCALE.x - time * 0.24 + warp.x * 0.9;
    let y = v * FLUID_SCALE.y + warp.y * 0.75;
    let a = noise(x + 0.31, y + 0.27, time * 0.24 + 6.4);
    let b = noise(
        x * 1.35 + 19.2 + time * 0.11,
        y * 1.25 + 13.7,
        time * -0.17 + 29.1,
    );
    let c = noise(x * 0.5 + 51.2, y * 0.6 + 3.2, time * 0.10 + 2.3);
    let pink_field = a * 0.50 + b * 0.30 + c * 0.20;
    let cyan_field =
        a * 0.30 + b * 0.25 + noise(x * 0.92 + 7.3, y * 0.95 + 11.2, time * -0.20 + 3.6) * 0.45;
    let envelope = noise(x * 0.6 + 10.7, y * 0.7 + 5.3, time * 0.14 + 12.4);
    let width = FLUID_BAND_WIDTH * mix(0.55, 1.55, c);
    let band = |field: f32, threshold: f32, width: f32| {
        1.0 - smoothstep(
            width,
            width + FLUID_BAND_SOFTNESS,
            (field - threshold).abs(),
        )
    };
    // Independent contours and broad 2D envelopes allow streams to end, cross and separate.
    let highlight = mix(0.68, 1.0, smoothstep(0.65, 0.92, b));
    let pink = band(pink_field, 0.52, width) * smoothstep(0.32, 0.84, envelope).powi(2) * highlight;
    let cyan =
        band(cyan_field, 0.46, width * 1.40) * smoothstep(0.28, 0.84, c).powf(1.4) * highlight;
    (pink, cyan)
}

fn fluid_color(pink: f32, cyan: f32) -> [u8; 3] {
    const PINK: [[f32; 3]; 6] = [
        [130.0, 145.0, 159.0],
        [146.0, 145.0, 158.0],
        [168.0, 136.0, 155.0],
        [213.0, 201.0, 212.0],
        [224.0, 160.0, 185.0],
        [245.0, 130.0, 177.0],
    ];
    const CYAN: [[f32; 3]; 6] = [
        [130.0, 145.0, 159.0],
        [119.0, 144.0, 160.0],
        [91.0, 151.0, 179.0],
        [173.0, 207.0, 217.0],
        [204.0, 223.0, 229.0],
        [226.0, 237.0, 242.0],
    ];
    let strength = pink.max(cyan);
    let position = strength.clamp(0.0, 1.0) * 5.0;
    let index = (position.floor() as usize).min(4);
    let pink_weight = pink / (pink + cyan).max(0.0001);
    std::array::from_fn(|channel| {
        let color = |stops: &[[f32; 3]; 6]| {
            mix(
                stops[index][channel],
                stops[index + 1][channel],
                position - index as f32,
            )
        };
        mix(
            PINK[0][channel],
            mix(color(&CYAN), color(&PINK), pink_weight),
            FLUID_COLOR_INTENSITY,
        ) as u8
    })
}

fn dot(mesh: &mut egui::Mesh, center: Pos2, radius: f32, color: Color32) {
    mesh.add_rect_with_uv(
        Rect::from_center_size(center, Vec2::splat(radius * 2.0)),
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matrix_graphics_keep_their_world_position_when_zooming_and_panning() {
        for preset in [MatrixPreset::FluidGrid, MatrixPreset::CrossingWaves] {
            let ctx = egui::Context::default();
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0));
            let mut matrix = ParticleMatrix {
                preset,
                ..Default::default()
            };
            let mut original_color = None;
            for scale in [1.0, 2.0, 4.0] {
                let origin = Pos2::new(-2.75 * scale, -10.0);
                matrix.view = Some(MatrixView {
                    origin,
                    scale_x: scale,
                    ..Default::default()
                });
                let output = ctx.run(
                    egui::RawInput {
                        time: Some(0.0),
                        ..Default::default()
                    },
                    |ctx| {
                        ctx.layer_painter(egui::LayerId::background())
                            .extend(matrix.paint(ctx, rect, 120.0, None));
                    },
                );
                let mesh = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Mesh(mesh) => Some(mesh),
                        _ => None,
                    })
                    .max_by_key(|mesh| mesh.vertices.len())
                    .unwrap();
                let world_y = if preset == MatrixPreset::FluidGrid {
                    35.0
                } else {
                    31.5
                };
                let target = Pos2::new(origin.x + 88.0 * scale, origin.y + world_y);
                let color = mesh
                    .vertices
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .find(|quad| quad[0].pos.lerp(quad[3].pos, 0.5).distance(target) < 0.001)
                    .expect("The same beat must remain visible")[0]
                    .color;
                assert_eq!(
                    *original_color.get_or_insert(color),
                    color,
                    "Zoom/pan moved the graph to a different beat"
                );
            }
        }
    }
    #[test]
    fn arrangement_matrix_columns_land_on_musical_grid_for_both_presets() {
        for preset in [MatrixPreset::FluidGrid, MatrixPreset::CrossingWaves] {
            for zoom in [8.0, 22.0, 44.0, 88.0] {
                let ctx = egui::Context::default();
                let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0));
                let origin = Pos2::new(-3.125 * zoom, -25.0);
                let mut matrix = ParticleMatrix {
                    preset,
                    view: Some(MatrixView {
                        origin,
                        scale_x: zoom / 22.0,
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                let step = zoom
                    / if zoom >= 32.0 {
                        4.0
                    } else if zoom >= 16.0 {
                        2.0
                    } else {
                        1.0
                    };
                let output = ctx.run(egui::RawInput::default(), |ctx| {
                    ctx.layer_painter(egui::LayerId::background())
                        .extend(matrix.paint(ctx, rect, 120.0, None));
                });
                let mesh = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Mesh(mesh) => Some(mesh),
                        _ => None,
                    })
                    .max_by_key(|mesh| mesh.vertices.len())
                    .unwrap();
                for quad in mesh.vertices.as_chunks::<4>().0 {
                    let center = quad[0].pos.lerp(quad[3].pos, 0.5);
                    let tick = (center.x - origin.x) / step;
                    assert!(
                        (tick - tick.round()).abs() < 0.0001,
                        "{} dot at {} missed the musical grid at {zoom} px/beat",
                        preset.label(),
                        center.x
                    );
                }
            }
        }
    }
    #[test]
    fn hover_and_tempo_motion_preserve_the_grid_and_follow_transport() {
        fn frame(
            matrix: &mut ParticleMatrix,
            ctx: &egui::Context,
            time: f64,
            pointer: Option<Pos2>,
            playback: Option<f64>,
        ) -> Vec<egui::epaint::Vertex> {
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0));
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(rect),
                    time: Some(time),
                    events: vec![
                        pointer.map_or(egui::Event::PointerGone, egui::Event::PointerMoved)
                    ],
                    ..Default::default()
                },
                |ctx| {
                    ctx.layer_painter(egui::LayerId::background())
                        .with_clip_rect(rect)
                        .extend(matrix.paint(ctx, rect, 120.0, playback))
                },
            );
            output
                .shapes
                .into_iter()
                .filter_map(|shape| match shape.shape {
                    egui::Shape::Mesh(mesh) => Some(mesh.vertices.clone()),
                    _ => None,
                })
                .max_by_key(Vec::len)
                .unwrap()
        }
        let ctx = egui::Context::default();
        let hover_ctx = egui::Context::default();
        let mut baseline = ParticleMatrix::default();
        let mut hovered = ParticleMatrix::default();
        let initial = frame(
            &mut ParticleMatrix::default(),
            &egui::Context::default(),
            0.0,
            None,
            None,
        );
        let pointer = initial
            .as_chunks::<4>()
            .0
            .iter()
            .find_map(|quad| {
                let center = quad[0].pos.lerp(quad[3].pos, 0.5);
                (quad[0].color.a() <= 40
                    && center.x > 80.0
                    && center.x < 520.0
                    && center.y > 80.0
                    && center.y < 320.0)
                    .then_some(center)
            })
            .expect("The matrix should contain inactive pale background dots");
        for index in 0..=75 {
            let time = index as f64 / 60.0;
            let normal = frame(&mut baseline, &ctx, time, None, None);
            let hover = frame(
                &mut hovered,
                &hover_ctx,
                time,
                (index <= 30).then_some(pointer),
                None,
            );
            if index == 30 || index == 75 {
                for (a, b) in normal
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(hover.as_chunks::<4>().0)
                {
                    let center = a[0].pos.lerp(a[3].pos, 0.5);
                    assert!(center.distance(b[0].pos.lerp(b[3].pos, 0.5)) < 0.0001);
                    if index == 30 && center.distance(pointer) < 30.0 {
                        assert!(
                            b[0].color.a() > a[0].color.a() + 15,
                            "Hover must subtly illuminate nearby dots"
                        );
                        assert!(b[0].color.a().abs_diff(a[0].color.a()) <= 46);
                        if a[0].color.a() <= 45 {
                            assert!(
                                b[0].color.a().abs_diff(a[0].color.a()) >= 30,
                                "Hover is too weak on the pale background dots"
                            );
                        }
                    }
                    if center.distance(pointer) > 230.0 || index == 75 {
                        assert!(
                            b[0].color.a().abs_diff(a[0].color.a()) <= 1,
                            "Hover must stay local and fade after leaving"
                        );
                    }
                }
            }
        }
        let base = frame(
            &mut ParticleMatrix::default(),
            &egui::Context::default(),
            0.0,
            None,
            None,
        );
        let pulse = frame(
            &mut ParticleMatrix::default(),
            &egui::Context::default(),
            0.0,
            None,
            Some(0.08),
        );
        assert!(
            base.iter().zip(&pulse).all(|(a, b)| a.pos == b.pos),
            "Beat wave must change brightness without resizing or moving dots"
        );
        assert!(
            base.iter()
                .zip(&pulse)
                .any(|(a, b)| b.color.a() > a.color.a() + 8),
            "White contours must appear on the first beat"
        );
        assert!(
            base.iter()
                .zip(&pulse)
                .filter(|(a, b)| a.color != b.color)
                .count()
                < base.len() / 3,
            "White contours must remain sparse"
        );
        for bpm in [60.0, 120.0, 240.0, 400.0] {
            let beat = 60.0 / bpm;
            assert_eq!(first_beat_envelope(bpm, None), 0.0);
            assert!(first_beat_envelope(bpm, Some(beat * 0.15)) > 0.9);
            for offset in [1.0, 2.0, 3.0] {
                assert_eq!(first_beat_envelope(bpm, Some(beat * (offset + 0.15))), 0.0);
            }
            assert!(
                (first_beat_envelope(bpm, Some(beat * 4.15))
                    - first_beat_envelope(bpm, Some(beat * 0.15)))
                .abs()
                    < 0.001
            );
        }
        let offbeat = frame(
            &mut ParticleMatrix::default(),
            &egui::Context::default(),
            0.0,
            None,
            Some(0.6),
        );
        assert!(base
            .iter()
            .zip(&offbeat)
            .all(|(a, b)| a.pos == b.pos && a.color == b.color));
        let elapsed = |bpm| {
            let ctx = egui::Context::default();
            let mut matrix = ParticleMatrix::default();
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(1.0));
            for index in 0..=60 {
                let _ = ctx.run(
                    egui::RawInput {
                        time: Some(index as f64 / 60.0),
                        ..Default::default()
                    },
                    |ctx| {
                        ctx.layer_painter(egui::LayerId::background())
                            .with_clip_rect(rect)
                            .extend(matrix.paint(ctx, rect, bpm, None));
                    },
                );
            }
            let previous = matrix.field_time;
            let _ = ctx.run(
                egui::RawInput {
                    time: Some(1.0),
                    ..Default::default()
                },
                |ctx| {
                    ctx.layer_painter(egui::LayerId::background())
                        .with_clip_rect(rect)
                        .extend(matrix.paint(ctx, rect, 400.0, None));
                },
            );
            assert_eq!(
                matrix.field_time, previous,
                "Changing tempo must not jump the field's phase"
            );
            matrix.field_time
        };
        assert!(
            elapsed(60.0) < elapsed(120.0) && elapsed(240.0) > elapsed(120.0),
            "BPM must gently influence motion speed"
        );
    }
    #[test]
    fn fluid_grid_is_coherent_slow_and_keeps_dark_space() {
        assert!(ParticleMatrix::default().preset == MatrixPreset::FluidGrid);
        for time in [0.0, 0.8, 2.0, 6.0, 20.0] {
            let mut quiet = 0;
            let mut active = 0;
            let mut strong = 0;
            let mut brightest = 0;
            let mut pink_only = 0;
            let mut cyan_only = 0;
            let mut neighbor_change = 0.0;
            for row in 0..60 {
                for column in 0..100 {
                    let u = column as f32 / 100.0;
                    let v = row as f32 / 60.0;
                    let (pink, cyan) = fluid_field(u, v, time);
                    let strength = pink.max(cyan);
                    assert!((0.0..=1.0).contains(&pink) && (0.0..=1.0).contains(&cyan));
                    let adjacent = fluid_field(u + 0.01, v, time);
                    let adjacent_row = fluid_field(u, v + 1.0 / 60.0, time);
                    neighbor_change += (pink - adjacent.0).abs()
                        + (cyan - adjacent.1).abs()
                        + (pink - adjacent_row.0).abs()
                        + (cyan - adjacent_row.1).abs();
                    let next = fluid_field(u, v, time + FLUID_TIME_RATE as f32 / 60.0);
                    assert!(
                        (pink - next.0).abs().max((cyan - next.1).abs()) < 0.025,
                        "Contours must evolve continuously without flicker"
                    );
                    quiet += usize::from(strength < 0.15);
                    active += usize::from(strength >= 0.15);
                    strong += usize::from(strength > 0.60);
                    brightest += usize::from(strength > 0.90);
                    pink_only += usize::from(pink > 0.15 && cyan < 0.05);
                    cyan_only += usize::from(cyan > 0.15 && pink < 0.05);
                }
            }
            assert!(
                quiet > 4200,
                "The background must leave extensive quiet space: {quiet}/6000 at {time}"
            );
            assert!(
                active > 100,
                "Some organic contours must remain visible at {time}"
            );
            assert!(
                strong < 300 && brightest < 60,
                "Strong highlights must stay rare at {time}"
            );
            if time == 0.0 {
                assert!(
                    pink_only > 100 && cyan_only > 100,
                    "Pink and cyan must follow distinct initial contours"
                );
            }
            assert!(
                neighbor_change / 24000.0 < 0.025,
                "Neighboring dots must remain coherent"
            );
        }
        let mut matrix = ParticleMatrix::default();
        let ctx = egui::Context::default();
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0));
        for time in [0.0, 1.0 / 60.0, 600.0] {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(rect),
                    time: Some(time),
                    ..Default::default()
                },
                |ctx| {
                    ctx.layer_painter(egui::LayerId::background())
                        .with_clip_rect(rect)
                        .extend(matrix.paint(ctx, rect, 120.0, None));
                },
            );
            let dots = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Mesh(mesh) => Some(mesh.vertices.len() / 4),
                    _ => None,
                })
                .max()
                .unwrap();
            assert_eq!(dots, 60 * 40, "Each grid cell must have exactly one dot");
        }
        assert!(
            matrix.field_time < 0.012,
            "Resuming after a pause must not jump the animation"
        );
    }
    #[test]
    fn fluid_wave_visibly_moves_without_playback_or_pointer_input() {
        let ctx = egui::Context::default();
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0));
        let mut matrix = ParticleMatrix::default();
        let mut before = Vec::new();
        let mut after = Vec::new();
        let mut one_second = Vec::new();
        // Feed real consecutive frames so this also checks the animation clock.
        for frame in 0..=360 {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(rect),
                    time: Some(frame as f64 / 60.0),
                    ..Default::default()
                },
                |ctx| {
                    ctx.layer_painter(egui::LayerId::background())
                        .with_clip_rect(rect)
                        .extend(matrix.paint(ctx, rect, 120.0, None))
                },
            );
            if frame == 0 || frame == 60 || frame == 360 {
                let vertices = output
                    .shapes
                    .into_iter()
                    .filter_map(|shape| match shape.shape {
                        egui::Shape::Mesh(mesh) => Some(mesh.vertices.clone()),
                        _ => None,
                    })
                    .max_by_key(Vec::len)
                    .unwrap();
                if frame == 0 {
                    before = vertices;
                } else if frame == 60 {
                    one_second = vertices;
                } else {
                    after = vertices;
                }
            }
        }
        let one_second_change = before
            .as_chunks::<4>()
            .0
            .iter()
            .zip(one_second.as_chunks::<4>().0.iter())
            .map(|(a, b)| {
                a[0].color
                    .to_array()
                    .into_iter()
                    .zip(b[0].color.to_array())
                    .map(|(a, b)| a.abs_diff(b) as f32)
                    .sum::<f32>()
                    / 4.0
            })
            .sum::<f32>()
            / (before.len() / 4) as f32;
        assert!(
            one_second_change < 4.0,
            "Ambient motion must remain gradual over one second: {one_second_change}"
        );
        let changed = before
            .as_chunks::<4>()
            .0
            .iter()
            .zip(after.as_chunks::<4>().0.iter())
            .filter(|(a, b)| {
                a[0].color
                    .to_array()
                    .into_iter()
                    .zip(b[0].color.to_array())
                    .any(|(a, b)| a.abs_diff(b) >= 8)
            })
            .count();
        let count = before.len() / 4;
        assert!(
            changed > count / 10,
            "Only {changed}/{count} dots visibly changed in six seconds"
        );
        assert!(
            before
                .as_chunks::<4>()
                .0
                .iter()
                .zip(after.as_chunks::<4>().0.iter())
                .all(|(a, b)| {
                    let center_a = a[0].pos.lerp(a[3].pos, 0.5);
                    let center_b = b[0].pos.lerp(b[3].pos, 0.5);
                    center_a.distance(center_b) < 0.0001
                }),
            "The moving wave must preserve the dot matrix"
        );
    }
    #[test]
    fn waves_move_across_a_stationary_white_grid() {
        let frame = |time| {
            let ctx = egui::Context::default();
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0));
            let mut matrix = ParticleMatrix {
                preset: MatrixPreset::CrossingWaves,
                ..Default::default()
            };
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(rect),
                    time: Some(time),
                    ..Default::default()
                },
                |ctx| {
                    ctx.layer_painter(egui::LayerId::background())
                        .with_clip_rect(rect)
                        .extend(matrix.paint(ctx, rect, 120.0, None));
                },
            );
            output
                .shapes
                .into_iter()
                .find_map(|shape| match shape.shape {
                    egui::Shape::Mesh(mesh) => Some(
                        mesh.vertices
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .map(|quad| {
                                let mut vertex = quad[0];
                                vertex.pos = quad[0].pos.lerp(quad[3].pos, 0.5);
                                vertex
                            })
                            .collect::<Vec<_>>(),
                    ),
                    _ => None,
                })
                .unwrap()
        };
        let before = frame(0.0);
        let after = frame(1.0);
        let changes = before
            .iter()
            .zip(&after)
            .filter(|(a, b)| {
                a.color
                    .r()
                    .abs_diff(b.color.r())
                    .max(a.color.g().abs_diff(b.color.g()))
                    .max(a.color.b().abs_diff(b.color.b()))
                    > 24
            })
            .count();
        assert!(
            changes > before.len() / 10,
            "Only {changes}/{} dots visibly changed in one second",
            before.len()
        );
        assert!(before.iter().zip(&after).all(|(a, b)| a.pos == b.pos));
        assert!(before
            .iter()
            .take(50)
            .all(|v| v.color.r().abs_diff(v.color.b()) <= 2 && v.color.a() > 20));
    }
    #[test]
    fn cursor_repulsion_returns_to_grid_without_instability() {
        let mut particle = Particle::default();
        let base = Pos2::new(110.0, 100.0);
        for _ in 0..120 {
            particle.step(base, Some(Pos2::new(100.0, 100.0)), 1.0 / 60.0);
        }
        assert!(particle.offset.x > 15.0 && particle.offset.x < 23.0);
        assert!(particle.offset.y.abs() < 0.001);
        for _ in 0..240 {
            particle.step(base, None, 1.0 / 60.0);
        }
        assert!(particle.offset.length() < 0.01);
        assert!(particle.velocity.length() < 0.01);
        particle.step(base, Some(base), 1.0 / 30.0);
        assert!(particle.offset.x.is_finite() && particle.offset.y.is_finite());
    }
}

/// The oscillator uses the background matrix's dot texture and brightness-field rendering.
pub(super) fn oscillator_matrix(ui: &egui::Ui, rect: Rect, wave: u8) {
    let texture_id = egui::Id::new("oscillator_matrix_texture");
    let texture = ui
        .ctx()
        .data_mut(|d| d.get_temp::<egui::TextureHandle>(texture_id))
        .unwrap_or_else(|| {
            let texture = dot_texture(ui.ctx());
            ui.ctx()
                .data_mut(|d| d.insert_temp(texture_id, texture.clone()));
            texture
        });
    let mut mesh = egui::Mesh::with_texture(texture.id());
    let phase = ui.input(|i| i.time) * 0.12;
    let pitch = 5.0;
    let columns = ((rect.width() - 8.0) / pitch).floor() as usize;
    let rows = ((rect.height() - 8.0) / pitch).floor() as usize;
    let origin = rect.center() - Vec2::new(columns as f32, rows as f32) * pitch * 0.5;
    for x in 0..=columns {
        let u = x as f64 / columns.max(1) as f64;
        let sample =
            velvet_audio::synth::oscillator_c4(u * 3.0 + phase, wave).clamp(-1.0, 1.0) as f32;
        let wave_y = rect.center().y - sample * rect.height() * 0.34;
        for y in 0..=rows {
            let base = origin + Vec2::new(x as f32, y as f32) * pitch;
            let distance = (base.y - wave_y).abs();
            let strength = (-distance.powi(2) / 14.0).exp();
            let hover = ui
                .input(|i| i.pointer.hover_pos())
                .filter(|p| rect.contains(*p))
                .map_or(0.0, |p| (-(base - p).length_sq() / 1600.0).exp() * 0.10);
            let alpha = ((0.10 + strength * 0.80 + hover) * 255.0) as u8;
            let color = Color32::from_rgba_unmultiplied(153, 218, 237, alpha);
            dot(&mut mesh, base, 0.60 + strength * 0.85, color);
        }
    }
    ui.painter()
        .with_clip_rect(rect)
        .add(egui::Shape::mesh(mesh));
}

#[cfg(test)]
mod oscillator_tests {
    use super::*;
    #[test]
    fn oscillator_matrix_uses_a_dot_field_and_changes_with_wave_type() {
        let ctx = egui::Context::default();
        let draw = |wave| {
            ctx.run(
                egui::RawInput {
                    time: Some(0.0),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        oscillator_matrix(
                            ui,
                            Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(500.0, 76.0)),
                            wave,
                        );
                    });
                },
            )
            .shapes
            .into_iter()
            .find_map(|shape| match shape.shape {
                egui::Shape::Mesh(mesh) if mesh.vertices.len() > 1000 => Some(mesh),
                _ => None,
            })
            .expect("Wave must be drawn as a matrix of textured dots")
        };
        let sine = draw(0);
        let square = draw(2);
        assert_eq!(sine.vertices.len(), square.vertices.len());
        assert!(sine
            .vertices
            .iter()
            .zip(&square.vertices)
            .any(|(a, b)| a.color != b.color));
    }
}
