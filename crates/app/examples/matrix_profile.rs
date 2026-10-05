use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
const CYAN: Color32 = Color32::from_rgb(56, 189, 248);
const ROSE: Color32 = Color32::from_rgb(243, 134, 161);
#[path = "../src/matrix.rs"]
#[allow(dead_code)] // The profiling example uses only the arrangement matrix.
mod matrix;

fn main() {
    for preset in [
        matrix::MatrixPreset::FluidGrid,
        matrix::MatrixPreset::CrossingWaves,
    ] {
        profile(preset, 1.0);
    }
    profile(matrix::MatrixPreset::FluidGrid, 8.0 / 22.0);
    profile(matrix::MatrixPreset::FluidGrid, 90.0 / 22.0);
}

fn profile(preset: matrix::MatrixPreset, scale_x: f32) {
    let ctx = egui::Context::default();
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1920.0, 1080.0));
    let mut matrix = matrix::ParticleMatrix::default();
    matrix.preset = preset;
    matrix.view = Some(matrix::MatrixView {
        scale_x,
        ..Default::default()
    });
    let mut frames = Vec::new();
    for frame in 0..100 {
        let start = std::time::Instant::now();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(rect),
                time: Some(frame as f64 / 60.0),
                events: vec![egui::Event::PointerMoved(Pos2::new(
                    900.0 + frame as f32,
                    500.0,
                ))],
                ..Default::default()
            },
            |ctx| {
                ctx.layer_painter(egui::LayerId::background())
                    .with_clip_rect(rect)
                    .extend(matrix.paint(ctx, rect, 120.0, Some(frame as f64 / 60.0)));
            },
        );
        let _primitives = ctx.tessellate(output.shapes, 1.5);
        if frame > 10 {
            frames.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    frames.sort_by(f64::total_cmp);
    println!(
        "{} / zoom {:.2} / 1920x1080 matrix CPU frame: median {:.2} ms, p95 {:.2} ms",
        preset.label(),
        scale_x,
        frames[frames.len() / 2],
        frames[frames.len() * 95 / 100]
    );
}
