//! Kelly–Lochbaum vocal-tract waveguide.
//!
//! A 1-D digital waveguide model of the vocal tract: the tube is divided into
//! `n` cylindrical sections, each carrying a right-going and a left-going
//! pressure wave with a one-sample delay. Between adjacent sections a scattering
//! junction reflects/transmits the waves by a coefficient derived from the
//! cross-sectional areas — `k = (Aᵢ₋₁ − Aᵢ) / (Aᵢ₋₁ + Aᵢ)`. Articulating the
//! area profile (tongue, lips) moves the formants, exactly as in a real tract.
//!
//! This is the speech-grade engine behind the `VocalTract` module; the
//! complementary `VoiceSynth` module uses the cheaper source–filter approach.
//! Reference lineage: Kelly & Lochbaum (1962); Pink Trombone (Thapen); Voc
//! (Batchelor).

/// Smallest area allowed, so junction denominators never reach zero.
const MIN_AREA: f32 = 1e-4;
/// Below this magnitude the wave state is flushed to zero, so a ringing-out
/// ladder can't linger in subnormal floats and stall the audio thread.
const DENORMAL_FLOOR: f32 = 1e-20;

/// A Kelly–Lochbaum vocal-tract ladder of `n` equal-length sections.
///
/// Allocation happens only in [`Self::new`] / [`Self::resize`]; [`Self::step`]
/// is allocation-free and real-time safe. All reflection coefficients have
/// magnitude < 1 and `damping` < 1, so the ladder is unconditionally stable.
#[derive(Clone, Debug)]
pub struct KellyLochbaumTract {
    n: usize,
    right: Vec<f32>,
    left: Vec<f32>,
    new_right: Vec<f32>,
    new_left: Vec<f32>,
    /// Reflection coefficient at the junction left of section `i` (1..n).
    k: Vec<f32>,
    /// Per-section cross-sectional area.
    area: Vec<f32>,
    glottal_reflection: f32,
    lip_reflection: f32,
    damping: f32,
}

impl KellyLochbaumTract {
    /// Create a uniform tube of `n` sections (clamped to a minimum of 2).
    #[must_use]
    pub fn new(n: usize) -> Self {
        let n = n.max(2);
        let mut t = Self {
            n,
            right: vec![0.0; n],
            left: vec![0.0; n],
            new_right: vec![0.0; n],
            new_left: vec![0.0; n],
            k: vec![0.0; n],
            area: vec![1.0; n],
            glottal_reflection: 0.75,
            lip_reflection: -0.85,
            damping: 0.995,
        };
        t.update_reflections();
        t
    }

    /// Number of sections.
    #[must_use]
    pub fn sections(&self) -> usize {
        self.n
    }

    /// Resize to `n` sections (uniform tube) — allocates; not real-time safe.
    pub fn resize(&mut self, n: usize) {
        let n = n.max(2);
        self.n = n;
        self.right = vec![0.0; n];
        self.left = vec![0.0; n];
        self.new_right = vec![0.0; n];
        self.new_left = vec![0.0; n];
        self.k = vec![0.0; n];
        self.area = vec![1.0; n];
        self.update_reflections();
    }

    /// Set the cross-sectional area of section `i` (no-op if out of range).
    /// Call [`Self::update_reflections`] after a batch of edits.
    pub fn set_area(&mut self, i: usize, area: f32) {
        if let Some(a) = self.area.get_mut(i) {
            *a = area.max(MIN_AREA);
        }
    }

    /// Set section `i` from a diameter (`area = diameter²`).
    pub fn set_diameter(&mut self, i: usize, diameter: f32) {
        self.set_area(i, diameter.max(0.0) * diameter.max(0.0));
    }

    /// End reflection coefficients and per-step damping (each clamped to a
    /// stable magnitude < 1).
    pub fn set_terminations(&mut self, glottal_reflection: f32, lip_reflection: f32, damping: f32) {
        self.glottal_reflection = glottal_reflection.clamp(-0.999, 0.999);
        self.lip_reflection = lip_reflection.clamp(-0.999, 0.999);
        self.damping = damping.clamp(0.0, 1.0);
    }

    /// Recompute junction reflection coefficients from the area profile.
    pub fn update_reflections(&mut self) {
        for i in 1..self.n {
            let a0 = self.area[i - 1];
            let a1 = self.area[i];
            self.k[i] = (a0 - a1) / (a0 + a1).max(MIN_AREA);
        }
    }

    /// Clear all wave state (keeps the area profile).
    pub fn reset(&mut self) {
        self.right.iter_mut().for_each(|x| *x = 0.0);
        self.left.iter_mut().for_each(|x| *x = 0.0);
        self.new_right.iter_mut().for_each(|x| *x = 0.0);
        self.new_left.iter_mut().for_each(|x| *x = 0.0);
    }

    /// Advance the waveguide one sample with `glottal_input` injected at the
    /// glottis end, returning the pressure radiated at the lips.
    #[inline]
    pub fn step(&mut self, glottal_input: f32) -> f32 {
        let n = self.n;

        // Terminations: glottis reflects + injects the source; lips reflect.
        self.new_right[0] = self.left[0] * self.glottal_reflection + glottal_input;
        self.new_left[n - 1] = self.right[n - 1] * self.lip_reflection;

        // Interior scattering junctions (one-multiply Kelly–Lochbaum form).
        for i in 1..n {
            let w = self.k[i] * (self.right[i - 1] + self.left[i]);
            self.new_right[i] = self.right[i - 1] - w;
            self.new_left[i - 1] = self.left[i] + w;
        }

        // The right-going wave leaving the last junction radiates past the lips.
        let lip_output = self.new_right[n - 1] * (1.0 + self.lip_reflection);

        // One-sample delay per section, with damping (keeps the ladder stable)
        // and a flush-to-zero so a silent tail can't decay into subnormals.
        let d = self.damping;
        for i in 0..n {
            let r = self.new_right[i] * d;
            let l = self.new_left[i] * d;
            self.right[i] = if r.abs() < DENORMAL_FLOOR { 0.0 } else { r };
            self.left[i] = if l.abs() < DENORMAL_FLOOR { 0.0 } else { l };
        }

        lip_output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A uniform tube driven by an impulse train must stay bounded (stable) and
    /// produce a non-trivial, resonant response.
    #[test]
    fn uniform_tube_is_stable_and_resonant() {
        let mut t = KellyLochbaumTract::new(44);
        let mut max = 0.0_f32;
        let mut energy = 0.0_f32;
        for n in 0..20_000 {
            // Sparse glottal impulses (~120 Hz at 44.1 kHz, two steps/sample).
            let drive = if n % 368 == 0 { 1.0 } else { 0.0 };
            let _ = t.step(drive);
            let out = t.step(0.0);
            max = max.max(out.abs());
            if n > 10_000 {
                energy += out * out;
            }
        }
        assert!(max.is_finite() && max < 50.0, "tract unstable, max={max}");
        assert!(energy > 1e-6, "tract produced no resonant energy: {energy}");
    }

    /// Changing the area profile (a constriction) must change the output —
    /// i.e. articulation actually moves the response.
    #[test]
    fn area_profile_changes_output() {
        let render = |constrict: bool| {
            let mut t = KellyLochbaumTract::new(44);
            if constrict {
                for i in 20..26 {
                    t.set_diameter(i, 0.3);
                }
                t.update_reflections();
            }
            let mut sum = 0.0_f32;
            for n in 0..8_000 {
                let drive = if n % 368 == 0 { 1.0 } else { 0.0 };
                t.step(drive);
                sum += t.step(0.0).abs();
            }
            sum
        };
        let open = render(false);
        let constricted = render(true);
        assert!(
            (open - constricted).abs() > 1e-3,
            "constriction had no effect: open={open}, constricted={constricted}"
        );
    }

    #[test]
    fn reflections_in_unit_range() {
        let mut t = KellyLochbaumTract::new(30);
        t.set_diameter(10, 0.2);
        t.set_diameter(11, 2.5);
        t.update_reflections();
        for i in 1..t.sections() {
            assert!(
                t.k[i].abs() < 1.0,
                "reflection {i} out of range: {}",
                t.k[i]
            );
        }
    }
}
