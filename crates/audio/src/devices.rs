use super::db;
use velvet_core::Device;

pub const EQ_FILTER_TYPES: [&str; 6] = [
    "Bell",
    "Low cut",
    "High cut",
    "Low shelf",
    "High shelf",
    "Notch",
];

pub(crate) fn process_devices(frames: &mut [[f32; 2]], devices: &[Device], rate: u32) {
    for device in devices {
        if device.kind == "builtin.limiter" {
            limiter(frames, device, rate);
        } else {
            DeviceProcessor::new(device, rate).process(frames);
        }
    }
}

/// Stateful gain, EQ and compressor processing. Construct on the DSP owner
/// thread after project validation, then reuse across blocks. Construction is
/// the reset boundary; processing neither allocates nor reads project state.
pub(crate) enum DeviceProcessor {
    Gain(f32),
    Eq {
        filters: Vec<[Biquad; 2]>,
        output: f32,
    },
    Compressor(Compressor),
}

impl DeviceProcessor {
    pub(crate) fn new(device: &Device, rate: u32) -> Self {
        let p = &device.parameters;
        match device.kind.as_str() {
            "builtin.gain" => Self::Gain(db(p["gain_db"])),
            "builtin.eq" => Self::Eq {
                filters: [
                    ("low_gain_db", 120.0),
                    ("mid_gain_db", 1000.0),
                    ("high_gain_db", 6000.0),
                ]
                .into_iter()
                .filter(|(key, _)| p[*key] != 0.0)
                .map(|(key, frequency)| [Biquad::peak(rate, frequency, p[key]); 2])
                .collect(),
                output: 1.0,
            },
            "builtin.eq8" => Self::Eq {
                filters: eq8_filters(device, rate)
                    .into_iter()
                    .map(|f| [f; 2])
                    .collect(),
                output: db(p["output_gain_db"]),
            },
            "builtin.compressor" => Self::Compressor(Compressor::new(device, rate)),
            _ => unreachable!("Only validated gain, EQ and compressor devices are supported"),
        }
    }

    pub(crate) fn process(&mut self, frames: &mut [[f32; 2]]) {
        match self {
            Self::Gain(value) => gain(frames, *value),
            Self::Eq { filters, output } => {
                for filter in filters {
                    for frame in frames.iter_mut() {
                        for (sample, channel) in frame.iter_mut().zip(filter.iter_mut()) {
                            *sample = channel.process(*sample);
                        }
                    }
                }
                gain(frames, *output);
            }
            Self::Compressor(compressor) => compressor.process(frames),
        }
    }
}
fn gain(frames: &mut [[f32; 2]], value: f32) {
    for frame in frames {
        for sample in frame {
            *sample *= value;
        }
    }
}
fn eq8_filters(device: &Device, rate: u32) -> Vec<Biquad> {
    let p = &device.parameters;
    (1..=8)
        .filter_map(|i| {
            let prefix = format!("band{i}");
            let kind = p[&format!("{prefix}_type")] as u8;
            let gain = p[&format!("{prefix}_gain_db")];
            if p[&format!("{prefix}_enabled")] == 0.0 || (kind == 0 && gain == 0.0) {
                return None;
            }
            Some(Biquad::new(
                rate,
                p[&format!("{prefix}_freq_hz")],
                gain,
                p[&format!("{prefix}_q")],
                kind,
            ))
        })
        .collect()
}
/// The graph uses the very same coefficients as audio processing.
pub fn eq_response(device: &Device, frequencies: &[f64], rate: u32) -> Vec<f64> {
    let filters = eq8_filters(device, rate);
    frequencies
        .iter()
        .map(|frequency| {
            device.parameters["output_gain_db"]
                + filters
                    .iter()
                    .map(|f| f.response(*frequency, rate))
                    .sum::<f64>()
        })
        .collect()
}

pub(crate) struct Compressor {
    attack: f64,
    release: f64,
    reduction: f64,
    threshold: f64,
    knee: f64,
    slope: f64,
    makeup: f64,
}

impl Compressor {
    fn new(device: &Device, rate: u32) -> Self {
        let p = &device.parameters;
        Self {
            attack: (-1.0 / (rate as f64 * p["attack_ms"] * 0.001)).exp(),
            release: (-1.0 / (rate as f64 * p["release_ms"] * 0.001)).exp(),
            reduction: 0.0,
            threshold: p["threshold_db"],
            knee: p["knee_db"],
            slope: 1.0 - 1.0 / p["ratio"],
            makeup: p["makeup_db"],
        }
    }

    fn process(&mut self, frames: &mut [[f32; 2]]) {
        for frame in frames {
            // Linked stereo peak detector preserves the stereo balance.
            let level = 20.0
                * f64::from(frame[0].abs().max(frame[1].abs()))
                    .max(1e-12)
                    .log10();
            let over = level - self.threshold;
            let knee = self.knee;
            let slope = self.slope;
            let target = if knee > 0.0 && over > -knee / 2.0 && over < knee / 2.0 {
                slope * (over + knee / 2.0).powi(2) / (2.0 * knee)
            } else {
                slope * over.max(0.0)
            };
            let coefficient = if target > self.reduction {
                self.attack
            } else {
                self.release
            };
            self.reduction = coefficient * self.reduction + (1.0 - coefficient) * target;
            let value = db(self.makeup - self.reduction);
            frame[0] *= value;
            frame[1] *= value;
        }
    }
}

#[cfg(test)]
fn compressor(frames: &mut [[f32; 2]], device: &Device, rate: u32) {
    Compressor::new(device, rate).process(frames);
}
fn limiter(frames: &mut [[f32; 2]], device: &Device, rate: u32) {
    let input = db(device.parameters["input_gain_db"]);
    let ceiling = db(device.parameters["ceiling_db"]);
    let release = (-1.0 / (rate as f64 * device.parameters["release_ms"] * 0.001)).exp() as f32;
    let step = 1.0 / (rate as f32 * 0.005);
    let mut next = 1.0_f32;
    // Offline lookahead is compensated: no samples are shifted in time.
    // ponytail: one f32 envelope per frame; switch to a bounded sliding window
    // when the renderer becomes a streaming graph.
    let attack: Vec<f32> = frames
        .iter()
        .rev()
        .map(|frame| {
            let peak = frame[0].abs().max(frame[1].abs()) * input;
            let required = if peak > ceiling { ceiling / peak } else { 1.0 };
            next = required.min(next + step);
            next
        })
        .collect();
    let mut attenuation = 1.0_f32;
    for (frame, desired) in frames.iter_mut().zip(attack.into_iter().rev()) {
        attenuation = desired.min(1.0 - (1.0 - attenuation) * release);
        let value = input * attenuation;
        frame[0] *= value;
        frame[1] *= value;
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}
impl Biquad {
    pub(crate) fn peak(rate: u32, frequency: f64, gain: f64) -> Self {
        Self::new(rate, frequency, gain, 0.7, 0)
    }
    // RBJ coefficients: https://www.w3.org/TR/audio-eq-cookbook/
    fn new(rate: u32, frequency: f64, gain: f64, q: f64, kind: u8) -> Self {
        let a = 10.0_f64.powf(gain / 40.0);
        let w = std::f64::consts::TAU * frequency.min(rate as f64 * 0.45) / rate as f64;
        let c = w.cos();
        let alpha = w.sin() / (2.0 * q);
        let beta = 2.0 * a.sqrt() * alpha;
        let (b, denominator) = match kind {
            0 => (
                [1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a],
                [1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a],
            ),
            1 => (
                [(1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0],
                [1.0 + alpha, -2.0 * c, 1.0 - alpha],
            ),
            2 => (
                [(1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0],
                [1.0 + alpha, -2.0 * c, 1.0 - alpha],
            ),
            3 => (
                [
                    a * ((a + 1.0) - (a - 1.0) * c + beta),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * c),
                    a * ((a + 1.0) - (a - 1.0) * c - beta),
                ],
                [
                    (a + 1.0) + (a - 1.0) * c + beta,
                    -2.0 * ((a - 1.0) + (a + 1.0) * c),
                    (a + 1.0) + (a - 1.0) * c - beta,
                ],
            ),
            4 => (
                [
                    a * ((a + 1.0) + (a - 1.0) * c + beta),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
                    a * ((a + 1.0) + (a - 1.0) * c - beta),
                ],
                [
                    (a + 1.0) - (a - 1.0) * c + beta,
                    2.0 * ((a - 1.0) - (a + 1.0) * c),
                    (a + 1.0) - (a - 1.0) * c - beta,
                ],
            ),
            5 => ([1.0, -2.0 * c, 1.0], [1.0 + alpha, -2.0 * c, 1.0 - alpha]),
            _ => unreachable!("Validated EQ filter type"),
        };
        Self {
            b: b.map(|v| v / denominator[0]),
            a: [
                denominator[1] / denominator[0],
                denominator[2] / denominator[0],
            ],
            z: [0.0; 2],
        }
    }
    fn response(&self, frequency: f64, rate: u32) -> f64 {
        let w = std::f64::consts::TAU * frequency / rate as f64;
        let magnitude = |b: [f64; 3]| {
            let real = b[0] + b[1] * w.cos() + b[2] * (2.0 * w).cos();
            let imaginary = b[1] * w.sin() + b[2] * (2.0 * w).sin();
            real * real + imaginary * imaginary
        };
        10.0 * (magnitude(self.b).max(1e-24) / magnitude([1.0, self.a[0], self.a[1]]).max(1e-24))
            .log10()
    }
    pub(crate) fn process(&mut self, x: f32) -> f32 {
        let x = x as f64;
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persistent_devices_preserve_filter_tails_and_compressor_release_across_blocks() {
        for kind in [
            "builtin.gain",
            "builtin.eq",
            "builtin.eq8",
            "builtin.compressor",
        ] {
            let mut device = Device::new(kind).unwrap();
            match kind {
                "builtin.eq" => {
                    device.parameters.insert("low_gain_db".into(), 12.0);
                }
                "builtin.eq8" => {
                    device.parameters.insert("band3_gain_db".into(), -9.0);
                }
                "builtin.compressor" => {
                    device.parameters.insert("threshold_db".into(), -30.0);
                }
                _ => {
                    device.parameters.insert("gain_db".into(), -6.0);
                }
            }
            for rate in [8000, 44100, 48000, 192000] {
                let mut input = vec![[0.01, -0.005]; 4097];
                // Transients on either side of common block boundaries, followed
                // by a quiet tail that exposes resetting filters or release.
                for i in [0, 63, 64, 255, 256, 511, 512] {
                    input[i] = [1.0, -0.5];
                }
                let mut reference = input.clone();
                process_devices(&mut reference, std::slice::from_ref(&device), rate);
                for block_size in [1, 63, 256, 511, 1024] {
                    let mut actual = input.clone();
                    let mut processor = DeviceProcessor::new(&device, rate);
                    processor.process(&mut []);
                    for block in actual.chunks_mut(block_size) {
                        processor.process(block);
                    }
                    assert_eq!(actual, reference, "{kind}, {rate} Hz, block {block_size}");
                }
                if kind != "builtin.gain" {
                    let mut restarted = input.clone();
                    for block in restarted.chunks_mut(64) {
                        DeviceProcessor::new(&device, rate).process(block);
                    }
                    assert!(
                        restarted
                            .iter()
                            .zip(&reference)
                            .any(|(a, b)| (a[0] - b[0]).abs() > 1e-6),
                        "Fixture must detect state loss: {kind}, {rate} Hz"
                    );
                }
            }
        }
    }

    #[test]
    fn compressor_ratio_attack_release_knee_and_stereo_link() {
        let mut device = Device::new("builtin.compressor").unwrap();
        device.parameters.insert("threshold_db".into(), -20.0);
        device.parameters.insert("knee_db".into(), 0.0);
        let mut frames = vec![[1.0, 0.5]; 48000];
        compressor(&mut frames, &device, 48000);
        assert!(
            frames[0][0] > frames[24000][0],
            "Attack must preserve the start of a transient"
        );
        assert!(
            (frames[24000][0] - db(-15.0)).abs() < 0.0001,
            "4:1 must turn 20 dB above threshold into 5 dB"
        );
        assert!((frames[24000][1] / frames[24000][0] - 0.5).abs() < 0.00001);
        let mut tail = vec![[1.0, 0.5]; 4800];
        tail.extend(vec![[0.01, 0.005]; 48000]);
        compressor(&mut tail, &device, 48000);
        assert!(tail[4801][0] < tail[47000][0]);
        assert!((tail.last().unwrap()[0] - 0.01).abs() < 0.00001);
        device.parameters.insert("ratio".into(), 1.0);
        device.parameters.insert("makeup_db".into(), 6.0);
        let mut unity = [[0.1, 0.05]; 100];
        compressor(&mut unity, &device, 48000);
        assert!((unity[50][0] - 0.1 * db(6.0)).abs() < 0.00001);
        device.parameters.insert("ratio".into(), 4.0);
        device.parameters.insert("makeup_db".into(), 0.0);
        device.parameters.insert("knee_db".into(), 12.0);
        let mut knee = vec![[db(-23.0); 2]; 48000];
        compressor(&mut knee, &device, 48000);
        assert!(
            knee[47000][0] < db(-23.0),
            "Soft knee must begin below threshold"
        );
    }
    #[test]
    fn limiter_guarantees_ceiling_without_shifting_transients_and_releases() {
        let mut device = Device::new("builtin.limiter").unwrap();
        device.parameters.insert("ceiling_db".into(), -6.0);
        device.parameters.insert("input_gain_db".into(), 6.0);
        for rate in [8000, 44100, 48000, 192000] {
            let center = rate as usize / 10;
            let mut frames = vec![[0.02, 0.01]; rate as usize];
            frames[0] = [2.0, -1.0];
            frames[center] = [-4.0, 2.0];
            frames[center + 1] = [3.0, -1.5];
            limiter(&mut frames, &device, rate);
            let ceiling = db(-6.0);
            assert!(frames
                .iter()
                .flatten()
                .all(|s| s.is_finite() && s.abs() <= ceiling + 1e-6));
            assert!((frames[center][0] + ceiling).abs() < 1e-5);
            assert!((frames[center][1] / frames[center][0] + 0.5).abs() < 1e-6);
            assert!(
                frames[center - 1][0] < frames[center - rate as usize / 50][0],
                "Lookahead must act before the peak"
            );
            assert!(
                frames[center + 2][0] < frames[rate as usize - 1][0],
                "Release must recover after the peak"
            );
        }
        let mut quiet = [[0.01, -0.02]; 100];
        device.parameters.insert("input_gain_db".into(), 0.0);
        limiter(&mut quiet, &device, 48000);
        assert_eq!(quiet, [[0.01, -0.02]; 100]);
    }
    #[test]
    fn eq_curve_matches_measured_audio_and_band_types_are_stable() {
        let mut device = Device::new("builtin.eq8").unwrap();
        device.parameters.insert("band3_gain_db".into(), -9.0);
        let curve = eq_response(&device, &[1000.0], 48000)[0];
        let mut frames: Vec<_> = (0..48000)
            .map(|i| [(std::f64::consts::TAU * 1000.0 * i as f64 / 48000.0).sin() as f32; 2])
            .collect();
        process_devices(&mut frames, &[device.clone()], 48000);
        let rms = (frames[4800..]
            .iter()
            .map(|f| f64::from(f[0]).powi(2))
            .sum::<f64>()
            / 43200.0)
            .sqrt();
        assert!((20.0 * (rms * 2.0_f64.sqrt()).log10() - curve).abs() < 0.01);
        assert!((curve + 9.0).abs() < 0.001);
        device.parameters.insert("band3_enabled".into(), 0.0);
        assert_eq!(eq_response(&device, &[1000.0], 48000), [0.0]);
        for kind in 0..=5 {
            for rate in [8000, 48000, 192000] {
                for frequency in [20.0, 1000.0, 20000.0] {
                    for q in [0.1, 0.707, 18.0] {
                        let mut filter = Biquad::new(rate, frequency, 24.0, q, kind);
                        for i in 0..10000 {
                            assert!(filter.process(if i == 0 { 1.0 } else { 0.0 }).is_finite());
                        }
                    }
                }
            }
        }
        assert!(Biquad::new(48000, 1000.0, 0.0, 0.707, 1).response(50.0, 48000) < -40.0);
        assert!(Biquad::new(48000, 1000.0, 0.0, 0.707, 2).response(18000.0, 48000) < -40.0);
        assert!(Biquad::new(48000, 1000.0, 0.0, 1.0, 5).response(1000.0, 48000) < -100.0);
        assert!(
            (Biquad::new(48000, 1000.0, 6.0, 0.707, 3).response(20.0, 48000) - 6.0).abs() < 0.01
        );
        assert!(
            (Biquad::new(48000, 1000.0, -6.0, 0.707, 4).response(20000.0, 48000) + 6.0).abs()
                < 0.01
        );
    }
}
