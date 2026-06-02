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

/// Flush subnormal/near-zero wave samples to zero so a decaying tail can't
/// stall the audio thread on denormal arithmetic.
#[inline]
fn flush(x: f32) -> f32 {
    if x.abs() < DENORMAL_FLOOR { 0.0 } else { x }
}

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

    // --- Optional nasal side-branch (empty when `nose_len < 2`) ---
    /// Nasal ladder, glottis→nostril order (right = toward the nostril).
    nose_right: Vec<f32>,
    nose_left: Vec<f32>,
    nose_new_right: Vec<f32>,
    nose_new_left: Vec<f32>,
    nose_k: Vec<f32>,
    nose_area: Vec<f32>,
    nose_len: usize,
    /// Main-tract junction (between section `velum_index-1` and `velum_index`)
    /// where the nose taps in.
    velum_index: usize,
    /// Velar port opening in 0..1; scales the nasal coupling area (0 = closed).
    velum_opening: f32,
    /// Reflection at the nostril termination.
    nostril_reflection: f32,
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
            nose_right: Vec::new(),
            nose_left: Vec::new(),
            nose_new_right: Vec::new(),
            nose_new_left: Vec::new(),
            nose_k: Vec::new(),
            nose_area: Vec::new(),
            nose_len: 0,
            velum_index: 0,
            velum_opening: 0.0,
            nostril_reflection: -0.85,
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
        // Keep the velum tap inside the resized tract.
        if self.nose_len >= 2 {
            self.velum_index = self.velum_index.clamp(1, n - 1);
        }
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

    /// Set only the glottal-end reflection (clamped to a stable magnitude < 1).
    /// Cheap enough to call per sample, for source–tract (glottal) coupling
    /// where the boundary stiffens as the glottis closes.
    #[inline]
    pub fn set_glottal_reflection(&mut self, reflection: f32) {
        self.glottal_reflection = reflection.clamp(-0.999, 0.999);
    }

    /// Attach a nasal side-branch of `nose_len` sections (clamped to ≥ 2),
    /// tapping the main tract at `velum_index` (clamped to a valid interior
    /// junction). Allocates; not real-time safe. The branch starts decoupled
    /// (`velum_opening` 0) — open it with [`Self::set_velum`].
    pub fn enable_nose(&mut self, nose_len: usize, velum_index: usize) {
        let nose_len = nose_len.max(2);
        self.nose_len = nose_len;
        self.nose_right = vec![0.0; nose_len];
        self.nose_left = vec![0.0; nose_len];
        self.nose_new_right = vec![0.0; nose_len];
        self.nose_new_left = vec![0.0; nose_len];
        self.nose_k = vec![0.0; nose_len];
        self.nose_area = vec![1.0; nose_len];
        self.velum_index = velum_index.clamp(1, self.n - 1);
        self.update_nose_reflections();
    }

    /// Whether a nasal branch is attached.
    #[must_use]
    pub fn has_nose(&self) -> bool {
        self.nose_len >= 2
    }

    /// Set the velar port opening (0 = nose decoupled, 1 = fully open).
    #[inline]
    pub fn set_velum(&mut self, opening: f32) {
        self.velum_opening = opening.clamp(0.0, 1.0);
    }

    /// Set nasal section `i` from a diameter (no-op if out of range / no nose).
    /// Call [`Self::update_nose_reflections`] after a batch of edits.
    pub fn set_nose_diameter(&mut self, i: usize, diameter: f32) {
        if let Some(a) = self.nose_area.get_mut(i) {
            *a = (diameter.max(0.0) * diameter.max(0.0)).max(MIN_AREA);
        }
    }

    /// Recompute the nasal ladder's junction reflection coefficients.
    pub fn update_nose_reflections(&mut self) {
        for i in 1..self.nose_len {
            let a0 = self.nose_area[i - 1];
            let a1 = self.nose_area[i];
            self.nose_k[i] = (a0 - a1) / (a0 + a1).max(MIN_AREA);
        }
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
        for v in [
            &mut self.right,
            &mut self.left,
            &mut self.new_right,
            &mut self.new_left,
            &mut self.nose_right,
            &mut self.nose_left,
            &mut self.nose_new_right,
            &mut self.nose_new_left,
        ] {
            v.iter_mut().for_each(|x| *x = 0.0);
        }
    }

    /// Advance the waveguide one sample with `glottal_input` injected at the
    /// glottis end, returning the radiated pressure (lips, plus the nostrils
    /// when a nasal branch is attached and the velum is open).
    #[inline]
    pub fn step(&mut self, glottal_input: f32) -> f32 {
        if self.nose_len < 2 {
            return self.step_oral(glottal_input);
        }
        self.step_with_nose(glottal_input)
    }

    /// Oral-only step: a single Kelly–Lochbaum ladder, glottis → lips.
    #[inline]
    fn step_oral(&mut self, glottal_input: f32) -> f32 {
        let n = self.n;

        // Terminations: glottis reflects + injects the source; lips reflect.
        self.new_right[0] = self.left[0] * self.glottal_reflection + glottal_input;
        self.new_left[n - 1] = self.right[n - 1] * self.lip_reflection;

        // Interior scattering junctions: the lossless area-weighted pressure
        // junction `p_j = ((1+k)·right + (1-k)·left)` (a rearrangement of
        // `2·(A₀·right + A₁·left)/(A₀+A₁)` using the precomputed reflection k),
        // with each outgoing wave `p_j − incoming`.
        for i in 1..n {
            let p_j = (1.0 + self.k[i]) * self.right[i - 1] + (1.0 - self.k[i]) * self.left[i];
            self.new_right[i] = p_j - self.left[i];
            self.new_left[i - 1] = p_j - self.right[i - 1];
        }

        // The right-going wave leaving the last junction radiates past the lips.
        let lip_output = self.new_right[n - 1] * (1.0 + self.lip_reflection);

        // One-sample delay per section, with damping (keeps the ladder stable)
        // and a flush-to-zero so a silent tail can't decay into subnormals.
        let d = self.damping;
        for i in 0..n {
            self.right[i] = flush(self.new_right[i] * d);
            self.left[i] = flush(self.new_left[i] * d);
        }

        lip_output
    }

    /// Step with the nasal branch: the velum junction is a 3-port scattering
    /// node coupling the pharyngeal side, the oral side, and the nasal ladder;
    /// output sums the lip and nostril radiation.
    #[inline]
    fn step_with_nose(&mut self, glottal_input: f32) -> f32 {
        let n = self.n;
        let m = self.nose_len;
        let v = self.velum_index;

        // Main-tract terminations.
        self.new_right[0] = self.left[0] * self.glottal_reflection + glottal_input;
        self.new_left[n - 1] = self.right[n - 1] * self.lip_reflection;

        // Main-tract interior junctions (pressure junctions), skipping the
        // velum (handled below as a 3-port).
        for i in 1..n {
            if i == v {
                continue;
            }
            let p_j = (1.0 + self.k[i]) * self.right[i - 1] + (1.0 - self.k[i]) * self.left[i];
            self.new_right[i] = p_j - self.left[i];
            self.new_left[i - 1] = p_j - self.right[i - 1];
        }

        // Velum: a lossless 3-port pressure junction connecting the pharyngeal
        // side (right[v-1]), the oral side (left[v]) and the nasal port
        // (nose_left[0]). The velum opening `g` scales the nasal coupling area,
        // so `g=0` collapses the main tract to the same 2-port as the oral path
        // (a closed velum never colours the oral response). The nasal injection
        // is additionally scaled by `g` — strictly dissipative (it only ever
        // attenuates an outgoing wave of the lossless junction), which keeps the
        // partly-open junction passive and unconditionally stable, and silences
        // the nose entirely when the velum is shut.
        let g = self.velum_opening;
        let p_b = self.right[v - 1];
        let p_f = self.left[v];
        let p_nz = self.nose_left[0];
        let a_b = self.area[v - 1];
        let a_f = self.area[v];
        let a_nz = self.nose_area[0] * g;
        let p_j = 2.0 * (a_b * p_b + a_f * p_f + a_nz * p_nz) / (a_b + a_f + a_nz).max(MIN_AREA);
        self.new_right[v] = p_j - p_f;
        self.new_left[v - 1] = p_j - p_b;
        let nose_in = g * (p_j - p_nz);

        // Nasal ladder: inject the junction wave, reflect at the nostril, and
        // scatter through the same pressure junctions as the main tract.
        self.nose_new_right[0] = nose_in;
        self.nose_new_left[m - 1] = self.nose_right[m - 1] * self.nostril_reflection;
        for i in 1..m {
            let p_jn = (1.0 + self.nose_k[i]) * self.nose_right[i - 1]
                + (1.0 - self.nose_k[i]) * self.nose_left[i];
            self.nose_new_right[i] = p_jn - self.nose_left[i];
            self.nose_new_left[i - 1] = p_jn - self.nose_right[i - 1];
        }

        let lip_output = self.new_right[n - 1] * (1.0 + self.lip_reflection);
        let nostril_output = self.nose_new_right[m - 1] * (1.0 + self.nostril_reflection);

        // One-sample delay + damping + denormal flush for both ladders.
        let d = self.damping;
        for i in 0..n {
            self.right[i] = flush(self.new_right[i] * d);
            self.left[i] = flush(self.new_left[i] * d);
        }
        for i in 0..m {
            self.nose_right[i] = flush(self.nose_new_right[i] * d);
            self.nose_left[i] = flush(self.nose_new_left[i] * d);
        }

        lip_output + nostril_output
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

    /// The glottal-reflection setter clamps to the stable range and a different
    /// glottal boundary produces a different response (source–tract coupling).
    #[test]
    fn glottal_reflection_setter_clamps_and_couples() {
        let mut t = KellyLochbaumTract::new(20);
        t.set_glottal_reflection(5.0);
        assert!((t.glottal_reflection - 0.999).abs() < 1e-6);
        t.set_glottal_reflection(-5.0);
        assert!((t.glottal_reflection + 0.999).abs() < 1e-6);

        let render = |reflection: f32| {
            let mut t = KellyLochbaumTract::new(20);
            let mut sum = 0.0_f32;
            for n in 0..4_000 {
                t.set_glottal_reflection(reflection);
                let drive = if n % 200 == 0 { 1.0 } else { 0.0 };
                t.step(drive);
                sum += t.step(0.0).abs();
            }
            sum
        };
        let stiff = render(0.97);
        let open = render(0.6);
        assert!(
            (stiff - open).abs() > 1e-3,
            "glottal coupling had no effect: stiff={stiff}, open={open}"
        );
    }

    /// A fresh tract has no nasal branch and uses the oral-only path.
    #[test]
    fn nose_disabled_by_default() {
        let t = KellyLochbaumTract::new(20);
        assert!(!t.has_nose());
    }

    /// With the velum shut, no energy may enter the nasal branch — it stays
    /// silent so a closed velum produces no nasal sound.
    #[test]
    fn closed_velum_seals_the_nose() {
        let mut t = KellyLochbaumTract::new(40);
        t.enable_nose(28, 24);
        t.set_velum(0.0);
        for n in 0..6_000 {
            let drive = if n % 300 == 0 { 1.0 } else { 0.0 };
            t.step(drive);
        }
        let nose_energy = t
            .nose_right
            .iter()
            .chain(t.nose_left.iter())
            .fold(0.0_f32, |m, &x| m.max(x.abs()));
        assert!(
            nose_energy < 1e-12,
            "closed velum leaked into nose: {nose_energy}"
        );
    }

    /// A closed velum must leave the oral response untouched: a nose-enabled
    /// tract with `velum = 0` radiates exactly like the oral-only path, even on
    /// a non-uniform (articulated) area profile.
    #[test]
    fn closed_velum_matches_oral_path() {
        let articulate = |t: &mut KellyLochbaumTract| {
            for i in 18..24 {
                t.set_diameter(i, 0.3);
            }
            t.update_reflections();
        };
        let mut oral = KellyLochbaumTract::new(40);
        articulate(&mut oral);
        let mut nasal = KellyLochbaumTract::new(40);
        articulate(&mut nasal);
        nasal.enable_nose(28, 20);
        nasal.set_velum(0.0);

        let mut max_diff = 0.0_f32;
        for n in 0..6_000 {
            let drive = if n % 300 == 0 { 1.0 } else { 0.0 };
            max_diff = max_diff.max((oral.step(drive) - nasal.step(drive)).abs());
        }
        assert!(
            max_diff < 1e-6,
            "closed velum perturbed the oral path: max_diff={max_diff}"
        );
    }

    /// Opening the velum couples the nasal branch: it fills with energy and the
    /// radiated output differs from the closed case. Both stay bounded.
    #[test]
    fn open_velum_couples_and_radiates() {
        let render = |opening: f32| {
            let mut t = KellyLochbaumTract::new(40);
            t.enable_nose(28, 24);
            t.set_velum(opening);
            let mut sum = 0.0_f32;
            let mut max = 0.0_f32;
            for n in 0..8_000 {
                let drive = if n % 300 == 0 { 1.0 } else { 0.0 };
                t.step(drive);
                let out = t.step(0.0);
                sum += out.abs();
                max = max.max(out.abs());
            }
            let nose_energy = t.nose_right.iter().fold(0.0_f32, |m, &x| m.max(x.abs()));
            (sum, max, nose_energy)
        };
        let (closed_sum, _, _) = render(0.0);
        let (open_sum, open_max, open_nose) = render(1.0);
        assert!(open_nose > 1e-6, "open velum did not couple: {open_nose}");
        assert!(
            open_max.is_finite() && open_max < 50.0,
            "nasal tract unstable: {open_max}"
        );
        assert!(
            (open_sum - closed_sum).abs() > 1e-3,
            "velum opening had no audible effect: closed={closed_sum}, open={open_sum}"
        );
    }

    /// The nasal branch must stay bounded for every velum opening.
    #[test]
    fn nasal_branch_stable_across_velum() {
        for &opening in &[0.0, 0.25, 0.5, 0.75, 1.0] {
            let mut t = KellyLochbaumTract::new(40);
            t.enable_nose(28, 24);
            t.set_velum(opening);
            let mut max = 0.0_f32;
            for n in 0..20_000 {
                let drive = if n % 200 == 0 { 1.0 } else { 0.0 };
                let out = t.step(drive);
                max = max.max(out.abs());
            }
            assert!(
                max.is_finite() && max < 50.0,
                "nasal tract unstable at velum={opening}: max={max}"
            );
        }
    }

    /// Stress the intermediate-opening crossfade with a pathological profile —
    /// a tight constriction straddling the velum and a wide nose — to confirm
    /// the partly-open junction stays bounded for non-uniform areas.
    #[test]
    fn nasal_branch_stable_with_constriction_at_velum() {
        for &opening in &[0.3, 0.5, 0.7, 1.0] {
            let mut t = KellyLochbaumTract::new(40);
            t.enable_nose(28, 24);
            t.set_diameter(23, 0.06);
            t.set_diameter(24, 2.5);
            t.update_reflections();
            for i in 0..28 {
                t.set_nose_diameter(i, 2.0);
            }
            t.update_nose_reflections();
            t.set_velum(opening);
            let mut max = 0.0_f32;
            for n in 0..20_000 {
                let drive = if n % 200 == 0 { 1.0 } else { 0.0 };
                max = max.max(t.step(drive).abs());
            }
            assert!(
                max.is_finite() && max < 50.0,
                "nasal tract unstable (constricted) at velum={opening}: max={max}"
            );
        }
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
