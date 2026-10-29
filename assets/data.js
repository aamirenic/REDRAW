/* Generated from redraw/PRODUCTION.md by tools/build-data.mjs. */
window.REDRAW_DATA = {
  "meta": {
    "name": "REDRAW",
    "tagline": "Rust-Engineered Digital Rhythmic Audio Workstation",
    "instrumentCount": 49,
    "effectCount": 74
  },
  "families": {
    "S": {
      "name": "Subtractive",
      "blurb": "Three oscillators, noise, a state-variable filter, two envelopes, two LFOs, unison and glide."
    },
    "W": {
      "name": "Wavetable",
      "blurb": "Two table oscillators over six generated tables, crossblend, unison, filter and vibrato."
    },
    "F": {
      "name": "FM",
      "blurb": "Six operators, four algorithms, feedback, a filter and two envelopes."
    },
    "A": {
      "name": "Additive / formant",
      "blurb": "Sixteen partials, five vowel formants, prism drift, blur and vibrato."
    },
    "P": {
      "name": "Physical",
      "blurb": "Two detuned damped strings, pick excitation, body tone and a sub oscillator."
    },
    "D": {
      "name": "Drum",
      "blurb": "Six synthesized drum voices plus kit key-mapping."
    },
    "G": {
      "name": "Granular",
      "blurb": "A sixteen-grain cloud over generated tone and noise beds, with window, spray and spread."
    },
    "X": {
      "name": "Sampler",
      "blurb": "Twelve-voice sample playback with slices, three loop modes, filter envelope and pitch."
    }
  },
  "cores": {
    "EQ": {
      "name": "Tone & filter",
      "blurb": "Biquad banks, shelves, graphic bands and morphing filters."
    },
    "DYN": {
      "name": "Dynamics",
      "blurb": "Compression, limiting, transient shaping and emphasis."
    },
    "DST": {
      "name": "Distortion",
      "blurb": "Soft clipping, overdrive, bitcrushing and transfer curves."
    },
    "MOD": {
      "name": "Modulation",
      "blurb": "Chorus, phaser, flanger, auto-pan and stereo shaping."
    },
    "DLY": {
      "name": "Delay",
      "blurb": "Tempo-synced delays, tap banks and multi-band echoes."
    },
    "REV": {
      "name": "Reverb",
      "blurb": "Algorithmic rooms, halls and partitioned convolution."
    },
    "PIT": {
      "name": "Pitch & time",
      "blurb": "Pitch correction, shifting, time manipulation and tape effects."
    },
    "V": {
      "name": "Vocoder",
      "blurb": "Sixteen- and thirty-two-band vocoding."
    },
    "ANL": {
      "name": "Analysis",
      "blurb": "Scopes, spectra, meters, tuners and envelope followers."
    },
    "ANL+UTL": {
      "name": "Analysis + utility",
      "blurb": "Buffer inspection and editing with utility routing."
    },
    "UTL": {
      "name": "Utility",
      "blurb": "Trim, mute, send, notes, clocks and modulation utilities."
    }
  },
  "instruments": [
    {
      "id": 1,
      "name": "Aurora",
      "code": "W",
      "capability": "Wavetable synth",
      "family": "Wavetable",
      "blurb": "Two table oscillators over six generated tables, crossblend, unison, filter and vibrato."
    },
    {
      "id": 2,
      "name": "Hexad FM",
      "code": "F",
      "capability": "Six-operator FM flagship",
      "family": "FM",
      "blurb": "Six operators, four algorithms, feedback, a filter and two envelopes."
    },
    {
      "id": 3,
      "name": "Triple Osc",
      "code": "S",
      "capability": "Three-oscillator subtractive synth",
      "family": "Subtractive",
      "blurb": "Three oscillators, noise, a state-variable filter, two envelopes, two LFOs, unison and glide."
    },
    {
      "id": 4,
      "name": "Prism",
      "code": "A",
      "capability": "Additive harmonic flagship",
      "family": "Additive / formant",
      "blurb": "Sixteen partials, five vowel formants, prism drift, blur and vibrato."
    },
    {
      "id": 5,
      "name": "Vowel",
      "code": "A",
      "capability": "Vowel formant synth",
      "family": "Additive / formant",
      "blurb": "Sixteen partials, five vowel formants, prism drift, blur and vibrato."
    },
    {
      "id": 6,
      "name": "Prism Lite",
      "code": "A",
      "capability": "Harmonic subtractive synth",
      "family": "Additive / formant",
      "blurb": "Sixteen partials, five vowel formants, prism drift, blur and vibrato."
    },
    {
      "id": 7,
      "name": "Shapeshift",
      "code": "A",
      "capability": "Additive morph synth",
      "family": "Additive / formant",
      "blurb": "Sixteen partials, five vowel formants, prism drift, blur and vibrato."
    },
    {
      "id": 8,
      "name": "Wildcard",
      "code": "W",
      "capability": "Bright wavetable synth",
      "family": "Wavetable",
      "blurb": "Two table oscillators over six generated tables, crossblend, unison, filter and vibrato."
    },
    {
      "id": 9,
      "name": "Forge",
      "code": "A",
      "capability": "Bright additive synth",
      "family": "Additive / formant",
      "blurb": "Sixteen partials, five vowel formants, prism drift, blur and vibrato."
    },
    {
      "id": 10,
      "name": "Circuit Bass",
      "code": "S",
      "capability": "resonant acid bass",
      "family": "Subtractive",
      "blurb": "Three oscillators, noise, a state-variable filter, two envelopes, two LFOs, unison and glide."
    },
    {
      "id": 11,
      "name": "Ember FM",
      "code": "F",
      "capability": "Six-operator FM with filter",
      "family": "FM",
      "blurb": "Six operators, four algorithms, feedback, a filter and two envelopes."
    },
    {
      "id": 12,
      "name": "DX Pair",
      "code": "F",
      "capability": "Two-operator FM preset",
      "family": "FM",
      "blurb": "Six operators, four algorithms, feedback, a filter and two envelopes."
    },
    {
      "id": 13,
      "name": "Orbit",
      "code": "S",
      "capability": "Vintage polysynth",
      "family": "Subtractive",
      "blurb": "Three oscillators, noise, a state-variable filter, two envelopes, two LFOs, unison and glide."
    },
    {
      "id": 14,
      "name": "Orbit X",
      "code": "S",
      "capability": "Vintage polysynth variant",
      "family": "Subtractive",
      "blurb": "Three oscillators, noise, a state-variable filter, two envelopes, two LFOs, unison and glide."
    },
    {
      "id": 15,
      "name": "Petal",
      "code": "P",
      "capability": "Plucked-string physical model",
      "family": "Physical",
      "blurb": "Two detuned damped strings, pick excitation, body tone and a sub oscillator."
    },
    {
      "id": 16,
      "name": "Sawline",
      "code": "S",
      "capability": "Unison saw synth",
      "family": "Subtractive",
      "blurb": "Three oscillators, noise, a state-variable filter, two envelopes, two LFOs, unison and glide."
    },
    {
      "id": 17,
      "name": "Zone",
      "code": "S",
      "capability": "Two-oscillator subtractive",
      "family": "Subtractive",
      "blurb": "Three oscillators, noise, a state-variable filter, two envelopes, two LFOs, unison and glide."
    },
    {
      "id": 18,
      "name": "Simplex",
      "code": "S",
      "capability": "Two-oscillator plus sub",
      "family": "Subtractive",
      "blurb": "Three oscillators, noise, a state-variable filter, two envelopes, two LFOs, unison and glide."
    },
    {
      "id": 19,
      "name": "Stinger",
      "code": "S",
      "capability": "Gritty two-oscillator synth",
      "family": "Subtractive",
      "blurb": "Three oscillators, noise, a state-variable filter, two envelopes, two LFOs, unison and glide."
    },
    {
      "id": 20,
      "name": "Stinger XT",
      "code": "S",
      "capability": "Three-oscillator synth",
      "family": "Subtractive",
      "blurb": "Three oscillators, noise, a state-variable filter, two envelopes, two LFOs, unison and glide."
    },
    {
      "id": 21,
      "name": "MiniVee",
      "code": "S",
      "capability": "warm three-oscillator synth",
      "family": "Subtractive",
      "blurb": "Three oscillators, noise, a state-variable filter, two envelopes, two LFOs, unison and glide."
    },
    {
      "id": 22,
      "name": "VoxForm",
      "code": "A",
      "capability": "Formant vocalizer",
      "family": "Additive / formant",
      "blurb": "Sixteen partials, five vowel formants, prism drift, blur and vibrato."
    },
    {
      "id": 23,
      "name": "Spectral Map",
      "code": "A",
      "capability": "Harmonic bitmap-style synth",
      "family": "Additive / formant",
      "blurb": "Sixteen partials, five vowel formants, prism drift, blur and vibrato."
    },
    {
      "id": 24,
      "name": "Multiwave",
      "code": "X",
      "capability": "Sampler voice with generated bed",
      "family": "Sampler",
      "blurb": "Twelve-voice sample playback with slices, three loop modes, filter envelope and pitch."
    },
    {
      "id": 25,
      "name": "PadForge",
      "code": "D",
      "capability": "Sixteen-pad drum module",
      "family": "Drum",
      "blurb": "Six synthesized drum voices plus kit key-mapping."
    },
    {
      "id": 26,
      "name": "KickOne",
      "code": "D",
      "capability": "Sine kick synth",
      "family": "Drum",
      "blurb": "Six synthesized drum voices plus kit key-mapping."
    },
    {
      "id": 27,
      "name": "SubKick",
      "code": "D",
      "capability": "long-tail kick synth",
      "family": "Drum",
      "blurb": "Six synthesized drum voices plus kit key-mapping."
    },
    {
      "id": 28,
      "name": "DrumCell",
      "code": "D",
      "capability": "Four-partial drum voice",
      "family": "Drum",
      "blurb": "Six synthesized drum voices plus kit key-mapping."
    },
    {
      "id": 29,
      "name": "Membrane",
      "code": "D",
      "capability": "Physical drum model",
      "family": "Drum",
      "blurb": "Six synthesized drum voices plus kit key-mapping."
    },
    {
      "id": 30,
      "name": "DrumLab",
      "code": "D",
      "capability": "Eight-voice drum kit",
      "family": "Drum",
      "blurb": "Six synthesized drum voices plus kit key-mapping."
    },
    {
      "id": 31,
      "name": "GrainMill",
      "code": "G",
      "capability": "Granular texture synth",
      "family": "Granular",
      "blurb": "A sixteen-grain cloud over generated tone and noise beds, with window, spray and spread."
    },
    {
      "id": 32,
      "name": "ChopLab",
      "code": "X",
      "capability": "16-slice transient slicer",
      "family": "Sampler",
      "blurb": "Twelve-voice sample playback with slices, three loop modes, filter envelope and pitch."
    },
    {
      "id": 33,
      "name": "ChopLab 2",
      "code": "X",
      "capability": "32-slice chop player",
      "family": "Sampler",
      "blurb": "Twelve-voice sample playback with slices, three loop modes, filter envelope and pitch."
    },
    {
      "id": 34,
      "name": "SliceLab",
      "code": "X",
      "capability": "32-slice player",
      "family": "Sampler",
      "blurb": "Twelve-voice sample playback with slices, three loop modes, filter envelope and pitch."
    },
    {
      "id": 35,
      "name": "WarpTrail",
      "code": "G",
      "capability": "Granular texture synth",
      "family": "Granular",
      "blurb": "A sixteen-grain cloud over generated tone and noise beds, with window, spray and spread."
    },
    {
      "id": 36,
      "name": "Sampler",
      "code": "X",
      "capability": "Single-sample player",
      "family": "Sampler",
      "blurb": "Twelve-voice sample playback with slices, three loop modes, filter envelope and pitch."
    },
    {
      "id": 37,
      "name": "Soundfont Player",
      "code": "X",
      "capability": "Sampler voice with ensemble fallback",
      "family": "Sampler",
      "blurb": "Twelve-voice sample playback with slices, three loop modes, filter envelope and pitch."
    },
    {
      "id": 38,
      "name": "KeyForge",
      "code": "A",
      "capability": "Electric piano",
      "family": "Additive / formant",
      "blurb": "Sixteen partials, five vowel formants, prism drift, blur and vibrato."
    },
    {
      "id": 39,
      "name": "StringStorm",
      "code": "P",
      "capability": "Karplus-Strong guitar",
      "family": "Physical",
      "blurb": "Two detuned damped strings, pick excitation, body tone and a sub oscillator."
    },
    {
      "id": 40,
      "name": "PluckBass",
      "code": "P",
      "capability": "Plucked bass",
      "family": "Physical",
      "blurb": "Two detuned damped strings, pick excitation, body tone and a sub oscillator."
    },
    {
      "id": 41,
      "name": "Groove Engine",
      "code": "S",
      "capability": "Groovebox-style synth",
      "family": "Subtractive",
      "blurb": "Three oscillators, noise, a state-variable filter, two envelopes, two LFOs, unison and glide."
    },
    {
      "id": 42,
      "name": "VocalDance",
      "code": "A",
      "capability": "Vocal formant synth",
      "family": "Additive / formant",
      "blurb": "Sixteen partials, five vowel formants, prism drift, blur and vibrato."
    },
    {
      "id": 43,
      "name": "AcidBox",
      "code": "S",
      "capability": "Acid mono bass",
      "family": "Subtractive",
      "blurb": "Three oscillators, noise, a state-variable filter, two envelopes, two LFOs, unison and glide."
    },
    {
      "id": 44,
      "name": "BassBox",
      "code": "S",
      "capability": "Sub/saw bass",
      "family": "Subtractive",
      "blurb": "Three oscillators, noise, a state-variable filter, two envelopes, two LFOs, unison and glide."
    },
    {
      "id": 45,
      "name": "DrumBox",
      "code": "D",
      "capability": "Drum-kit voice",
      "family": "Drum",
      "blurb": "Six synthesized drum voices plus kit key-mapping."
    },
    {
      "id": 46,
      "name": "DrumBox B",
      "code": "D",
      "capability": "Alternate drum-kit voice",
      "family": "Drum",
      "blurb": "Six synthesized drum voices plus kit key-mapping."
    },
    {
      "id": 47,
      "name": "PadBox",
      "code": "W",
      "capability": "Warm pad synth",
      "family": "Wavetable",
      "blurb": "Two table oscillators over six generated tables, crossblend, unison, filter and vibrato."
    },
    {
      "id": 48,
      "name": "PianoBox",
      "code": "A",
      "capability": "Compact piano",
      "family": "Additive / formant",
      "blurb": "Sixteen partials, five vowel formants, prism drift, blur and vibrato."
    },
    {
      "id": 49,
      "name": "SynthBox",
      "code": "S",
      "capability": "General subtractive voice",
      "family": "Subtractive",
      "blurb": "Three oscillators, noise, a state-variable filter, two envelopes, two LFOs, unison and glide."
    }
  ],
  "effects": [
    {
      "id": 1,
      "name": "Time Loom",
      "code": "PIT",
      "capability": "128-step buffer time/gate pattern sequencer with four slots and smoothing",
      "coreName": "Pitch & time",
      "blurb": "Pitch correction, shifting, time manipulation and tape effects."
    },
    {
      "id": 2,
      "name": "Wave Bench",
      "code": "ANL+UTL",
      "capability": "Record-buffer editor with trim/fade/normalize and scrub playback",
      "coreName": "Analysis + utility",
      "blurb": "Buffer inspection and editing with utility routing."
    },
    {
      "id": 3,
      "name": "Loop Chisel",
      "code": "PIT",
      "capability": "Loop-buffer time manipulation (stretch/quantize)",
      "coreName": "Pitch & time",
      "blurb": "Pitch correction, shifting, time manipulation and tape effects."
    },
    {
      "id": 4,
      "name": "Tone Trace",
      "code": "PIT",
      "capability": "Monophonic pitch-editing view with offline analysis",
      "coreName": "Pitch & time",
      "blurb": "Pitch correction, shifting, time manipulation and tape effects."
    },
    {
      "id": 5,
      "name": "Pitch Anchor",
      "code": "PIT",
      "capability": "Real-time pitch correction/shift with formant preservation",
      "coreName": "Pitch & time",
      "blurb": "Pitch correction, shifting, time manipulation and tape effects."
    },
    {
      "id": 6,
      "name": "Pitch Drift",
      "code": "PIT",
      "capability": "Semitone pitch shifter (+/-24), grain size and feedback",
      "coreName": "Pitch & time",
      "blurb": "Pitch correction, shifting, time manipulation and tape effects."
    },
    {
      "id": 7,
      "name": "Vocal Grid",
      "code": "V",
      "capability": "16-band modulator/carrier vocoder",
      "coreName": "Vocoder",
      "blurb": "Sixteen- and thirty-two-band vocoding."
    },
    {
      "id": 8,
      "name": "Vocal Prism",
      "code": "V",
      "capability": "32-band vocoder with unvoiced passthrough and formant mix",
      "coreName": "Vocoder",
      "blurb": "Sixteen- and thirty-two-band vocoding."
    },
    {
      "id": 9,
      "name": "Filter Loom",
      "code": "MOD",
      "capability": "Eight filter units with per-unit envelope/LFO and routing",
      "coreName": "Modulation",
      "blurb": "Chorus, phaser, flanger, auto-pan and stereo shaping."
    },
    {
      "id": 10,
      "name": "Curve EQ",
      "code": "EQ",
      "capability": "Seven-band parametric EQ (peak/shelf/high-pass/low-pass) with spectrum view",
      "coreName": "Tone & filter",
      "blurb": "Biquad banks, shelves, graphic bands and morphing filters."
    },
    {
      "id": 11,
      "name": "Graphic Seven",
      "code": "EQ",
      "capability": "Seven fixed graphic bands, +/-18 dB",
      "coreName": "Tone & filter",
      "blurb": "Biquad banks, shelves, graphic bands and morphing filters."
    },
    {
      "id": 12,
      "name": "Morph EQ",
      "code": "EQ",
      "capability": "Morphable three-band EQ with tilt",
      "coreName": "Tone & filter",
      "blurb": "Biquad banks, shelves, graphic bands and morphing filters."
    },
    {
      "id": 13,
      "name": "Impulse Room",
      "code": "REV",
      "capability": "Partitioned convolution with generated impulse responses",
      "coreName": "Reverb",
      "blurb": "Algorithmic rooms, halls and partitioned convolution."
    },
    {
      "id": 14,
      "name": "Silver Room",
      "code": "REV",
      "capability": "algorithmic room: size, damping, width and dry/wet",
      "coreName": "Reverb",
      "blurb": "Algorithmic rooms, halls and partitioned convolution."
    },
    {
      "id": 15,
      "name": "Silver Room II",
      "code": "REV",
      "capability": "Room plus predelay, low/high cut, modulation and early/late mix",
      "coreName": "Reverb",
      "blurb": "Algorithmic rooms, halls and partitioned convolution."
    },
    {
      "id": 16,
      "name": "Velvet Hall",
      "code": "REV",
      "capability": "FDN hall/plate/room modes with shimmer and freeze",
      "coreName": "Reverb",
      "blurb": "Algorithmic rooms, halls and partitioned convolution."
    },
    {
      "id": 17,
      "name": "Echo One",
      "code": "DLY",
      "capability": "Mono/stereo delay with feedback, filters and ping-pong",
      "coreName": "Delay",
      "blurb": "Tempo-synced delays, tap banks and multi-band echoes."
    },
    {
      "id": 18,
      "name": "Echo Two",
      "code": "DLY",
      "capability": "Stereo delay with filters, diffusion and tempo sync",
      "coreName": "Delay",
      "blurb": "Tempo-synced delays, tap banks and multi-band echoes."
    },
    {
      "id": 19,
      "name": "Echo Three",
      "code": "DLY",
      "capability": "Dual delay lines with modulation, ducking and sync",
      "coreName": "Delay",
      "blurb": "Tempo-synced delays, tap banks and multi-band echoes."
    },
    {
      "id": 20,
      "name": "Echo Bank",
      "code": "DLY",
      "capability": "Eight-tap delay bank with per-tap time/level/pan/filter",
      "coreName": "Delay",
      "blurb": "Tempo-synced delays, tap banks and multi-band echoes."
    },
    {
      "id": 21,
      "name": "Band Echo",
      "code": "DLY",
      "capability": "Three-band delay with crossover and per-band times",
      "coreName": "Delay",
      "blurb": "Tempo-synced delays, tap banks and multi-band echoes."
    },
    {
      "id": 22,
      "name": "Glue Comp",
      "code": "DYN",
      "capability": "Threshold/ratio/attack/release/knee/makeup compressor",
      "coreName": "Dynamics",
      "blurb": "Compression, limiting, transient shaping and emphasis."
    },
    {
      "id": 23,
      "name": "Band Comp",
      "code": "DYN",
      "capability": "Three-band crossover compressor with per-band mix",
      "coreName": "Dynamics",
      "blurb": "Compression, limiting, transient shaping and emphasis."
    },
    {
      "id": 24,
      "name": "Peak Guard",
      "code": "DYN",
      "capability": "Look-ahead limiter/compressor/saturator modes",
      "coreName": "Dynamics",
      "blurb": "Compression, limiting, transient shaping and emphasis."
    },
    {
      "id": 25,
      "name": "Triad Guard",
      "code": "DYN",
      "capability": "Three-band multiband compressor/limiter with per-band curves",
      "coreName": "Dynamics",
      "blurb": "Compression, limiting, transient shaping and emphasis."
    },
    {
      "id": 26,
      "name": "Focus Comp",
      "code": "DYN",
      "capability": "Mid/side emphasis compressor and exciter",
      "coreName": "Dynamics",
      "blurb": "Compression, limiting, transient shaping and emphasis."
    },
    {
      "id": 27,
      "name": "Spectral Tilt",
      "code": "DYN",
      "capability": "Spectral emphasis/tilt with dynamics",
      "coreName": "Dynamics",
      "blurb": "Compression, limiting, transient shaping and emphasis."
    },
    {
      "id": 28,
      "name": "Soft Clip",
      "code": "DST",
      "capability": "Tanh/soft-knee clipper with threshold and output",
      "coreName": "Distortion",
      "blurb": "Soft clipping, overdrive, bitcrushing and transfer curves."
    },
    {
      "id": 29,
      "name": "Attack Shape",
      "code": "DYN",
      "capability": "Attack/sustain transient shaper",
      "coreName": "Dynamics",
      "blurb": "Compression, limiting, transient shaping and emphasis."
    },
    {
      "id": 30,
      "name": "Bit Stream",
      "code": "DLY",
      "capability": "Bit/rate transmitter with lo-fi downsampling and FX send",
      "coreName": "Delay",
      "blurb": "Tempo-synced delays, tap banks and multi-band echoes."
    },
    {
      "id": 31,
      "name": "Macro Glow",
      "code": "DYN",
      "capability": "Four macro compressor/exciter presets (A-D)",
      "coreName": "Dynamics",
      "blurb": "Compression, limiting, transient shaping and emphasis."
    },
    {
      "id": 32,
      "name": "Redline Drive",
      "code": "DST",
      "capability": "Pre-gain, X-shape and tone-stack overdrive",
      "coreName": "Distortion",
      "blurb": "Soft clipping, overdrive, bitcrushing and transfer curves."
    },
    {
      "id": 33,
      "name": "Quick Dist",
      "code": "DST",
      "capability": "Fast tanh distortion with three modes",
      "coreName": "Distortion",
      "blurb": "Soft clipping, overdrive, bitcrushing and transfer curves."
    },
    {
      "id": 34,
      "name": "Dist Stack",
      "code": "DST",
      "capability": "Six distortion modules in serial/parallel",
      "coreName": "Distortion",
      "blurb": "Soft clipping, overdrive, bitcrushing and transfer curves."
    },
    {
      "id": 35,
      "name": "Pedal Board",
      "code": "DST",
      "capability": "Pedalboard with distortion, wah, delay, compression and EQ",
      "coreName": "Distortion",
      "blurb": "Soft clipping, overdrive, bitcrushing and transfer curves."
    },
    {
      "id": 36,
      "name": "Crush Box",
      "code": "DST",
      "capability": "Bitcrusher/decimator with compressor",
      "coreName": "Distortion",
      "blurb": "Soft clipping, overdrive, bitcrushing and transfer curves."
    },
    {
      "id": 37,
      "name": "Curve Shaper",
      "code": "DST",
      "capability": "Editable 64-point transfer curve with drive and DC control",
      "coreName": "Distortion",
      "blurb": "Soft clipping, overdrive, bitcrushing and transfer curves."
    },
    {
      "id": 38,
      "name": "Chorus Three",
      "code": "MOD",
      "capability": "Three-voice chorus with depth/rate/feedback",
      "coreName": "Modulation",
      "blurb": "Chorus, phaser, flanger, auto-pan and stereo shaping."
    },
    {
      "id": 39,
      "name": "Chorus Eight",
      "code": "MOD",
      "capability": "Eight-voice ensemble with width",
      "coreName": "Modulation",
      "blurb": "Chorus, phaser, flanger, auto-pan and stereo shaping."
    },
    {
      "id": 40,
      "name": "Analog Chorus",
      "code": "MOD",
      "capability": "classic bucket-brigade chorus with optional hiss/noise",
      "coreName": "Modulation",
      "blurb": "Chorus, phaser, flanger, auto-pan and stereo shaping."
    },
    {
      "id": 41,
      "name": "Phase Six",
      "code": "MOD",
      "capability": "Six-stage all-pass phaser with feedback",
      "coreName": "Modulation",
      "blurb": "Chorus, phaser, flanger, auto-pan and stereo shaping."
    },
    {
      "id": 42,
      "name": "Analog Phaser",
      "code": "MOD",
      "capability": "Four-stage opto-style phaser with drift",
      "coreName": "Modulation",
      "blurb": "Chorus, phaser, flanger, auto-pan and stereo shaping."
    },
    {
      "id": 43,
      "name": "Flange TZ",
      "code": "MOD",
      "capability": "Through-zero-style flanger with feedback and invert",
      "coreName": "Modulation",
      "blurb": "Chorus, phaser, flanger, auto-pan and stereo shaping."
    },
    {
      "id": 44,
      "name": "Flange Chorus",
      "code": "MOD",
      "capability": "Flanger/chorus combination",
      "coreName": "Modulation",
      "blurb": "Chorus, phaser, flanger, auto-pan and stereo shaping."
    },
    {
      "id": 45,
      "name": "Auto Pan",
      "code": "MOD",
      "capability": "Tempo-syncable auto-pan with shape control",
      "coreName": "Modulation",
      "blurb": "Chorus, phaser, flanger, auto-pan and stereo shaping."
    },
    {
      "id": 46,
      "name": "Stereo Width",
      "code": "MOD",
      "capability": "Mid/side width and short-delay widening",
      "coreName": "Modulation",
      "blurb": "Chorus, phaser, flanger, auto-pan and stereo shaping."
    },
    {
      "id": 47,
      "name": "Stereo Matrix",
      "code": "MOD",
      "capability": "Per-channel delay/pan/phase matrix",
      "coreName": "Modulation",
      "blurb": "Chorus, phaser, flanger, auto-pan and stereo shaping."
    },
    {
      "id": 48,
      "name": "Band Spreader",
      "code": "MOD",
      "capability": "Multiband stereo widener",
      "coreName": "Modulation",
      "blurb": "Chorus, phaser, flanger, auto-pan and stereo shaping."
    },
    {
      "id": 49,
      "name": "Center Extract",
      "code": "UTL",
      "capability": "Center extraction / vocal removal",
      "coreName": "Utility",
      "blurb": "Trim, mute, send, notes, clocks and modulation utilities."
    },
    {
      "id": 50,
      "name": "Polarity Flip",
      "code": "UTL",
      "capability": "Per-channel polarity inversion",
      "coreName": "Utility",
      "blurb": "Trim, mute, send, notes, clocks and modulation utilities."
    },
    {
      "id": 51,
      "name": "Bass Lift",
      "code": "EQ",
      "capability": "Low shelf with harmonic enhancement",
      "coreName": "Tone & filter",
      "blurb": "Biquad banks, shelves, graphic bands and morphing filters."
    },
    {
      "id": 52,
      "name": "Sub Lift",
      "code": "EQ",
      "capability": "Subharmonic generator with low shelf",
      "coreName": "Tone & filter",
      "blurb": "Biquad banks, shelves, graphic bands and morphing filters."
    },
    {
      "id": 53,
      "name": "Freq Shift",
      "code": "PIT",
      "capability": "Hilbert-transform single-sideband frequency shift",
      "coreName": "Pitch & time",
      "blurb": "Pitch correction, shifting, time manipulation and tape effects."
    },
    {
      "id": 54,
      "name": "Band Split",
      "code": "UTL",
      "capability": "Two/three-band splitter with per-band gain for send/return use",
      "coreName": "Utility",
      "blurb": "Trim, mute, send, notes, clocks and modulation utilities."
    },
    {
      "id": 55,
      "name": "Vinyl Scratch",
      "code": "PIT",
      "capability": "Vinyl scratch simulation over a buffer",
      "coreName": "Pitch & time",
      "blurb": "Pitch correction, shifting, time manipulation and tape effects."
    },
    {
      "id": 56,
      "name": "Tape Stop",
      "code": "PIT",
      "capability": "Time/pitch transport effects (tape stop, gate)",
      "coreName": "Pitch & time",
      "blurb": "Pitch correction, shifting, time manipulation and tape effects."
    },
    {
      "id": 57,
      "name": "Signal Scope",
      "code": "ANL",
      "capability": "Scope, spectrum and peak-meter views",
      "coreName": "Analysis",
      "blurb": "Scopes, spectra, meters, tuners and envelope followers."
    },
    {
      "id": 58,
      "name": "Spectrum 31",
      "code": "ANL",
      "capability": "31-band spectrum analyzer",
      "coreName": "Analysis",
      "blurb": "Scopes, spectra, meters, tuners and envelope followers."
    },
    {
      "id": 59,
      "name": "Level Meter",
      "code": "ANL",
      "capability": "Peak/RMS/LUFS meter",
      "coreName": "Analysis",
      "blurb": "Scopes, spectra, meters, tuners and envelope followers."
    },
    {
      "id": 60,
      "name": "Session Clock",
      "code": "UTL",
      "capability": "Bar:beat:step and millisecond time display",
      "coreName": "Utility",
      "blurb": "Trim, mute, send, notes, clocks and modulation utilities."
    },
    {
      "id": 61,
      "name": "Rich Notes",
      "code": "UTL",
      "capability": "Embedded rich notes with plain-text rendering",
      "coreName": "Utility",
      "blurb": "Trim, mute, send, notes, clocks and modulation utilities."
    },
    {
      "id": 62,
      "name": "Plain Notes",
      "code": "UTL",
      "capability": "Plain-text project notes",
      "coreName": "Utility",
      "blurb": "Trim, mute, send, notes, clocks and modulation utilities."
    },
    {
      "id": 63,
      "name": "Formula Mod",
      "code": "UTL",
      "capability": "Safe math-expression modulation source",
      "coreName": "Utility",
      "blurb": "Trim, mute, send, notes, clocks and modulation utilities."
    },
    {
      "id": 64,
      "name": "Peak Follower",
      "code": "ANL",
      "capability": "Envelope follower used as a modulation source",
      "coreName": "Analysis",
      "blurb": "Scopes, spectra, meters, tuners and envelope followers."
    },
    {
      "id": 65,
      "name": "XY Pad",
      "code": "UTL",
      "capability": "Two-axis controller for modulation",
      "coreName": "Utility",
      "blurb": "Trim, mute, send, notes, clocks and modulation utilities."
    },
    {
      "id": 66,
      "name": "Stage FX",
      "code": "DST",
      "capability": "Performance FX: filter, distortion, gate and stutter",
      "coreName": "Distortion",
      "blurb": "Soft clipping, overdrive, bitcrushing and transfer curves."
    },
    {
      "id": 67,
      "name": "Visual Host",
      "code": "UTL",
      "capability": "Audio-visualizer host (passthrough audio)",
      "coreName": "Utility",
      "blurb": "Trim, mute, send, notes, clocks and modulation utilities."
    },
    {
      "id": 68,
      "name": "Tuner",
      "code": "ANL",
      "capability": "Instrument tuner using autocorrelation",
      "coreName": "Analysis",
      "blurb": "Scopes, spectra, meters, tuners and envelope followers."
    },
    {
      "id": 69,
      "name": "Balance Trim",
      "code": "UTL",
      "capability": "Gain/pan trim",
      "coreName": "Utility",
      "blurb": "Trim, mute, send, notes, clocks and modulation utilities."
    },
    {
      "id": 70,
      "name": "Soft Mute",
      "code": "UTL",
      "capability": "Click-free mute with ramp",
      "coreName": "Utility",
      "blurb": "Trim, mute, send, notes, clocks and modulation utilities."
    },
    {
      "id": 71,
      "name": "Send Tap",
      "code": "UTL",
      "capability": "Explicit send tap with level and mute",
      "coreName": "Utility",
      "blurb": "Trim, mute, send, notes, clocks and modulation utilities."
    },
    {
      "id": 72,
      "name": "Fast Lowpass",
      "code": "EQ",
      "capability": "One-pole low-pass with cutoff/resonance",
      "coreName": "Tone & filter",
      "blurb": "Biquad banks, shelves, graphic bands and morphing filters."
    },
    {
      "id": 73,
      "name": "Multi Filter",
      "code": "EQ",
      "capability": "Multimode filter with cutoff/resonance/mode",
      "coreName": "Tone & filter",
      "blurb": "Biquad banks, shelves, graphic bands and morphing filters."
    },
    {
      "id": 74,
      "name": "Morph Filter",
      "code": "EQ",
      "capability": "Morphing state-variable filter with LFO",
      "coreName": "Tone & filter",
      "blurb": "Biquad banks, shelves, graphic bands and morphing filters."
    }
  ]
};
