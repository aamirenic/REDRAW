//! REDRAW mixer: 64 channel strips, 4 send buses, master chain, lock-free metering.
//!
//! All buffers are preallocated. `process_strip`/`finish` never allocate; structural
//! changes (installing/removing effects) are control-thread operations handed to the
//! engine through commands and installed at block boundaries.
#![forbid(unsafe_code)]

use redraw_dsp::Limiter;
use redraw_shared::{db_to_gain, pan_gains, Effect, BLOCK_SIZE, MAX_INSERTS, MAX_SENDS};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

pub const MAX_STRIPS: usize = 64;

// ------------------------------------------------------------------ meters

/// Lock-free meters shared between the audio thread and the UI.
#[derive(Debug)]
pub struct Meters {
    strip_peak: Vec<AtomicU32>,
    master_peak_l: AtomicU32,
    master_peak_r: AtomicU32,
    pub voices: AtomicU32,
    pub cpu_permille: AtomicU32,
    /// Engine transport position in ticks, stored as f64 bits.
    pub position_ticks: AtomicU64,
    pub playing: AtomicU32,
}

impl Meters {
    pub fn new(strips: usize) -> Self {
        Self {
            strip_peak: (0..strips).map(|_| AtomicU32::new(0)).collect(),
            master_peak_l: AtomicU32::new(0),
            master_peak_r: AtomicU32::new(0),
            voices: AtomicU32::new(0),
            cpu_permille: AtomicU32::new(0),
            position_ticks: AtomicU64::new(0f64.to_bits()),
            playing: AtomicU32::new(0),
        }
    }
    #[inline]
    pub fn store_peak(&self, strip: usize, peak: f32) {
        if let Some(a) = self.strip_peak.get(strip) {
            a.store(peak.clamp(0.0, 16.0).to_bits(), Ordering::Relaxed);
        }
    }
    #[inline]
    pub fn peak(&self, strip: usize) -> f32 {
        self.strip_peak
            .get(strip)
            .map(|a| f32::from_bits(a.load(Ordering::Relaxed)))
            .unwrap_or(0.0)
    }
    #[inline]
    pub fn store_master(&self, l: f32, r: f32) {
        self.master_peak_l
            .store(l.clamp(0.0, 16.0).to_bits(), Ordering::Relaxed);
        self.master_peak_r
            .store(r.clamp(0.0, 16.0).to_bits(), Ordering::Relaxed);
    }
    #[inline]
    pub fn master(&self) -> (f32, f32) {
        (
            f32::from_bits(self.master_peak_l.load(Ordering::Relaxed)),
            f32::from_bits(self.master_peak_r.load(Ordering::Relaxed)),
        )
    }
    pub fn store_position(&self, ticks: f64) {
        self.position_ticks.store(ticks.to_bits(), Ordering::Relaxed);
    }
    pub fn position(&self) -> f64 {
        f64::from_bits(self.position_ticks.load(Ordering::Relaxed))
    }
    pub fn set_playing(&self, playing: bool) {
        self.playing.store(playing as u32, Ordering::Relaxed);
    }
    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed) != 0
    }
    pub fn set_voices(&self, v: usize) {
        self.voices.store(v as u32, Ordering::Relaxed);
    }
    pub fn voice_count(&self) -> usize {
        self.voices.load(Ordering::Relaxed) as usize
    }
}

// ------------------------------------------------------------------ inserts

pub struct InsertChain {
    slots: [Option<Box<dyn Effect>>; MAX_INSERTS],
}

impl InsertChain {
    pub fn new() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
        }
    }
    /// Install/replace an effect. Returns the previous occupant (dropped by the caller,
    /// never in the audio callback — the engine routes it to the garbage ring).
    pub fn set(&mut self, slot: usize, effect: Option<Box<dyn Effect>>) -> Option<Box<dyn Effect>> {
        if slot < MAX_INSERTS {
            std::mem::replace(&mut self.slots[slot], effect)
        } else {
            effect
        }
    }
    pub fn take(&mut self, slot: usize) -> Option<Box<dyn Effect>> {
        if slot < MAX_INSERTS {
            self.slots[slot].take()
        } else {
            None
        }
    }
    pub fn get(&self, slot: usize) -> Option<&dyn Effect> {
        self.slots.get(slot).and_then(|s| s.as_deref())
    }
    pub fn get_mut(&mut self, slot: usize) -> Option<&mut (dyn Effect + 'static)> {
        match self.slots.get_mut(slot) {
            Some(Some(b)) => Some(b.as_mut()),
            _ => None,
        }
    }
    #[inline]
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], sr: f32, tempo: f32) {
        for fx in self.slots.iter_mut().flatten() {
            if !fx.is_bypassed() {
                fx.process(l, r, sr, tempo);
            }
        }
    }
    pub fn set_param(&mut self, slot: usize, index: usize, value: f32) {
        if let Some(Some(fx)) = self.slots.get_mut(slot) {
            fx.set_param(index, value);
        }
    }
    pub fn set_bypass(&mut self, slot: usize, bypass: bool) {
        if let Some(Some(fx)) = self.slots.get_mut(slot) {
            fx.set_bypass(bypass);
        }
    }
    pub fn clear(&mut self) -> impl Iterator<Item = Box<dyn Effect>> + '_ {
        self.slots.iter_mut().filter_map(|s| s.take())
    }
    pub fn count(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }
    pub fn type_names(&self) -> Vec<(usize, &'static str)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, f)| f.as_ref().map(|f| (i, f.effect_type())))
            .collect()
    }
}

impl Default for InsertChain {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for InsertChain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<_> = self
            .slots
            .iter()
            .map(|s| s.as_ref().map(|e| e.effect_type()).unwrap_or("-"))
            .collect();
        f.debug_struct("InsertChain").field("slots", &names).finish()
    }
}

// ------------------------------------------------------------------ sends

#[derive(Clone, Copy, Debug)]
pub struct SendRt {
    pub level_db: f32,
    pub pre_fader: bool,
    pub target: u8,
}

impl Default for SendRt {
    fn default() -> Self {
        Self {
            level_db: -90.0,
            pre_fader: false,
            target: 0,
        }
    }
}

// ------------------------------------------------------------------ strip

#[derive(Clone, Copy, Debug, Default)]
pub struct StripLevels {
    pub peak_l: f32,
    pub peak_r: f32,
    pub rms_l: f32,
    pub rms_r: f32,
}

pub struct StripRt {
    pub gain_db: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub polarity: bool,
    pub inserts: InsertChain,
    pub sends: [SendRt; MAX_SENDS],
    pre_scratch_l: [[f32; BLOCK_SIZE]; MAX_SENDS],
    pre_scratch_r: [[f32; BLOCK_SIZE]; MAX_SENDS],
    pub levels: StripLevels,
}

impl StripRt {
    pub fn new() -> Self {
        Self {
            gain_db: 0.0,
            pan: 0.0,
            mute: false,
            solo: false,
            polarity: false,
            inserts: InsertChain::new(),
            sends: [SendRt::default(); MAX_SENDS],
            pre_scratch_l: [[0.0; BLOCK_SIZE]; MAX_SENDS],
            pre_scratch_r: [[0.0; BLOCK_SIZE]; MAX_SENDS],
            levels: StripLevels::default(),
        }
    }
}

// ------------------------------------------------------------------ bus

pub struct BusRt {
    pub gain_db: f32,
    pub mute: bool,
    pub inserts: InsertChain,
    buf_l: [f32; BLOCK_SIZE],
    buf_r: [f32; BLOCK_SIZE],
    peak_l: f32,
    peak_r: f32,
}

impl BusRt {
    pub fn new() -> Self {
        Self {
            gain_db: 0.0,
            mute: false,
            inserts: InsertChain::new(),
            buf_l: [0.0; BLOCK_SIZE],
            buf_r: [0.0; BLOCK_SIZE],
            peak_l: 0.0,
            peak_r: 0.0,
        }
    }
    pub fn clear(&mut self) {
        self.buf_l.fill(0.0);
        self.buf_r.fill(0.0);
        self.peak_l = 0.0;
        self.peak_r = 0.0;
    }
}

// ------------------------------------------------------------------ mixer

pub struct MixerRt {
    pub strips: Vec<StripRt>,
    pub buses: Vec<BusRt>,
    pub master_inserts: InsertChain,
    pub master_gain_db: f32,
    pub limiter: Limiter,
    pub meters: Arc<Meters>,
    master_l: [f32; BLOCK_SIZE],
    master_r: [f32; BLOCK_SIZE],
    master_peak_l: f32,
    master_peak_r: f32,
    any_solo: bool,
}

impl MixerRt {
    pub fn new(strip_count: usize, sr: f32) -> Self {
        let count = strip_count.clamp(1, MAX_STRIPS);
        Self {
            strips: (0..count).map(|_| StripRt::new()).collect(),
            buses: (0..MAX_SENDS).map(|_| BusRt::new()).collect(),
            master_inserts: InsertChain::new(),
            master_gain_db: 0.0,
            limiter: Limiter::new(sr),
            meters: Arc::new(Meters::new(count)),
            master_l: [0.0; BLOCK_SIZE],
            master_r: [0.0; BLOCK_SIZE],
            master_peak_l: 0.0,
            master_peak_r: 0.0,
            any_solo: false,
        }
    }
    pub fn strip_count(&self) -> usize {
        self.strips.len()
    }
    pub fn recompute_solo(&mut self) {
        self.any_solo = self.strips.iter().any(|s| s.solo);
    }
    pub fn any_solo(&self) -> bool {
        self.any_solo
    }
    pub fn reset(&mut self) {
        for b in &mut self.buses {
            b.clear();
        }
        for fx in self.master_inserts.slots.iter_mut().flatten() {
            fx.reset();
        }
        self.limiter.reset();
    }
    /// Begin a processing block (zeroes buses/master, resets peak accumulators).
    pub fn begin_block(&mut self) {
        for b in &mut self.buses {
            b.clear();
        }
        self.master_l.fill(0.0);
        self.master_r.fill(0.0);
        self.master_peak_l = 0.0;
        self.master_peak_r = 0.0;
        for s in &mut self.strips {
            s.levels = StripLevels::default();
        }
    }
    /// Process one channel strip through inserts, sends and fader, summing into master.
    /// `l`/`r` are modified in place to the post-fader signal.
    pub fn process_strip(
        &mut self,
        index: usize,
        l: &mut [f32],
        r: &mut [f32],
        sr: f32,
        tempo: f32,
    ) {
        let n = l.len().min(r.len()).min(BLOCK_SIZE);
        if index >= self.strips.len() || n == 0 {
            return;
        }
        let any_solo = self.any_solo;
        // --- inserts + pre-fader sends + fader (strip borrowed in a tight scope) ---
        {
            let strip = &mut self.strips[index];
            strip.inserts.process(&mut l[..n], &mut r[..n], sr, tempo);
            let active = !strip.mute && (!any_solo || strip.solo);
            let gain = db_to_gain(strip.gain_db);
            let (pl, pr) = pan_gains(strip.pan);
            let pol = if strip.polarity { -1.0 } else { 1.0 };
            for s in 0..MAX_SENDS {
                let send = strip.sends[s];
                let sg = db_to_gain(send.level_db);
                if send.pre_fader {
                    let dst_l = &mut strip.pre_scratch_l[s];
                    let dst_r = &mut strip.pre_scratch_r[s];
                    for i in 0..n {
                        dst_l[i] = l[i] * sg;
                        dst_r[i] = r[i] * sg;
                    }
                }
            }
            let mut peak_l = 0.0f32;
            let mut peak_r = 0.0f32;
            let mut sum_l = 0.0f32;
            let mut sum_r = 0.0f32;
            for i in 0..n {
                let gl = l[i] * gain * pl * pol;
                let gr = r[i] * gain * pr * pol;
                let (ol, or_) = if active { (gl, gr) } else { (0.0, 0.0) };
                l[i] = ol;
                r[i] = or_;
                peak_l = peak_l.max(ol.abs());
                peak_r = peak_r.max(or_.abs());
                sum_l += ol * ol;
                sum_r += or_ * or_;
            }
            // A callback may be rendered as several control-rate chunks; keep the
            // loudest chunk so the UI meter does not collapse to the last one.
            strip.levels.peak_l = strip.levels.peak_l.max(peak_l);
            strip.levels.peak_r = strip.levels.peak_r.max(peak_r);
            strip.levels.rms_l = strip.levels.rms_l.max((sum_l / n as f32).sqrt());
            strip.levels.rms_r = strip.levels.rms_r.max((sum_r / n as f32).sqrt());
            // fill post-fader send taps
            for s in 0..MAX_SENDS {
                let send = strip.sends[s];
                if !send.pre_fader {
                    let sg = db_to_gain(send.level_db);
                    let dst_l = &mut strip.pre_scratch_l[s];
                    let dst_r = &mut strip.pre_scratch_r[s];
                    for i in 0..n {
                        dst_l[i] = l[i] * sg;
                        dst_r[i] = r[i] * sg;
                    }
                }
            }
        }
        // --- sends into buses (disjoint field borrows) ---
        for s in 0..MAX_SENDS {
            let send = self.strips[index].sends[s];
            let target = (send.target as usize).min(MAX_SENDS - 1);
            let bus = &mut self.buses[target];
            let src_l = &self.strips[index].pre_scratch_l[s];
            let src_r = &self.strips[index].pre_scratch_r[s];
            for i in 0..n {
                bus.buf_l[i] += src_l[i];
                bus.buf_r[i] += src_r[i];
            }
        }
        // --- sum into master ---
        for i in 0..n {
            self.master_l[i] += l[i];
            self.master_r[i] += r[i];
            self.master_peak_l = self.master_peak_l.max(l[i].abs());
            self.master_peak_r = self.master_peak_r.max(r[i].abs());
        }
    }
    /// Process buses, master inserts, master gain and limiter; write the final stereo
    /// block into `out_l`/`out_r`.
    pub fn finish(&mut self, out_l: &mut [f32], out_r: &mut [f32], n: usize, sr: f32, tempo: f32) {
        let n = n.min(out_l.len()).min(out_r.len()).min(BLOCK_SIZE);
        if n == 0 {
            return;
        }
        for b in &mut self.buses {
            if b.mute {
                continue;
            }
            b.inserts.process(&mut b.buf_l[..n], &mut b.buf_r[..n], sr, tempo);
            let g = db_to_gain(b.gain_db);
            for i in 0..n {
                self.master_l[i] += b.buf_l[i] * g;
                self.master_r[i] += b.buf_r[i] * g;
                b.peak_l = b.peak_l.max((b.buf_l[i] * g).abs());
                b.peak_r = b.peak_r.max((b.buf_r[i] * g).abs());
            }
        }
        let mg = db_to_gain(self.master_gain_db);
        for i in 0..n {
            self.master_l[i] *= mg;
            self.master_r[i] *= mg;
        }
        self.master_inserts
            .process(&mut self.master_l[..n], &mut self.master_r[..n], sr, tempo);
        self.limiter
            .process(&mut self.master_l[..n], &mut self.master_r[..n], sr);
        for i in 0..n {
            let l = self.master_l[i];
            let r = self.master_r[i];
            out_l[i] = l;
            out_r[i] = r;
            self.master_peak_l = self.master_peak_l.max(l.abs());
            self.master_peak_r = self.master_peak_r.max(r.abs());
        }
    }
    /// Publish meter values to the lock-free UI snapshot.
    pub fn publish_meters(&self) {
        for (i, s) in self.strips.iter().enumerate() {
            let peak = s.levels.peak_l.max(s.levels.peak_r);
            self.meters.store_peak(i, peak);
        }
        self.meters
            .store_master(self.master_peak_l, self.master_peak_r);
    }
    pub fn set_gain_db(&mut self, strip: usize, db: f32) {
        if let Some(s) = self.strips.get_mut(strip) {
            s.gain_db = db.clamp(-90.0, 12.0);
        }
    }
    pub fn set_pan(&mut self, strip: usize, pan: f32) {
        if let Some(s) = self.strips.get_mut(strip) {
            s.pan = pan.clamp(-1.0, 1.0);
        }
    }
    pub fn set_mute(&mut self, strip: usize, mute: bool) {
        if let Some(s) = self.strips.get_mut(strip) {
            s.mute = mute;
        }
    }
    pub fn set_solo(&mut self, strip: usize, solo: bool) {
        if let Some(s) = self.strips.get_mut(strip) {
            s.solo = solo;
        }
        self.recompute_solo();
    }
    pub fn set_send(&mut self, strip: usize, send: usize, level_db: f32, pre: bool, target: u8) {
        if let Some(s) = self.strips.get_mut(strip) {
            if send < MAX_SENDS {
                s.sends[send] = SendRt {
                    level_db: level_db.clamp(-90.0, 12.0),
                    pre_fader: pre,
                    target: target.min((MAX_SENDS - 1) as u8),
                };
            }
        }
    }
    /// Rebuild the look-ahead limiter for a new sample rate. Setup-time only.
    pub fn set_sample_rate(&mut self, sr: f32) {
        self.limiter = Limiter::new(sr);
    }
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use redraw_shared::{ParamSpec, SAMPLE_RATE};

    struct HalfGain {
        bypass: bool,
    }
    impl Default for HalfGain {
        fn default() -> Self {
            Self { bypass: false }
        }
    }
    impl Effect for HalfGain {
        fn effect_type(&self) -> &'static str {
            "HalfGain"
        }
        fn reset(&mut self) {}
        fn process(&mut self, l: &mut [f32], r: &mut [f32], _sr: f32, _tempo: f32) {
            for x in l {
                *x *= 0.5;
            }
            for x in r {
                *x *= 0.5;
            }
        }
        fn params(&self) -> &'static [ParamSpec] {
            &[]
        }
        fn set_param(&mut self, _i: usize, _v: f32) {}
        fn get_param(&self, _i: usize) -> f32 {
            0.0
        }
        fn set_bypass(&mut self, b: bool) {
            self.bypass = b;
        }
        fn is_bypassed(&self) -> bool {
            self.bypass
        }
        fn controller_value(&self) -> f32 {
            0.0
        }
    }

    fn run(mixer: &mut MixerRt, l: &mut [f32], r: &mut [f32]) {
        mixer.begin_block();
        // one test channel, others silent
        let mut cl = [0.0f32; BLOCK_SIZE];
        let mut cr = [0.0f32; BLOCK_SIZE];
        cl[..l.len()].copy_from_slice(l);
        cr[..r.len()].copy_from_slice(r);
        mixer.process_strip(0, &mut cl[..l.len()], &mut cr[..r.len()], SAMPLE_RATE as f32, 140.0);
        mixer.finish(l, r, l.len(), SAMPLE_RATE as f32, 140.0);
        mixer.publish_meters();
    }

    #[test]
    fn gain_and_pan() {
        let mut m = MixerRt::new(64, SAMPLE_RATE as f32);
        m.set_pan(0, 0.0);
        let mut l = [0.5f32; 256];
        let mut r = [0.5f32; 256];
        run(&mut m, &mut l, &mut r);
        // 0.5 * 0.707 on both sides, sampled past the 72-sample limiter look-ahead
        assert!((l[200] - 0.3535).abs() < 0.01, "l={}", l[200]);
        assert!((r[200] - 0.3535).abs() < 0.01);
        m.set_gain_db(0, -6.0);
        let mut l2 = [0.5f32; 256];
        let mut r2 = [0.5f32; 256];
        run(&mut m, &mut l2, &mut r2);
        assert!((l2[200] - 0.177).abs() < 0.01, "l2={}", l2[200]);
    }

    #[test]
    fn mute_and_solo() {
        let mut m = MixerRt::new(64, SAMPLE_RATE as f32);
        m.set_mute(0, true);
        let mut l = [0.5f32; 256];
        let mut r = [0.5f32; 256];
        run(&mut m, &mut l, &mut r);
        assert!(l[200].abs() < 1e-6);
        m.set_mute(0, false);
        m.set_solo(1, true); // strip 0 is not soloed
        let mut l2 = [0.5f32; 256];
        let mut r2 = [0.5f32; 256];
        run(&mut m, &mut l2, &mut r2);
        assert!(l2[200].abs() < 1e-6);
        m.set_solo(0, true);
        let mut l3 = [0.5f32; 256];
        let mut r3 = [0.5f32; 256];
        run(&mut m, &mut l3, &mut r3);
        assert!(l3[200].abs() > 0.01);
    }

    #[test]
    fn insert_processing_and_bypass() {
        let mut m = MixerRt::new(8, SAMPLE_RATE as f32);
        m.strips[0].inserts.set(0, Some(Box::new(HalfGain::default())));
        let mut l = [1.0f32; 256];
        let mut r = [1.0f32; 256];
        run(&mut m, &mut l, &mut r);
        assert!((l[200] - 0.3535).abs() < 0.01);
        m.strips[0].inserts.set_bypass(0, true);
        let mut l2 = [1.0f32; 256];
        let mut r2 = [1.0f32; 256];
        run(&mut m, &mut l2, &mut r2);
        assert!((l2[200] - 0.707).abs() < 0.01, "l2={}", l2[200]);
    }

    #[test]
    fn send_bus_routing() {
        let mut m = MixerRt::new(4, SAMPLE_RATE as f32);
        m.set_send(0, 0, 0.0, false, 0); // post-fader, unity
        m.strips[0].gain_db = -90.0; // dry path silent, send still post-fader -> silent too
        let mut l = [1.0f32; 256];
        let mut r = [1.0f32; 256];
        run(&mut m, &mut l, &mut r);
        assert!(l[200].abs() < 1e-4);
        // pre-fader send survives a closed fader
        m.set_send(0, 0, 0.0, true, 0);
        let mut l2 = [1.0f32; 256];
        let mut r2 = [1.0f32; 256];
        run(&mut m, &mut l2, &mut r2);
        assert!(l2[200].abs() > 0.1, "pre-fader send missing l2={}", l2[200]);
    }

    #[test]
    fn master_limiter_caps_output() {
        let mut m = MixerRt::new(4, SAMPLE_RATE as f32);
        m.master_gain_db = 24.0;
        let mut l = [1.0f32; 256];
        let mut r = [1.0f32; 256];
        run(&mut m, &mut l, &mut r);
        for v in l.iter().chain(r.iter()) {
            assert!(v.abs() <= 10f32.powf(-0.3 / 20.0) + 1e-4, "v={v}");
        }
    }

    #[test]
    fn sixty_four_strips() {
        let mut m = MixerRt::new(64, SAMPLE_RATE as f32);
        m.begin_block();
        for i in 0..64 {
            let mut l = [0.001f32; 64];
            let mut r = [0.001f32; 64];
            m.process_strip(i, &mut l, &mut r, 48_000.0, 120.0);
        }
        let (mut l, mut r) = ([0.0f32; 64], [0.0f32; 64]);
        m.finish(&mut l, &mut r, 64, 48_000.0, 120.0);
        assert!(l.iter().all(|x| x.is_finite()));
        assert_eq!(m.strip_count(), 64);
    }
}
