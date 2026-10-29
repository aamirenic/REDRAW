/* REDRAW showcase interactions.
   Browser demos are original sketches built with WebAudio and Canvas.
   They are not the Rust engine. All audio starts from a user gesture. */

(function () {
  'use strict';

  const DATA = window.REDRAW_DATA || {
    meta: { instrumentCount: 49, effectCount: 74 },
    families: {},
    cores: {},
    instruments: [],
    effects: []
  };
  const DATA_READY = !!(window.REDRAW_DATA && Array.isArray(window.REDRAW_DATA.instruments) && Array.isArray(window.REDRAW_DATA.effects));
  const PAGE = document.body ? document.body.dataset.page : '';
  const $ = (sel, root) => (root || document).querySelector(sel);
  const $$ = (sel, root) => Array.from((root || document).querySelectorAll(sel));
  const reduceMotion = !!(window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches);
  const clamp = (n, lo, hi) => Math.min(hi, Math.max(lo, n));
  const pad = (n) => String(n).padStart(2, '0');
  const midiToFreq = (m) => 440 * Math.pow(2, (m - 69) / 12);
  const dbToGain = (db) => Math.pow(10, db / 20);
  const fmtInt = (n) => new Intl.NumberFormat('en-US').format(n);
  const esc = (s) => String(s == null ? '' : s).replace(/[&<>"']/g, (c) => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'
  }[c]));

  const FAMILY_ORDER = ['S', 'W', 'F', 'A', 'P', 'D', 'G', 'X'];
  const FAMILY_COLORS = {
    S: '#d8451c', W: '#0b7d8c', F: '#9a6410', A: '#7b4b95',
    P: '#4f6a30', D: '#a32e3c', G: '#2a5f7e', X: '#665a4a'
  };
  const FAMILY_TRACE = {
    S: '#f08a5d', W: '#65c8f0', F: '#e4a33b', A: '#c49ae0',
    P: '#a9d17a', D: '#e46a7a', G: '#7fb3d5', X: '#b9a88e'
  };
  const CORE_ORDER = Object.keys(DATA.cores || {}).length
    ? Object.keys(DATA.cores)
    : ['EQ', 'DYN', 'DST', 'MOD', 'DLY', 'REV', 'PIT', 'V', 'ANL', 'ANL+UTL', 'UTL'];
  const CORE_COLORS = {
    EQ: '#d8451c', DYN: '#0b7d8c', DST: '#a32e3c', MOD: '#7b4b95',
    DLY: '#9a6410', REV: '#2a5f7e', PIT: '#4f6a30', V: '#8a3b6b',
    ANL: '#5b6b7c', 'ANL+UTL': '#665a4a', UTL: '#3f6b5a'
  };

  const familyColor = (code) => FAMILY_COLORS[code] || '#665a4a';
  const familyTrace = (code) => FAMILY_TRACE[code] || '#b9a88e';
  const coreColor = (code) => CORE_COLORS[code] || '#5b6b7c';
  const instrumentById = (id) => DATA.instruments.find((item) => item.id === Number(id));
  const effectById = (id) => DATA.effects.find((item) => item.id === Number(id));

  function mulberry32(a) {
    return function () {
      a |= 0;
      a = (a + 0x6D2B79F5) | 0;
      let t = Math.imul(a ^ (a >>> 15), 1 | a);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }

  function formatBytes(bytes) {
    if (!Number.isFinite(bytes)) return '—';
    if (bytes < 1024) return bytes + ' B';
    const units = ['KB', 'MB', 'GB'];
    let value = bytes / 1024;
    let i = 0;
    while (value >= 1024 && i < units.length - 1) {
      value /= 1024;
      i += 1;
    }
    return value.toFixed(value >= 10 ? 1 : 2) + ' ' + units[i];
  }

  function formatDuration(seconds) {
    if (!Number.isFinite(seconds)) return '—';
    return seconds.toFixed(2) + ' s';
  }

  /* ---------------- toast + small shared UI ---------------- */

  let toastEl = null;
  let toastTimer = 0;
  function toast(message) {
    if (!toastEl) {
      toastEl = document.createElement('div');
      toastEl.className = 'toast';
      toastEl.setAttribute('role', 'status');
      document.body.appendChild(toastEl);
    }
    toastEl.textContent = message;
    toastEl.classList.add('is-visible');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => toastEl.classList.remove('is-visible'), 3400);
    const sr = $('#sr-status');
    if (sr) sr.textContent = message;
  }

  function shake(el) {
    if (!el) return;
    el.classList.remove('shake');
    void el.offsetWidth;
    el.classList.add('shake');
    setTimeout(() => el.classList.remove('shake'), 420);
  }

  function initChrome() {
    const progress = $('.scroll-progress');
    if (progress) {
      const update = () => {
        const max = document.documentElement.scrollHeight - window.innerHeight;
        const ratio = max > 0 ? clamp(window.scrollY / max, 0, 1) : 0;
        progress.style.transform = 'scaleX(' + ratio + ')';
      };
      window.addEventListener('scroll', update, { passive: true });
      window.addEventListener('resize', update);
      update();
    }

    const menu = $('#site-menu');
    const openBtn = $('#menu-open');
    const closeBtn = $('#menu-close');
    if (menu && openBtn) {
      const isMenuOpen = () =>
        typeof menu.showModal === 'function' ? menu.open : menu.hasAttribute('open');
      const syncMenuState = () => {
        const open = isMenuOpen();
        document.documentElement.classList.toggle('menu-open', open);
        openBtn.setAttribute('aria-expanded', open ? 'true' : 'false');
      };
      const openMenu = () => {
        if (isMenuOpen()) {
          syncMenuState();
          return;
        }
        if (typeof menu.showModal === 'function') menu.showModal();
        else menu.setAttribute('open', '');
        syncMenuState();
      };
      const closeMenu = () => {
        if (isMenuOpen()) {
          if (typeof menu.close === 'function') menu.close();
          else menu.removeAttribute('open');
        }
        syncMenuState();
      };
      openBtn.addEventListener('click', openMenu);
      if (closeBtn) closeBtn.addEventListener('click', closeMenu);
      menu.addEventListener('click', (event) => {
        if (event.target === menu) closeMenu();
      });
      $$('a', menu).forEach((link) => link.addEventListener('click', closeMenu));
      menu.addEventListener('keydown', (event) => {
        if (event.key !== 'Tab') return;
        const focusables = $$('a[href], button:not([disabled])', menu).filter((el) => el.getClientRects().length);
        if (!focusables.length) return;
        const first = focusables[0];
        const last = focusables[focusables.length - 1];
        if (event.shiftKey && document.activeElement === first) {
          event.preventDefault();
          last.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first.focus();
        }
      });
      menu.addEventListener('close', () => {
        // A close event can be queued by the previous dialog and arrive after a
        // rapid reopen. Only clear the visual state if the menu is really shut.
        if (!isMenuOpen()) syncMenuState();
      });
      const desktopQuery = window.matchMedia('(min-width: 901px)');
      const closeOnDesktop = (event) => {
        if (event.matches && menu.open) closeMenu();
      };
      if (typeof desktopQuery.addEventListener === 'function') desktopQuery.addEventListener('change', closeOnDesktop);
      else if (typeof desktopQuery.addListener === 'function') desktopQuery.addListener(closeOnDesktop);
    }

    const tablist = $('[role="tablist"]');
    if (tablist) {
      const tabs = $$('[role="tab"]', tablist);
      const select = (tab, focus) => {
        tabs.forEach((item) => {
          const on = item === tab;
          item.setAttribute('aria-selected', on ? 'true' : 'false');
          item.tabIndex = on ? 0 : -1;
          const panel = document.getElementById(item.getAttribute('aria-controls'));
          if (panel) panel.hidden = !on;
        });
        if (focus) tab.focus();
      };
      tabs.forEach((tab, index) => {
        tab.addEventListener('click', () => select(tab, false));
        tab.addEventListener('keydown', (event) => {
          let next = null;
          if (event.key === 'ArrowDown' || event.key === 'ArrowRight') next = (index + 1) % tabs.length;
          else if (event.key === 'ArrowUp' || event.key === 'ArrowLeft') next = (index - 1 + tabs.length) % tabs.length;
          else if (event.key === 'Home') next = 0;
          else if (event.key === 'End') next = tabs.length - 1;
          if (next === null) return;
          event.preventDefault();
          select(tabs[next], true);
        });
      });
      const selected = tabs.find((tab) => tab.getAttribute('aria-selected') === 'true') || tabs[0];
      if (selected) select(selected, false);
    }
  }

  function initReveal() {
    const items = $$('[data-reveal]');
    if (!('IntersectionObserver' in window) || reduceMotion) {
      items.forEach((item) => item.classList.add('is-visible'));
    } else {
      const io = new IntersectionObserver((entries) => {
        entries.forEach((entry) => {
          if (!entry.isIntersecting) return;
          entry.target.classList.add('is-visible');
          io.unobserve(entry.target);
        });
      }, { rootMargin: '0px 0px -8% 0px', threshold: 0.08 });
      items.forEach((item) => io.observe(item));
    }

    const counters = $$('[data-count]');
    if (reduceMotion) return;
    const animate = (el) => {
      if (el.dataset.counted) return;
      el.dataset.counted = '1';
      const target = Number(el.dataset.count);
      if (!Number.isFinite(target) || target <= 0) {
        el.textContent = String(target || 0);
        return;
      }
      const start = performance.now();
      const duration = 900;
      const frame = (now) => {
        const p = clamp((now - start) / duration, 0, 1);
        const eased = 1 - Math.pow(1 - p, 3);
        el.textContent = String(Math.round(target * eased));
        if (p < 1) requestAnimationFrame(frame);
        else el.textContent = String(target);
      };
      requestAnimationFrame(frame);
    };
    if (!('IntersectionObserver' in window)) {
      counters.forEach(animate);
    } else {
      const io = new IntersectionObserver((entries) => {
        entries.forEach((entry) => {
          if (!entry.isIntersecting) return;
          animate(entry.target);
          io.unobserve(entry.target);
        });
      }, { threshold: 0.35 });
      counters.forEach((counter) => io.observe(counter));
    }
  }

  /* ---------------- audio foundation ---------------- */

  const audio = {
    ctx: null,
    ensure() {
      const Ctor = window.AudioContext || window.webkitAudioContext;
      if (!Ctor) return null;
      if (!this.ctx) this.ctx = new Ctor({ latencyHint: 'interactive' });
      if (this.ctx.state === 'suspended') this.ctx.resume();
      return this.ctx;
    }
  };

  const noiseCache = new WeakMap();
  const impulseCache = new WeakMap();

  function getNoise(ctx) {
    if (!noiseCache.has(ctx)) {
      const buffer = ctx.createBuffer(1, Math.ceil(ctx.sampleRate), ctx.sampleRate);
      const data = buffer.getChannelData(0);
      for (let i = 0; i < data.length; i += 1) data[i] = Math.random() * 2 - 1;
      noiseCache.set(ctx, buffer);
    }
    return noiseCache.get(ctx);
  }

  function makeImpulse(ctx, seconds, decay) {
    let map = impulseCache.get(ctx);
    if (!map) {
      map = new Map();
      impulseCache.set(ctx, map);
    }
    const key = seconds + ':' + decay;
    if (map.has(key)) return map.get(key);
    const length = Math.max(1, Math.floor(ctx.sampleRate * seconds));
    const buffer = ctx.createBuffer(2, length, ctx.sampleRate);
    for (let ch = 0; ch < 2; ch += 1) {
      const data = buffer.getChannelData(ch);
      for (let i = 0; i < length; i += 1) {
        data[i] = (Math.random() * 2 - 1) * Math.pow(1 - i / length, decay);
      }
    }
    map.set(key, buffer);
    return buffer;
  }

  function panOut(ctx, dest, pan) {
    const input = ctx.createGain();
    if (ctx.createStereoPanner) {
      const panner = ctx.createStereoPanner();
      panner.pan.value = clamp(pan || 0, -1, 1);
      input.connect(panner);
      panner.connect(dest);
    } else {
      input.connect(dest);
    }
    return input;
  }

  function envelope(ctx, time, peak, attack, release) {
    const gain = ctx.createGain();
    gain.gain.setValueAtTime(0.0001, time);
    gain.gain.exponentialRampToValueAtTime(Math.max(0.0002, peak), time + attack);
    gain.gain.exponentialRampToValueAtTime(0.0001, time + attack + release);
    return gain;
  }

  function makeDriveCurve(amount) {
    const samples = 2048;
    const curve = new Float32Array(samples);
    for (let i = 0; i < samples; i += 1) {
      const x = (i * 2) / (samples - 1) - 1;
      curve[i] = Math.tanh(x * amount);
    }
    return curve;
  }

  function encodeWav(channels, sampleRate, format) {
    const bits = format === 'float32' ? 32 : format === '24' ? 24 : 16;
    const bytes = bits / 8;
    const channelCount = channels.length;
    const frames = channels[0] ? channels[0].length : 0;
    const blockAlign = channelCount * bytes;
    const dataSize = frames * blockAlign;
    const buffer = new ArrayBuffer(44 + dataSize);
    const view = new DataView(buffer);
    const writeString = (offset, value) => {
      for (let i = 0; i < value.length; i += 1) view.setUint8(offset + i, value.charCodeAt(i));
    };
    writeString(0, 'RIFF');
    view.setUint32(4, 36 + dataSize, true);
    writeString(8, 'WAVE');
    writeString(12, 'fmt ');
    view.setUint32(16, 16, true);
    view.setUint16(20, format === 'float32' ? 3 : 1, true);
    view.setUint16(22, channelCount, true);
    view.setUint32(24, sampleRate, true);
    view.setUint32(28, sampleRate * blockAlign, true);
    view.setUint16(32, blockAlign, true);
    view.setUint16(34, bits, true);
    writeString(36, 'data');
    view.setUint32(40, dataSize, true);

    let offset = 44;
    for (let frame = 0; frame < frames; frame += 1) {
      for (let ch = 0; ch < channelCount; ch += 1) {
        const sample = clamp(channels[ch][frame] || 0, -1, 1);
        if (bits === 16) {
          view.setInt16(offset, Math.round(sample < 0 ? sample * 0x8000 : sample * 0x7FFF), true);
          offset += 2;
        } else if (bits === 24) {
          let value = Math.round(sample < 0 ? sample * 0x800000 : sample * 0x7FFFFF);
          if (value < 0) value += 0x1000000;
          view.setUint8(offset, value & 0xFF);
          view.setUint8(offset + 1, (value >> 8) & 0xFF);
          view.setUint8(offset + 2, (value >> 16) & 0xFF);
          offset += 3;
        } else {
          view.setFloat32(offset, sample, true);
          offset += 4;
        }
      }
    }
    return buffer;
  }

  function measurePeak(channels) {
    let peak = 0;
    channels.forEach((data) => {
      for (let i = 0; i < data.length; i += 1) {
        const v = Math.abs(data[i]);
        if (v > peak) peak = v;
      }
    });
    return peak;
  }

  function scaleChannels(channels, gain) {
    channels.forEach((data) => {
      for (let i = 0; i < data.length; i += 1) data[i] *= gain;
    });
  }

  function fitCanvas(canvas) {
    if (!canvas) return null;
    const rect = canvas.getBoundingClientRect();
    const width = Math.max(1, Math.round(rect.width));
    const height = Math.max(1, Math.round(rect.height));
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const pixelWidth = Math.max(1, Math.round(width * dpr));
    const pixelHeight = Math.max(1, Math.round(height * dpr));
    if (canvas.width !== pixelWidth || canvas.height !== pixelHeight) {
      canvas.width = pixelWidth;
      canvas.height = pixelHeight;
    }
    const ctx = canvas.getContext('2d');
    if (!ctx) return null;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    return { ctx, width, height, dpr };
  }

  function drawGrid(ctx, width, height) {
    ctx.strokeStyle = 'rgba(101, 200, 240, 0.09)';
    ctx.lineWidth = 1;
    ctx.beginPath();
    for (let x = 0.5; x < width; x += 36) {
      ctx.moveTo(x, 0);
      ctx.lineTo(x, height);
    }
    for (let y = 0.5; y < height; y += 36) {
      ctx.moveTo(0, y);
      ctx.lineTo(width, y);
    }
    ctx.stroke();
    ctx.strokeStyle = 'rgba(101, 200, 240, 0.18)';
    ctx.beginPath();
    ctx.moveTo(0, height / 2 + 0.5);
    ctx.lineTo(width, height / 2 + 0.5);
    ctx.stroke();
  }

  function drawScope(canvas, analyser, active, timestamp) {
    const fit = fitCanvas(canvas);
    if (!fit) return;
    const { ctx, width, height } = fit;
    ctx.clearRect(0, 0, width, height);
    ctx.strokeStyle = 'rgba(101, 200, 240, 0.08)';
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(0, height / 2);
    ctx.lineTo(width, height / 2);
    ctx.stroke();

    let data = null;
    if (active && analyser) {
      data = new Float32Array(analyser.fftSize);
      analyser.getFloatTimeDomainData(data);
    }
    const points = 240;
    ctx.beginPath();
    for (let i = 0; i < points; i += 1) {
      const p = i / (points - 1);
      const x = p * width;
      let value;
      if (data) {
        const index = Math.floor(p * (data.length - 1));
        value = data[index];
      } else {
        value = 0.11 * Math.sin(p * Math.PI * 6 + timestamp * 0.001) +
          0.045 * Math.sin(p * Math.PI * 17 - timestamp * 0.0013);
      }
      const y = height / 2 - value * height * 0.42;
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.strokeStyle = active ? 'rgba(101, 200, 240, 0.94)' : 'rgba(101, 200, 240, 0.26)';
    ctx.lineWidth = active ? 1.6 : 1;
    ctx.shadowColor = active ? 'rgba(101, 200, 240, 0.65)' : 'transparent';
    ctx.shadowBlur = active ? 8 : 0;
    ctx.stroke();
    ctx.shadowBlur = 0;
  }

  function makeScope(canvas, getAnalyser, isActive) {
    if (!canvas) return { refresh() {} };
    let raf = 0;
    const draw = (timestamp) => drawScope(canvas, getAnalyser(), isActive(), timestamp);
    if (reduceMotion) {
      draw(0);
      return {
        refresh() { draw(performance.now()); }
      };
    }
    const loop = (timestamp) => {
      draw(timestamp);
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return {
      refresh() {},
      stop() { cancelAnimationFrame(raf); }
    };
  }

  function wavePath(seed, code, width, height) {
    const w = width || 200;
    const h = height || 60;
    const random = mulberry32((seed + 1) * 2654435761 + code.charCodeAt(0) * 97);
    const count = 5 + Math.floor(random() * 3);
    const harmonics = [];
    for (let i = 0; i < count; i += 1) {
      harmonics.push({
        freq: 1 + i * (code === 'F' ? 1.65 : 1.08) + random() * 0.4,
        amp: (0.55 + random() * 0.45) / (i + 1),
        phase: random() * Math.PI * 2
      });
    }
    const raw = [];
    let max = 0.0001;
    const points = 160;
    for (let i = 0; i <= points; i += 1) {
      const p = i / points;
      let value = 0;
      harmonics.forEach((harmonic) => {
        value += Math.sin(p * Math.PI * 2 * harmonic.freq + harmonic.phase) * harmonic.amp;
      });
      let env = 1;
      if (code === 'P') env = Math.exp(-p * 2.8);
      if (code === 'D') env = Math.exp(-p * 5.5);
      if (code === 'G') env = 0.32 + 0.68 * Math.abs(Math.sin(p * Math.PI * 7));
      value *= env;
      raw.push(value);
      max = Math.max(max, Math.abs(value));
    }
    const scale = (h * 0.37) / max;
    const d = [];
    raw.forEach((value, i) => {
      const x = (i / points) * w;
      const y = h / 2 - value * scale;
      d.push((i === 0 ? 'M' : 'L') + x.toFixed(2) + ' ' + y.toFixed(2));
    });
    return d.join(' ');
  }

  /* ---------------- simple synth voices ---------------- */

  function heroKick(ctx, dest, time, velocity, options) {
    const level = velocity == null ? 1 : velocity;
    const opts = options || {};
    const startHz = opts.start || 155;
    const endHz = opts.end || 43;
    const decay = opts.decay || 0.42;
    const clickLevel = opts.click == null ? 0.18 : opts.click;
    const out = panOut(ctx, dest, 0);
    const osc = ctx.createOscillator();
    osc.type = 'sine';
    osc.frequency.setValueAtTime(startHz, time);
    osc.frequency.exponentialRampToValueAtTime(endHz, time + 0.16);
    const gain = ctx.createGain();
    gain.gain.setValueAtTime(0.0001, time);
    gain.gain.exponentialRampToValueAtTime(0.92 * level, time + 0.005);
    gain.gain.exponentialRampToValueAtTime(0.0001, time + decay);
    osc.connect(gain);
    gain.connect(out);
    osc.start(time);
    osc.stop(time + decay + 0.03);

    const click = ctx.createBufferSource();
    click.buffer = getNoise(ctx);
    const high = ctx.createBiquadFilter();
    high.type = 'highpass';
    high.frequency.value = 1400;
    const clickGain = ctx.createGain();
    clickGain.gain.setValueAtTime(clickLevel * level, time);
    clickGain.gain.exponentialRampToValueAtTime(0.0001, time + 0.025);
    click.connect(high);
    high.connect(clickGain);
    clickGain.connect(out);
    click.start(time);
    click.stop(time + 0.03);
  }

  function heroBass(ctx, dest, time, freq, velocity, duration) {
    const level = velocity == null ? 1 : velocity;
    const dur = duration || 0.22;
    const out = panOut(ctx, dest, 0);
    const osc = ctx.createOscillator();
    osc.type = 'sawtooth';
    osc.frequency.setValueAtTime(freq, time);
    const filter = ctx.createBiquadFilter();
    filter.type = 'lowpass';
    filter.Q.value = 7;
    filter.frequency.setValueAtTime(920, time);
    filter.frequency.exponentialRampToValueAtTime(190, time + dur);
    const gain = ctx.createGain();
    gain.gain.setValueAtTime(0.0001, time);
    gain.gain.exponentialRampToValueAtTime(0.3 * level, time + 0.009);
    gain.gain.exponentialRampToValueAtTime(0.0001, time + dur);
    osc.connect(filter);
    filter.connect(gain);
    gain.connect(out);
    osc.start(time);
    osc.stop(time + dur + 0.02);
  }

  function heroKeys(ctx, dest, time, freqs, velocity, duration) {
    const level = velocity == null ? 1 : velocity;
    const dur = duration || 0.62;
    const out = panOut(ctx, dest, 0.12);
    freqs.forEach((freq, index) => {
      const osc = ctx.createOscillator();
      osc.type = index % 2 ? 'triangle' : 'sine';
      osc.frequency.value = freq;
      osc.detune.value = (index - 1) * 4;
      const gain = ctx.createGain();
      gain.gain.setValueAtTime(0.0001, time);
      gain.gain.exponentialRampToValueAtTime(0.075 * level, time + 0.014);
      gain.gain.exponentialRampToValueAtTime(0.0001, time + dur);
      osc.connect(gain);
      gain.connect(out);
      osc.start(time);
      osc.stop(time + dur + 0.02);
    });
  }

  function heroHat(ctx, dest, time, velocity, open) {
    const level = velocity == null ? 1 : velocity;
    const duration = open ? 0.2 : 0.045;
    const source = ctx.createBufferSource();
    source.buffer = getNoise(ctx);
    const high = ctx.createBiquadFilter();
    high.type = 'highpass';
    high.frequency.value = 7200;
    const band = ctx.createBiquadFilter();
    band.type = 'bandpass';
    band.frequency.value = 10200;
    band.Q.value = 0.7;
    const gain = ctx.createGain();
    gain.gain.setValueAtTime(0.15 * level, time);
    gain.gain.exponentialRampToValueAtTime(0.0001, time + duration);
    source.connect(high);
    high.connect(band);
    band.connect(gain);
    gain.connect(dest);
    source.start(time);
    source.stop(time + duration + 0.01);
  }

  /* ---------------- home / live console ---------------- */

  function initHome() {
    const seq = $('#sequencer');
    if (!seq) return;
    const playBtn = $('#seq-play');
    const bpmInput = $('#bpm');
    const bpmValue = $('#bpm-value');
    const readout = $('#step-readout');
    const status = $('#seq-status');
    const scopeState = $('#scope-state');
    const renderBtn = $('#seq-render');
    const download = $('#seq-download');
    const randomBtn = $('#seq-random');
    const clearBtn = $('#seq-clear');
    const barsSelect = $('#seq-bars');

    const trackDefs = [
      {
        name: 'Kick', color: '#f09a4b', kind: 'kick',
        steps: [1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0],
        vels: [1, 0, 0, 0, 0.86, 0, 0, 0, 0.94, 0, 0, 0, 0.82, 0, 0, 0]
      },
      {
        name: 'Bass', color: '#65c8f0', kind: 'bass',
        steps: [1, 0, 0, 1, 0, 0, 1, 0, 1, 0, 1, 0, 0, 0, 1, 0],
        vels: [0.9, 0, 0, 0.62, 0, 0, 0.7, 0, 0.86, 0, 0.6, 0, 0, 0, 0.72, 0]
      },
      {
        name: 'Keys', color: '#d9e4f2', kind: 'keys',
        steps: [1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0],
        vels: [0.82, 0, 0, 0, 0, 0, 0.66, 0, 0, 0, 0.74, 0, 0, 0, 0.6, 0]
      },
      {
        name: 'Hats', color: '#72d284', kind: 'hat',
        steps: [1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0],
        vels: [0.72, 0, 0.5, 0, 0.6, 0, 0.44, 0, 0.68, 0, 0.5, 0, 0.58, 0, 0.78, 0]
      }
    ];
    const tracks = trackDefs.map((track) => ({
      name: track.name,
      color: track.color,
      kind: track.kind,
      steps: track.steps.slice(),
      vels: track.vels.slice(),
      mute: false
    }));

    const seqState = {
      playing: false,
      timer: 0,
      raf: 0,
      nextTime: 0,
      step: 0,
      queue: [],
      lastVisual: -1,
      bpm: 112,
      bars: 2,
      rendering: false
    };
    let heroBus = null;
    let heroAnalyser = null;
    let heroUrl = null;

    makeScope($('#hero-scope'), () => heroAnalyser, () => seqState.playing);

    function setStatus(text) {
      if (status) status.textContent = text;
    }

    function setScopeState(text) {
      if (scopeState) scopeState.textContent = text;
    }

    function stepLabel(track, index) {
      const on = track.steps[index] ? 'on' : 'off';
      return track.name + ' step ' + pad(index + 1) + ', ' + on + ', velocity ' + Math.round(track.vels[index] * 100) + ' percent';
    }

    function updateStep(button, track, index) {
      button.setAttribute('aria-pressed', track.steps[index] ? 'true' : 'false');
      button.style.setProperty('--vel', track.vels[index].toFixed(2));
      button.setAttribute('aria-label', stepLabel(track, index));
    }

    function renderSequencer() {
      const numbers = '<div class="step-numbers"><span aria-hidden="true"></span><div class="num-grid">' +
        Array.from({ length: 16 }, (_, i) =>
          '<span class="step-number' + (i % 4 === 0 ? ' beat' : '') + '">' + pad(i + 1) + '</span>'
        ).join('') + '</div></div>';
      const rows = tracks.map((track, ti) =>
        '<div class="track" style="--track:' + track.color + '" data-track="' + ti + '">' +
          '<div class="track-head">' +
            '<span class="track-dot" aria-hidden="true"></span>' +
            '<span class="track-name">' + esc(track.name) + '</span>' +
            '<button class="mute-btn" type="button" data-mute="' + ti + '" aria-pressed="' + (track.mute ? 'true' : 'false') + '" aria-label="Mute ' + esc(track.name) + '">M</button>' +
          '</div>' +
          '<div class="step-grid">' +
            track.steps.map((on, si) =>
              '<button class="step' + (si % 4 === 0 ? ' beat' : '') + '" type="button" data-track="' + ti + '" data-step="' + si + '" tabindex="' + (ti === 0 && si === 0 ? '0' : '-1') + '" aria-pressed="' + (on ? 'true' : 'false') + '" aria-label="' + esc(stepLabel(track, si)) + '" style="--vel:' + track.vels[si].toFixed(2) + '"></button>'
            ).join('') +
          '</div>' +
        '</div>'
      ).join('');
      seq.innerHTML = numbers + rows;
      if (seqState.playing && seqState.lastVisual >= 0) setVisual(seqState.lastVisual);
    }

    function setRovingStep(active) {
      $$('.step', seq).forEach((el) => { el.tabIndex = el === active ? 0 : -1; });
    }

    function setVisual(step) {
      seqState.lastVisual = step;
      $$('.step.is-current', seq).forEach((el) => el.classList.remove('is-current'));
      if (step >= 0) {
        $$('.step[data-step="' + step + '"]', seq).forEach((el) => el.classList.add('is-current'));
      }
      if (readout) readout.textContent = 'Step ' + pad((step >= 0 ? step : 0) + 1) + ' / 16';
    }

    function ensureBus(ctx) {
      if (heroBus) return;
      heroBus = ctx.createGain();
      heroBus.gain.value = 0.78;
      const compressor = ctx.createDynamicsCompressor();
      compressor.threshold.value = -14;
      compressor.knee.value = 8;
      compressor.ratio.value = 3;
      compressor.attack.value = 0.004;
      compressor.release.value = 0.18;
      heroAnalyser = ctx.createAnalyser();
      heroAnalyser.fftSize = 2048;
      heroAnalyser.smoothingTimeConstant = 0.7;
      heroBus.connect(compressor);
      compressor.connect(heroAnalyser);
      heroAnalyser.connect(ctx.destination);
    }

    function scheduleStep(ctx, dest, step, time) {
      const bassPattern = [45, 45, 48, 45, 52, 45, 50, 48];
      const chords = [
        [57, 60, 64], [53, 57, 60], [55, 59, 62], [52, 55, 59]
      ];
      tracks.forEach((track) => {
        if (track.mute || !track.steps[step]) return;
        const velocity = track.vels[step] || 0.8;
        if (track.kind === 'kick') {
          heroKick(ctx, dest, time, velocity);
        } else if (track.kind === 'bass') {
          const midi = bassPattern[Math.floor(step / 2) % bassPattern.length];
          heroBass(ctx, dest, time, midiToFreq(midi), velocity, 0.21);
        } else if (track.kind === 'keys') {
          const chord = chords[Math.floor(step / 4) % chords.length];
          heroKeys(ctx, dest, time, chord.map(midiToFreq), velocity, 0.68);
        } else if (track.kind === 'hat') {
          heroHat(ctx, dest, time, velocity, step === 14);
        }
      });
    }

    function scheduler() {
      const ctx = audio.ctx;
      if (!ctx || !seqState.playing) return;
      const secondsPerStep = 60 / seqState.bpm / 4;
      if (seqState.nextTime < ctx.currentTime - 0.4) seqState.nextTime = ctx.currentTime + 0.05;
      while (seqState.nextTime < ctx.currentTime + 0.14) {
        scheduleStep(ctx, heroBus, seqState.step, seqState.nextTime);
        seqState.queue.push({ step: seqState.step, time: seqState.nextTime });
        seqState.nextTime += secondsPerStep;
        seqState.step = (seqState.step + 1) % 16;
      }
    }

    function tick() {
      if (!seqState.playing || !audio.ctx) return;
      const now = audio.ctx.currentTime;
      while (seqState.queue.length && seqState.queue[0].time <= now + 0.012) {
        const item = seqState.queue.shift();
        setVisual(item.step);
      }
      seqState.raf = requestAnimationFrame(tick);
    }

    function startSequencer() {
      const ctx = audio.ensure();
      if (!ctx) {
        toast('Web Audio is not available in this browser.');
        return;
      }
      if (seqState.playing) return;
      ensureBus(ctx);
      seqState.playing = true;
      seqState.step = 0;
      seqState.queue = [];
      seqState.nextTime = ctx.currentTime + 0.07;
      setVisual(-1);
      scheduler();
      seqState.timer = setInterval(scheduler, 25);
      seqState.raf = requestAnimationFrame(tick);
      if (playBtn) {
        playBtn.setAttribute('aria-pressed', 'true');
        playBtn.setAttribute('aria-label', 'Stop pattern');
      }
      setStatus('Playing · 16-step pattern');
      setScopeState('Live');
    }

    function stopSequencer() {
      seqState.playing = false;
      clearInterval(seqState.timer);
      cancelAnimationFrame(seqState.raf);
      seqState.queue = [];
      setVisual(-1);
      if (playBtn) {
        playBtn.setAttribute('aria-pressed', 'false');
        playBtn.setAttribute('aria-label', 'Play pattern');
      }
      setStatus('Stopped · click steps to redraw the pattern');
      setScopeState('Idle');
    }

    function toggleSequencer() {
      if (seqState.playing) stopSequencer();
      else startSequencer();
    }

    function cycleVelocity(track, index) {
      const values = [0.45, 0.65, 0.85, 1];
      let current = values.indexOf(track.vels[index]);
      if (current < 0) current = 2;
      track.vels[index] = values[(current + 1) % values.length];
      track.steps[index] = 1;
    }

    function applyPatternMutation() {
      renderSequencer();
      if (seqState.playing) setVisual(seqState.lastVisual);
    }

    async function renderWav() {
      if (seqState.rendering) return;
      const Ctor = window.OfflineAudioContext || window.webkitOfflineAudioContext;
      if (!Ctor) {
        toast('Offline audio rendering is not available in this browser.');
        return;
      }
      if (!renderBtn) return;
      const audible = tracks.some((track) => !track.mute && track.steps.some(Boolean));
      if (!audible) {
        const hasSteps = tracks.some((track) => track.steps.some(Boolean));
        const message = hasSteps
          ? 'All tracks are muted. Unmute a track to render.'
          : 'Pattern is empty. Turn on a step to render.';
        if (download) download.classList.remove('is-visible');
        setStatus(message);
        toast(message);
        return;
      }
      const bpm = seqState.bpm;
      const bars = seqState.bars;
      if (download) download.classList.remove('is-visible');
      seqState.rendering = true;
      seq.setAttribute('aria-busy', 'true');
      if (bpmInput) bpmInput.disabled = true;
      if (barsSelect) barsSelect.disabled = true;
      if (randomBtn) randomBtn.disabled = true;
      if (clearBtn) clearBtn.disabled = true;
      renderBtn.disabled = true;
      renderBtn.textContent = 'Rendering…';
      setStatus('Rendering ' + bars + ' bars at ' + bpm + ' BPM…');
      try {
        const sampleRate = 48000;
        const secondsPerStep = 60 / bpm / 4;
        const steps = bars * 16;
        const duration = steps * secondsPerStep + 1.15;
        const offline = new Ctor(2, Math.ceil(duration * sampleRate), sampleRate);
        const master = offline.createGain();
        master.gain.value = 0.76;
        const compressor = offline.createDynamicsCompressor();
        compressor.threshold.value = -14;
        compressor.knee.value = 8;
        compressor.ratio.value = 3;
        compressor.attack.value = 0.004;
        compressor.release.value = 0.18;
        master.connect(compressor);
        compressor.connect(offline.destination);
        for (let i = 0; i < steps; i += 1) {
          scheduleStep(offline, master, i % 16, 0.04 + i * secondsPerStep);
        }
        const rendered = await offline.startRendering();
        const channels = [rendered.getChannelData(0), rendered.getChannelData(1)];
        let peak = measurePeak(channels);
        if (peak > 0.985) {
          scaleChannels(channels, 0.985 / peak);
          peak = 0.985;
        }
        const wav = encodeWav(channels, sampleRate, '16');
        const blob = new Blob([wav], { type: 'audio/wav' });
        if (heroUrl) URL.revokeObjectURL(heroUrl);
        heroUrl = URL.createObjectURL(blob);
        if (download) {
          download.href = heroUrl;
          download.download = 'redraw-pattern-' + bpm + 'bpm-' + bars + 'bars.wav';
          download.classList.add('is-visible');
        }
        const peakDb = peak > 0 ? (20 * Math.log10(peak)).toFixed(1) : '-inf';
        setStatus('WAV ready · ' + bars + ' bars · ' + bpm + ' BPM · 48 kHz / 16-bit · ' + formatBytes(wav.byteLength));
        toast('Rendered ' + fmtInt(channels[0].length) + ' frames · peak ' + peakDb + ' dBFS');
      } catch (error) {
        console.error(error);
        setStatus('Render failed. Try again.');
        toast('Render failed. Try again.');
      } finally {
        seqState.rendering = false;
        seq.removeAttribute('aria-busy');
        if (bpmInput) bpmInput.disabled = false;
        if (barsSelect) barsSelect.disabled = false;
        if (randomBtn) randomBtn.disabled = false;
        if (clearBtn) clearBtn.disabled = false;
        renderBtn.disabled = false;
        renderBtn.textContent = 'Render WAV';
      }
    }

    function randomize() {
      if (seqState.rendering) return;
      const random = mulberry32((Date.now() & 0xFFFFFFFF) >>> 0);
      const chance = (kind, index) => {
        if (kind === 'kick') return index % 4 === 0 ? 0.96 : 0.12;
        if (kind === 'bass') return index % 4 === 0 ? 0.78 : index % 2 === 0 ? 0.42 : 0.16;
        if (kind === 'keys') return index % 8 === 0 ? 0.78 : index % 4 === 0 ? 0.45 : 0.1;
        return index % 2 === 1 ? 0.82 : 0.2;
      };
      tracks.forEach((track) => {
        for (let i = 0; i < 16; i += 1) {
          track.steps[i] = random() < chance(track.kind, i) ? 1 : 0;
          track.vels[i] = track.steps[i] ? clamp(0.5 + random() * 0.5 + (i % 4 === 0 ? 0.08 : 0), 0.4, 1) : 0.65;
        }
      });
      tracks[0].steps[0] = 1;
      tracks[0].vels[0] = 1;
      applyPatternMutation();
      setStatus('Pattern randomized · shift-click a step for velocity');
    }

    function clearPattern() {
      if (seqState.rendering) return;
      tracks.forEach((track) => {
        track.steps = track.steps.map(() => 0);
        track.vels = track.vels.map(() => 0.65);
      });
      applyPatternMutation();
      setStatus('Pattern cleared · click steps to redraw');
    }

    renderSequencer();

    /* flagship card scopes + browser auditions */
    $$('.card-scope').forEach((scope) => {
      const marker = scope.querySelector('[data-wave]');
      const code = marker ? marker.dataset.wave : 'S';
      const seed = marker ? Number(marker.dataset.seed) || 1 : 1;
      const pathData = wavePath(seed, code, 200, 60);
      $$('.scope-path', scope).forEach((path) => path.setAttribute('d', pathData));
    });

    let cardAudition = { btn: null, timer: 0, master: null };
    function stopCardAudition() {
      clearTimeout(cardAudition.timer);
      cardAudition.timer = 0;
      if (cardAudition.master) {
        const master = cardAudition.master;
        cardAudition.master = null;
        try {
          const now = master.context.currentTime;
          master.gain.cancelScheduledValues(now);
          master.gain.setTargetAtTime(0.0001, now, 0.01);
          setTimeout(() => {
            try { master.disconnect(); } catch (error) { /* already disconnected */ }
          }, 80);
        } catch (error) {
          try { master.disconnect(); } catch (disconnectError) { /* already disconnected */ }
        }
      }
      if (cardAudition.btn) {
        cardAudition.btn.classList.remove('is-playing');
        cardAudition.btn.setAttribute('aria-pressed', 'false');
      }
      cardAudition.btn = null;
    }

    function playCardAudition(button) {
      if (cardAudition.btn === button) {
        stopCardAudition();
        return;
      }
      stopCardAudition();
      const ctx = audio.ensure();
      if (!ctx) {
        toast('Web Audio is not available in this browser.');
        return;
      }
      const code = button.dataset.auditionFamily || 'S';
      const name = button.dataset.auditionName || 'Engine';
      const id = Number(button.dataset.auditionId) || 0;
      const master = ctx.createGain();
      master.gain.value = 0.7;
      const compressor = ctx.createDynamicsCompressor();
      compressor.threshold.value = -15;
      compressor.knee.value = 8;
      compressor.ratio.value = 3;
      compressor.attack.value = 0.004;
      compressor.release.value = 0.2;
      master.connect(compressor);
      compressor.connect(ctx.destination);
      cardAudition.master = master;
      scheduleFamilyAudition(ctx, master, { code, name, id }, ctx.currentTime + 0.05);
      button.classList.add('is-playing');
      button.setAttribute('aria-pressed', 'true');
      cardAudition.btn = button;
      cardAudition.timer = setTimeout(stopCardAudition, 2200);
      toast('Auditioning ' + name + ' · per-engine browser sketch');
    }

    const rack = $('#rack');
    if (rack) {
      $$('[data-audition-family]', rack).forEach((button) => button.setAttribute('aria-pressed', 'false'));
      rack.addEventListener('click', (event) => {
        const button = event.target.closest('[data-audition-family]');
        if (button) playCardAudition(button);
      });
    }

    if (bpmInput) {
      seqState.bpm = Number(bpmInput.value) || 112;
      if (bpmValue) bpmValue.textContent = String(seqState.bpm);
      bpmInput.addEventListener('input', () => {
        seqState.bpm = Number(bpmInput.value) || 112;
        if (bpmValue) bpmValue.textContent = String(seqState.bpm);
      });
    }
    if (barsSelect) {
      seqState.bars = Number(barsSelect.value) || 2;
      barsSelect.addEventListener('change', () => {
        seqState.bars = Number(barsSelect.value) || 2;
      });
    }
    if (playBtn) playBtn.addEventListener('click', toggleSequencer);
    if (renderBtn) renderBtn.addEventListener('click', renderWav);
    if (randomBtn) randomBtn.addEventListener('click', randomize);
    if (clearBtn) clearBtn.addEventListener('click', clearPattern);

    seq.addEventListener('click', (event) => {
      const mute = event.target.closest('[data-mute]');
      if (seqState.rendering && (mute || event.target.closest('.step'))) return;
      if (mute) {
        const index = Number(mute.dataset.mute);
        const track = tracks[index];
        if (!track) return;
        track.mute = !track.mute;
        mute.setAttribute('aria-pressed', track.mute ? 'true' : 'false');
        setStatus(track.name + (track.mute ? ' muted' : ' unmuted'));
        return;
      }
      const step = event.target.closest('.step');
      if (!step) return;
      const ti = Number(step.dataset.track);
      const si = Number(step.dataset.step);
      const track = tracks[ti];
      if (!track) return;
      if (event.shiftKey || event.altKey) {
        cycleVelocity(track, si);
      } else {
        const wasOn = track.steps[si];
        track.steps[si] = wasOn ? 0 : 1;
        if (!wasOn && !track.vels[si]) track.vels[si] = 0.8;
      }
      updateStep(step, track, si);
      setRovingStep(step);
      setStatus(track.name + ' step ' + pad(si + 1) + (track.steps[si] ? ' on' : ' off') + ' · velocity ' + Math.round(track.vels[si] * 100) + '%');
    });

    seq.addEventListener('contextmenu', (event) => {
      if (seqState.rendering) return;
      const step = event.target.closest('.step');
      if (!step) return;
      event.preventDefault();
      const track = tracks[Number(step.dataset.track)];
      const index = Number(step.dataset.step);
      if (!track) return;
      cycleVelocity(track, index);
      updateStep(step, track, index);
      setRovingStep(step);
      setStatus(track.name + ' step ' + pad(index + 1) + ' velocity ' + Math.round(track.vels[index] * 100) + '%');
    });

    seq.addEventListener('keydown', (event) => {
      const step = event.target.closest('.step');
      if (!step) return;
      const ti = Number(step.dataset.track);
      const si = Number(step.dataset.step);
      let nextTrack = ti;
      let nextStep = si;
      if (event.key === 'ArrowRight') nextStep = (si + 1) % 16;
      else if (event.key === 'ArrowLeft') nextStep = (si + 15) % 16;
      else if (event.key === 'ArrowDown') nextTrack = (ti + 1) % tracks.length;
      else if (event.key === 'ArrowUp') nextTrack = (ti - 1 + tracks.length) % tracks.length;
      else if (event.key === 'Home') nextStep = 0;
      else if (event.key === 'End') nextStep = 15;
      else return;
      event.preventDefault();
      const next = seq.querySelector('.step[data-track="' + nextTrack + '"][data-step="' + nextStep + '"]');
      if (next) {
        setRovingStep(next);
        next.focus();
      }
    });

    const consoleEl = $('#console');
    if (consoleEl) {
      consoleEl.tabIndex = 0;
      consoleEl.addEventListener('keydown', (event) => {
        if (event.code !== 'Space' || event.repeat) return;
        if (event.target.closest('button, input, select, textarea, a')) return;
        event.preventDefault();
        toggleSequencer();
      });
    }
  }

  /* ---------------- instrument family voices ---------------- */

  function playFamilyNote(ctx, dest, time, freq, duration, code, level, pan, seed) {
    const gainLevel = level == null ? 1 : level;
    const rnd = mulberry32(((Number(seed) || 1) * 2654435761) >>> 0);
    const v1 = rnd();
    const v2 = rnd();
    const v3 = rnd();
    const v4 = rnd();
    const out = panOut(ctx, dest, pan || 0);
    const attack = code === 'P' || code === 'D' ? 0.004 : 0.01 + v4 * 0.02;

    if (code === 'S') {
      const filter = ctx.createBiquadFilter();
      filter.type = 'lowpass';
      filter.Q.value = 1.6 + v1 * 5.4;
      filter.frequency.setValueAtTime(900 + v2 * 2400, time);
      filter.frequency.exponentialRampToValueAtTime(260 + v3 * 700, time + duration);
      const gain = envelope(ctx, time, 0.11 * gainLevel, attack, duration);
      filter.connect(gain);
      gain.connect(out);
      const spread = 3 + v4 * 16;
      [-spread, spread].forEach((detune) => {
        const osc = ctx.createOscillator();
        osc.type = v1 > 0.8 ? 'square' : 'sawtooth';
        osc.frequency.value = freq;
        osc.detune.value = detune;
        osc.connect(filter);
        osc.start(time);
        osc.stop(time + duration + 0.08);
      });
      if (v3 > 0.4) {
        const sub = ctx.createOscillator();
        sub.type = 'sine';
        sub.frequency.value = freq / 2;
        const subGain = ctx.createGain();
        subGain.gain.value = 0.12 + v4 * 0.18;
        sub.connect(subGain);
        subGain.connect(gain);
        sub.start(time);
        sub.stop(time + duration + 0.08);
      }
      return;
    }

    if (code === 'W') {
      const real = new Float32Array([0, 0, 0, 0, 0, 0, 0, 0]);
      const imag = new Float32Array([
        0,
        1,
        0.24 + v1 * 0.48,
        0.1 + v2 * 0.34,
        0.05 + v3 * 0.22,
        0.03 + v4 * 0.14,
        0.02,
        0.01
      ]);
      const wave = ctx.createPeriodicWave(real, imag);
      const filter = ctx.createBiquadFilter();
      filter.type = 'lowpass';
      filter.frequency.setValueAtTime(1800 + v1 * 3600, time);
      filter.frequency.exponentialRampToValueAtTime(700 + v2 * 1700, time + duration);
      const gain = envelope(ctx, time, 0.095 * gainLevel, attack, duration);
      filter.connect(gain);
      gain.connect(out);
      const spread = 2 + v3 * 14;
      [-spread, spread].forEach((detune) => {
        const osc = ctx.createOscillator();
        osc.setPeriodicWave(wave);
        osc.frequency.value = freq;
        osc.detune.value = detune;
        osc.connect(filter);
        osc.start(time);
        osc.stop(time + duration + 0.08);
      });
      return;
    }

    if (code === 'F') {
      const carrier = ctx.createOscillator();
      carrier.type = v4 > 0.55 ? 'triangle' : 'sine';
      carrier.frequency.value = freq;
      const modulator = ctx.createOscillator();
      modulator.type = 'sine';
      modulator.frequency.value = freq * (1.5 + v1 * 3.5);
      const modGain = ctx.createGain();
      modGain.gain.setValueAtTime(freq * (0.7 + v2 * 3.4), time);
      modGain.gain.exponentialRampToValueAtTime(freq * (0.04 + v3 * 0.42), time + duration * 0.85);
      modulator.connect(modGain);
      modGain.connect(carrier.frequency);
      const gain = envelope(ctx, time, 0.1 * gainLevel, attack, duration);
      carrier.connect(gain);
      gain.connect(out);
      carrier.start(time);
      modulator.start(time);
      carrier.stop(time + duration + 0.08);
      modulator.stop(time + duration + 0.08);
      return;
    }

    if (code === 'A') {
      const gain = envelope(ctx, time, 0.075 * gainLevel, attack, duration);
      gain.connect(out);
      const partials = 4 + Math.floor(v1 * 6);
      for (let harmonic = 1; harmonic <= partials; harmonic += 1) {
        const osc = ctx.createOscillator();
        osc.type = 'sine';
        osc.frequency.value = freq * harmonic;
        osc.detune.value = (v3 - 0.5) * (harmonic % 2 ? 6 : -4);
        const partial = ctx.createGain();
        const odd = harmonic % 2 === 1 ? 1 : 0.55 + v4 * 0.35;
        partial.gain.value = (0.6 / Math.pow(harmonic, 0.7 + v2 * 0.8)) * odd;
        osc.connect(partial);
        partial.connect(gain);
        osc.start(time);
        osc.stop(time + duration + 0.08);
      }
      return;
    }

    if (code === 'P') {
      const source = ctx.createBufferSource();
      source.buffer = getNoise(ctx);
      const filter = ctx.createBiquadFilter();
      filter.type = 'lowpass';
      filter.frequency.value = 1800 + v1 * 3200;
      const delay = ctx.createDelay(1);
      delay.delayTime.value = clamp((0.8 + v2 * 0.5) / Math.max(40, freq), 0.001, 0.9);
      const feedback = ctx.createGain();
      feedback.gain.value = 0.84 + v3 * 0.11;
      const gain = ctx.createGain();
      gain.gain.setValueAtTime(0.0001, time);
      gain.gain.linearRampToValueAtTime((0.28 + v4 * 0.12) * gainLevel, time + 0.004);
      gain.gain.exponentialRampToValueAtTime(0.0001, time + 0.72 + v1 * 0.2);
      source.connect(filter);
      filter.connect(delay);
      delay.connect(feedback);
      feedback.connect(delay);
      delay.connect(gain);
      gain.connect(out);
      source.start(time);
      source.stop(time + 0.025);
      const body = ctx.createOscillator();
      body.type = 'sine';
      body.frequency.value = freq * (0.5 + v4 * 0.5);
      const bodyGain = envelope(ctx, time, 0.04 * gainLevel, 0.004, 0.2 + v3 * 0.18);
      body.connect(bodyGain);
      bodyGain.connect(out);
      body.start(time);
      body.stop(time + 0.42);
      return;
    }

    if (code === 'D') {
      heroKick(ctx, dest, time, 0.8 * gainLevel, {
        start: 118 + v1 * 86,
        end: 34 + v2 * 22,
        decay: 0.3 + v3 * 0.22,
        click: 0.08 + v4 * 0.18
      });
      heroHat(ctx, dest, time + 0.03, (0.45 + v2 * 0.3) * gainLevel, v3 > 0.7);
      return;
    }

    if (code === 'G') {
      const grains = 5 + Math.floor(v1 * 8);
      for (let i = 0; i < grains; i += 1) {
        const source = ctx.createBufferSource();
        source.buffer = getNoise(ctx);
        const band = ctx.createBiquadFilter();
        band.type = 'bandpass';
        band.frequency.value = 300 + v2 * 2600 + rnd() * 900;
        band.Q.value = 1.6 + v3 * 5.4;
        const gain = ctx.createGain();
        const start = time + rnd() * duration * 0.6;
        gain.gain.setValueAtTime(0.0001, start);
        gain.gain.exponentialRampToValueAtTime((0.03 + v4 * 0.04) * gainLevel, start + 0.007);
        gain.gain.exponentialRampToValueAtTime(0.0001, start + 0.07 + v1 * 0.06);
        source.connect(band);
        band.connect(gain);
        gain.connect(out);
        source.start(start);
        source.stop(start + 0.15);
      }
      return;
    }

    const osc = ctx.createOscillator();
    osc.type = ['triangle', 'sawtooth', 'square'][Math.floor(v1 * 3)];
    osc.frequency.value = freq;
    const formant = ctx.createBiquadFilter();
    formant.type = 'bandpass';
    formant.frequency.value = 500 + v2 * 2400;
    formant.Q.value = 2 + v3 * 8;
    const gain = envelope(ctx, time, (0.1 + v4 * 0.06) * gainLevel, attack, duration);
    osc.connect(formant);
    formant.connect(gain);
    gain.connect(out);
    osc.start(time);
    osc.stop(time + duration + 0.08);
  }

  function scheduleFamilyAudition(ctx, dest, inst, startTime) {
    const patterns = {
      S: [57, 60, 64, 67],
      W: [57, 64, 60, 67],
      F: [45, 57, 60, 64],
      A: [57, 60, 64, 69],
      P: [45, 52, 57, 64],
      D: [36, 36, 43, 36],
      G: [57, 64, 69, 76],
      X: [57, 60, 64, 67]
    };
    const seed = Number(inst.id) || 1;
    const notes = patterns[inst.code] || patterns.S;
    const transpose = ((seed % 5) - 2) * 2;
    const spacing = 0.19 + (seed % 4) * 0.012;
    notes.forEach((midi, index) => {
      playFamilyNote(
        ctx,
        dest,
        startTime + index * spacing,
        midiToFreq(midi + transpose),
        inst.code === 'D' ? 0.45 : 0.55,
        inst.code,
        0.9,
        index % 2 ? 0.16 : -0.16,
        seed
      );
    });
  }

  function initInstruments() {
    const grid = $('#instrument-grid');
    if (!grid) return;
    const filters = $('#instrument-filters');
    const search = $('#instrument-search');
    const count = $('#instrument-count');
    const dialog = $('#instrument-dialog');
    const dialogClose = $('#dialog-close');
    const dialogAudition = $('#dialog-audition');

    if (!DATA_READY || !DATA.instruments.length) {
      if (count) count.textContent = '—';
      if (filters) filters.innerHTML = '';
      if (search) {
        search.value = '';
        search.disabled = true;
      }
      grid.innerHTML = '<div class="empty-state" role="status"><strong>Catalog unavailable.</strong><p>The instrument data did not load. Check your connection and reload the page.</p></div>';
      return;
    }

    const counts = {};
    FAMILY_ORDER.forEach((code) => {
      counts[code] = DATA.instruments.filter((item) => item.code === code).length;
    });

    const state = {
      family: 'all',
      query: '',
      current: null,
      opener: null,
      playingId: null,
      timer: 0,
      btn: null,
      master: null
    };
    let searchTimer = 0;

    function renderFamilyChrome() {
      const bar = $('#family-bar');
      if (bar) {
        bar.innerHTML = FAMILY_ORDER.map((code) => {
          const n = counts[code];
          if (!n) return '';
          return '<span class="family-seg" style="--family:' + familyColor(code) + ';flex:' + n + '" title="' + esc(DATA.families[code] ? DATA.families[code].name : code) + ': ' + n + '"></span>';
        }).join('');
      }
      const legend = $('#family-legend');
      if (legend) {
        legend.innerHTML = FAMILY_ORDER.map((code) => {
          const family = DATA.families[code] || { name: code };
          return '<div class="family-legend-item" style="--family:' + familyColor(code) + '">' +
            '<span class="swatch" aria-hidden="true"></span>' +
            '<span class="legend-name">' + esc(family.name) + '</span>' +
            '<span class="legend-count">' + counts[code] + '</span>' +
          '</div>';
        }).join('');
      }
    }

    function chipHtml(label, total, color, value, active) {
      const dot = color ? '<span class="chip-dot" aria-hidden="true"></span>' : '';
      return '<button class="chip" type="button" data-filter="' + value + '" aria-pressed="' + (active ? 'true' : 'false') + '"' + (color ? ' style="--family:' + color + '"' : '') + '>' +
        dot + esc(label) + '<span class="chip-count">' + total + '</span>' +
      '</button>';
    }

    function renderFilters() {
      if (!filters) return;
      filters.innerHTML = chipHtml('All', DATA.instruments.length, null, 'all', state.family === 'all') +
        FAMILY_ORDER.filter((code) => counts[code]).map((code) => {
          const family = DATA.families[code] || { name: code };
          return chipHtml(family.name, counts[code], familyColor(code), code, state.family === code);
        }).join('');
    }

    function filtered() {
      const needle = state.query.trim().toLowerCase();
      return DATA.instruments.filter((inst) => {
        if (state.family !== 'all' && inst.code !== state.family) return false;
        if (!needle) return true;
        return [inst.name, inst.family, inst.capability, inst.blurb]
          .join(' ').toLowerCase().includes(needle);
      });
    }

    function cardHtml(inst) {
      const color = familyColor(inst.code);
      const trace = familyTrace(inst.code);
      return '<article class="catalog-card" style="--family:' + color + ';--trace:' + trace + '" data-card="' + inst.id + '">' +
        '<div class="catalog-card-head">' +
          '<span class="catalog-index">' + pad(inst.id) + ' / ' + DATA.instruments.length + '</span>' +
          '<span class="tag" style="--family:' + color + '">' + esc(inst.family) + '</span>' +
        '</div>' +
        '<h3 class="catalog-title"><button type="button" data-open="' + inst.id + '">' + esc(inst.name) + '</button></h3>' +
        '<p class="catalog-cap">' + esc(inst.capability) + '</p>' +
        '<div class="catalog-wave" aria-hidden="true"><svg viewBox="0 0 200 60" preserveAspectRatio="none"><path d="' + wavePath(inst.id, inst.code) + '"></path></svg></div>' +
        '<div class="catalog-card-foot">' +
          '<span class="catalog-family">' + esc(inst.family) + '</span>' +
          '<div class="catalog-actions">' +
            '<button class="card-audition" type="button" data-audition="' + inst.id + '" aria-pressed="false" aria-label="Play browser sketch of ' + esc(inst.name) + '">' +
              '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4 2.5v11l9-5.5z"/></svg>' +
            '</button>' +
            '<button class="card-open" type="button" data-open="' + inst.id + '" aria-label="Open engine notes for ' + esc(inst.name) + '">↗</button>' +
          '</div>' +
        '</div>' +
      '</article>';
    }

    function renderGrid() {
      stopAudition();
      const items = filtered();
      if (count) count.textContent = String(items.length);
      if (!items.length) {
        grid.innerHTML = '<div class="empty-state"><strong>No engines match.</strong><p>Try a different family or search term.</p><button class="btn" type="button" data-clear-filters>Clear filters</button></div>';
        return;
      }
      grid.innerHTML = items.map(cardHtml).join('');
    }

    function stopAudition() {
      clearTimeout(state.timer);
      state.timer = 0;
      if (state.master) {
        const master = state.master;
        state.master = null;
        try {
          const now = master.context.currentTime;
          master.gain.cancelScheduledValues(now);
          master.gain.setTargetAtTime(0.0001, now, 0.01);
          setTimeout(() => {
            try { master.disconnect(); } catch (error) { /* already disconnected */ }
          }, 80);
        } catch (error) {
          try { master.disconnect(); } catch (disconnectError) { /* already disconnected */ }
        }
      }
      if (state.btn) {
        state.btn.classList.remove('is-playing');
        state.btn.setAttribute('aria-pressed', 'false');
        if (state.btn.dataset.originalText) {
          state.btn.textContent = state.btn.dataset.originalText;
          delete state.btn.dataset.originalText;
        }
      }
      if (state.playingId != null) {
        const button = grid.querySelector('[data-audition="' + state.playingId + '"]');
        if (button) {
          button.classList.remove('is-playing');
          button.setAttribute('aria-pressed', 'false');
        }
      }
      state.playingId = null;
      state.btn = null;
    }

    function toggleAudition(inst, button) {
      if (state.playingId === inst.id) {
        stopAudition();
        return;
      }
      stopAudition();
      const ctx = audio.ensure();
      if (!ctx) {
        toast('Web Audio is not available in this browser.');
        return;
      }
      const master = ctx.createGain();
      master.gain.value = 0.72;
      const compressor = ctx.createDynamicsCompressor();
      compressor.threshold.value = -16;
      compressor.knee.value = 8;
      compressor.ratio.value = 3;
      compressor.attack.value = 0.004;
      compressor.release.value = 0.2;
      master.connect(compressor);
      compressor.connect(ctx.destination);
      state.master = master;
      scheduleFamilyAudition(ctx, master, inst, ctx.currentTime + 0.06);
      state.playingId = inst.id;
      state.btn = button;
      button.classList.add('is-playing');
      button.setAttribute('aria-pressed', 'true');
      if (button.id === 'dialog-audition') {
        button.dataset.originalText = button.textContent;
        button.textContent = 'Playing…';
      }
      state.timer = setTimeout(stopAudition, 2900);
      toast('Auditioning ' + inst.name + ' · per-engine browser sketch');
    }

    function openDialog(inst, opener) {
      if (!dialog || !inst) return;
      stopAudition();
      state.current = inst;
      state.opener = opener || null;
      const color = familyColor(inst.code);
      const trace = familyTrace(inst.code);
      const scope = $('.dialog-scope', dialog);
      if (scope) {
        scope.style.setProperty('--family', color);
        scope.style.setProperty('--trace', trace);
      }
      const setText = (id, value) => {
        const el = document.getElementById(id);
        if (el) el.textContent = value;
      };
      setText('dialog-family-label', inst.family + ' engine');
      setText('dialog-index', pad(inst.id) + ' / ' + DATA.instruments.length);
      setText('dialog-family-name', inst.family);
      setText('dialog-family-tag', inst.family);
      setText('dialog-code', 'Code ' + inst.code + ' · engine ' + pad(inst.id));
      setText('dialog-title', inst.name);
      setText('dialog-cap', inst.capability);
      setText('dialog-blurb', inst.blurb);
      setText('dialog-glance', 'One of ' + DATA.instruments.length + ' original engines. Shares the fixed 64-voice pool with every channel.');
      setText('dialog-note', inst.code === 'X'
        ? 'This engine needs user-supplied content. A synthetic fallback is generated in code; no sample library ships with REDRAW. The browser sketch varies that fallback per engine.'
        : 'The browser sketch varies this engine’s voice; the desktop engine uses the full instrument and its shared 64-voice pool.');
      const tag = $('#dialog-family-tag');
      if (tag) tag.style.setProperty('--family', color);
      const details = $('#dialog-details');
      if (details) {
        details.innerHTML = [
          'Family: ' + inst.family + ' (' + inst.code + ')',
          'Capability: ' + inst.capability,
          inst.code === 'X' ? 'Content: user-supplied samples with a generated fallback' : 'Content: generated in code; no preset or sample library',
          'Voice policy: shared 64-voice pool with oldest / lowest-velocity stealing'
        ].map((text) => '<li>' + esc(text) + '</li>').join('');
      }
      const wave = wavePath(inst.id, inst.code);
      const path = $('#dialog-wave');
      if (path) path.setAttribute('d', wave);
      const ghost = $('#dialog-wave-ghost');
      if (ghost) ghost.setAttribute('d', wave);
      if (dialogAudition) {
        dialogAudition.dataset.instrument = String(inst.id);
        if (!dialogAudition.dataset.originalText) dialogAudition.dataset.originalText = 'Play browser sketch';
        dialogAudition.textContent = 'Play browser sketch';
        dialogAudition.classList.remove('is-playing');
      }
      if (typeof dialog.showModal === 'function') {
        if (!dialog.open) dialog.showModal();
      } else {
        dialog.setAttribute('open', '');
      }
    }

    renderFamilyChrome();
    renderFilters();
    renderGrid();

    if (filters) {
      filters.addEventListener('click', (event) => {
        const chip = event.target.closest('[data-filter]');
        if (!chip) return;
        state.family = chip.dataset.filter;
        renderFilters();
        renderGrid();
      });
    }

    if (search) {
      search.addEventListener('input', () => {
        clearTimeout(searchTimer);
        searchTimer = setTimeout(() => {
          state.query = search.value;
          renderGrid();
        }, 110);
      });
    }

    grid.addEventListener('click', (event) => {
      const clear = event.target.closest('[data-clear-filters]');
      if (clear) {
        state.family = 'all';
        state.query = '';
        if (search) search.value = '';
        renderFilters();
        renderGrid();
        return;
      }
      const open = event.target.closest('[data-open]');
      if (open) {
        openDialog(instrumentById(open.dataset.open), open);
        return;
      }
      const audition = event.target.closest('[data-audition]');
      if (audition) {
        const inst = instrumentById(audition.dataset.audition);
        if (inst) toggleAudition(inst, audition);
      }
    });

    if (dialogClose) dialogClose.addEventListener('click', () => dialog && dialog.close());
    if (dialog) {
      dialog.addEventListener('click', (event) => {
        if (event.target === dialog) dialog.close();
      });
      dialog.addEventListener('keydown', (event) => {
        if (event.key !== 'Tab') return;
        const focusables = $$('a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])', dialog)
          .filter((el) => el.getClientRects().length);
        if (!focusables.length) return;
        const first = focusables[0];
        const last = focusables[focusables.length - 1];
        if (event.shiftKey && document.activeElement === first) {
          event.preventDefault();
          last.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first.focus();
        }
      });
      dialog.addEventListener('close', () => {
        stopAudition();
        if (state.opener && typeof state.opener.focus === 'function') state.opener.focus();
      });
    }
    if (dialogAudition) {
      dialogAudition.addEventListener('click', () => {
        const inst = state.current || instrumentById(dialogAudition.dataset.instrument);
        if (inst) toggleAudition(inst, dialogAudition);
      });
    }
  }

  /* ---------------- effect chain approximations ---------------- */

  function fxNode(ctx, code) {
    if (code === 'EQ') {
      const input = ctx.createGain();
      const low = ctx.createBiquadFilter();
      low.type = 'lowshelf';
      low.frequency.value = 180;
      low.gain.value = 1.5;
      const peak = ctx.createBiquadFilter();
      peak.type = 'peaking';
      peak.frequency.value = 1400;
      peak.Q.value = 0.9;
      peak.gain.value = 4;
      const high = ctx.createBiquadFilter();
      high.type = 'highshelf';
      high.frequency.value = 6500;
      high.gain.value = 2;
      input.connect(low);
      low.connect(peak);
      peak.connect(high);
      return { input, output: high };
    }
    if (code === 'DYN') {
      const comp = ctx.createDynamicsCompressor();
      comp.threshold.value = -24;
      comp.knee.value = 12;
      comp.ratio.value = 5;
      comp.attack.value = 0.004;
      comp.release.value = 0.2;
      return { input: comp, output: comp };
    }
    if (code === 'DST') {
      const shaper = ctx.createWaveShaper();
      shaper.curve = makeDriveCurve(2.4);
      shaper.oversample = '2x';
      const tone = ctx.createBiquadFilter();
      tone.type = 'lowpass';
      tone.frequency.value = 5400;
      shaper.connect(tone);
      return { input: shaper, output: tone };
    }
    if (code === 'MOD') {
      const input = ctx.createGain();
      const dry = ctx.createGain();
      dry.gain.value = 0.72;
      const wet = ctx.createGain();
      wet.gain.value = 0.42;
      const delay = ctx.createDelay(0.06);
      delay.delayTime.value = 0.018;
      const lfo = ctx.createOscillator();
      lfo.type = 'sine';
      lfo.frequency.value = 0.45;
      const depth = ctx.createGain();
      depth.gain.value = 0.004;
      lfo.connect(depth);
      depth.connect(delay.delayTime);
      input.connect(dry);
      input.connect(delay);
      delay.connect(wet);
      const out = ctx.createGain();
      dry.connect(out);
      wet.connect(out);
      return {
        input,
        output: out,
        start() { lfo.start(); },
        stop() { try { lfo.stop(); } catch (error) { /* already stopped */ } }
      };
    }
    if (code === 'DLY') {
      const input = ctx.createGain();
      const dry = ctx.createGain();
      dry.gain.value = 0.76;
      const delay = ctx.createDelay(1.2);
      delay.delayTime.value = 0.26;
      const feedback = ctx.createGain();
      feedback.gain.value = 0.32;
      const wet = ctx.createGain();
      wet.gain.value = 0.38;
      input.connect(dry);
      input.connect(delay);
      delay.connect(feedback);
      feedback.connect(delay);
      delay.connect(wet);
      const out = ctx.createGain();
      dry.connect(out);
      wet.connect(out);
      return { input, output: out };
    }
    if (code === 'REV') {
      const input = ctx.createGain();
      const dry = ctx.createGain();
      dry.gain.value = 0.66;
      const convolver = ctx.createConvolver();
      convolver.buffer = makeImpulse(ctx, 2.2, 2.6);
      const wet = ctx.createGain();
      wet.gain.value = 0.56;
      input.connect(dry);
      input.connect(convolver);
      convolver.connect(wet);
      const out = ctx.createGain();
      dry.connect(out);
      wet.connect(out);
      return { input, output: out };
    }
    if (code === 'PIT') {
      const input = ctx.createGain();
      const delay = ctx.createDelay(0.1);
      delay.delayTime.value = 0.012;
      const feedback = ctx.createGain();
      feedback.gain.value = 0.18;
      const filter = ctx.createBiquadFilter();
      filter.type = 'bandpass';
      filter.frequency.value = 1200;
      filter.Q.value = 0.7;
      input.connect(delay);
      delay.connect(feedback);
      feedback.connect(delay);
      delay.connect(filter);
      const out = ctx.createGain();
      input.connect(out);
      filter.connect(out);
      return { input, output: out };
    }
    if (code === 'V') {
      const input = ctx.createGain();
      const out = ctx.createGain();
      [650, 1080, 2400].forEach((frequency, index) => {
        const band = ctx.createBiquadFilter();
        band.type = 'bandpass';
        band.frequency.value = frequency;
        band.Q.value = 5;
        const gain = ctx.createGain();
        gain.gain.value = 0.55 / (index + 1);
        input.connect(band);
        band.connect(gain);
        gain.connect(out);
      });
      const dry = ctx.createGain();
      dry.gain.value = 0.22;
      input.connect(dry);
      dry.connect(out);
      return { input, output: out };
    }
    if (code === 'ANL') {
      const analyser = ctx.createAnalyser();
      analyser.fftSize = 512;
      return { input: analyser, output: analyser };
    }
    const gain = ctx.createGain();
    gain.gain.value = code === 'UTL' ? 0.88 : 1;
    return { input: gain, output: gain };
  }

  function buildEffectChain(ctx, effects) {
    const input = ctx.createGain();
    let node = input;
    const nodes = [];
    effects.forEach((effect) => {
      const processor = fxNode(ctx, effect.code);
      node.connect(processor.input);
      node = processor.output;
      nodes.push(processor);
    });
    const output = ctx.createGain();
    node.connect(output);
    return {
      input,
      output,
      nodes,
      start() {
        nodes.forEach((processor) => {
          if (processor.start) {
            try { processor.start(); } catch (error) { /* ignore */ }
          }
        });
      },
      stop() {
        nodes.forEach((processor) => {
          if (processor.stop) {
            try { processor.stop(); } catch (error) { /* ignore */ }
          }
        });
      }
    };
  }

  function playFxNote(ctx, dest, time, freq, duration, level) {
    const out = panOut(ctx, dest, 0);
    const filter = ctx.createBiquadFilter();
    filter.type = 'lowpass';
    filter.Q.value = 4;
    filter.frequency.setValueAtTime(3200, time);
    filter.frequency.exponentialRampToValueAtTime(680, time + duration);
    const gain = ctx.createGain();
    gain.gain.setValueAtTime(0.0001, time);
    gain.gain.exponentialRampToValueAtTime(0.16 * (level || 1), time + 0.007);
    gain.gain.exponentialRampToValueAtTime(0.0001, time + duration);
    const osc = ctx.createOscillator();
    osc.type = 'sawtooth';
    osc.frequency.value = freq;
    osc.connect(filter);
    filter.connect(gain);
    gain.connect(out);
    osc.start(time);
    osc.stop(time + duration + 0.03);
  }

  function scheduleFxPhrase(ctx, dest, start) {
    const notes = [57, 60, 64, 67, 64, 60, 57, 55];
    notes.forEach((midi, index) => {
      playFxNote(ctx, dest, start + index * 0.28, midiToFreq(midi), 0.62, 0.9);
    });
    for (let i = 0; i < 8; i += 1) {
      if (i % 2 === 0) heroKick(ctx, dest, start + i * 0.28, 0.55);
      heroHat(ctx, dest, start + i * 0.28 + 0.14, 0.42, false);
    }
  }

  function initEffects() {
    const list = $('#effect-list');
    if (!list) return;
    const filters = $('#effect-filters');
    const search = $('#effect-search');
    const count = $('#effect-count');
    const chainPanel = $('#chain-panel');
    const chainSlots = $('#chain-slots');
    const chainCount = $('#chain-count');
    const chainStatus = $('#chain-status');
    const chainAudition = $('#chain-audition');
    const chainClear = $('#chain-clear');

    if (!DATA_READY || !DATA.effects.length) {
      if (count) count.textContent = '—';
      if (filters) filters.innerHTML = '';
      if (search) {
        search.value = '';
        search.disabled = true;
      }
      if (chainStatus) chainStatus.textContent = 'Catalog unavailable — reload to build a chain.';
      if (chainAudition) chainAudition.disabled = true;
      if (chainClear) chainClear.disabled = true;
      list.innerHTML = '<div class="fx-empty" role="status"><strong>Catalog unavailable.</strong><p>The processor data did not load. Check your connection and reload the page.</p></div>';
      return;
    }

    const counts = {};
    CORE_ORDER.forEach((code) => {
      counts[code] = DATA.effects.filter((item) => item.code === code).length;
    });

    const state = {
      core: 'all',
      query: '',
      selected: [],
      audio: null,
      timer: 0
    };
    let searchTimer = 0;

    function renderCoreChrome() {
      const legend = $('#core-legend');
      if (!legend) return;
      legend.innerHTML = CORE_ORDER.map((code) => {
        const core = DATA.cores[code] || { name: code };
        return '<div class="core-legend-item" style="--core:' + coreColor(code) + '">' +
          '<span class="swatch" aria-hidden="true"></span>' +
          '<span class="legend-name">' + esc(core.name) + '</span>' +
          '<span class="legend-count">' + counts[code] + '</span>' +
        '</div>';
      }).join('');
    }

    function chipHtml(label, total, color, value, active) {
      const dot = color ? '<span class="chip-dot" aria-hidden="true"></span>' : '';
      return '<button class="chip" type="button" data-core="' + value + '" aria-pressed="' + (active ? 'true' : 'false') + '"' + (color ? ' style="--family:' + color + '"' : '') + '>' +
        dot + esc(label) + '<span class="chip-count">' + total + '</span>' +
      '</button>';
    }

    function renderFilters() {
      if (!filters) return;
      filters.innerHTML = chipHtml('All', DATA.effects.length, null, 'all', state.core === 'all') +
        CORE_ORDER.filter((code) => counts[code]).map((code) => {
          const core = DATA.cores[code] || { name: code };
          return chipHtml(core.name, counts[code], coreColor(code), code, state.core === code);
        }).join('');
    }

    function filtered() {
      const needle = state.query.trim().toLowerCase();
      return DATA.effects.filter((fx) => {
        if (state.core !== 'all' && fx.code !== state.core) return false;
        if (!needle) return true;
        return [fx.name, fx.capability, fx.coreName, fx.blurb]
          .join(' ').toLowerCase().includes(needle);
      });
    }

    function isSelected(fx) {
      return state.selected.some((item) => item.id === fx.id);
    }

    function renderList() {
      const items = filtered();
      if (count) count.textContent = String(items.length);
      if (!items.length) {
        list.innerHTML = '<div class="fx-empty"><strong>No processors match.</strong><p>Try a different core or search term.</p><button class="btn" type="button" data-clear-effects>Clear filters</button></div>';
        return;
      }
      list.innerHTML = items.map((fx) => {
        const selected = isSelected(fx);
        return '<button class="fx-row' + (selected ? ' is-selected' : '') + '" type="button" data-effect="' + fx.id + '" aria-pressed="' + (selected ? 'true' : 'false') + '" style="--core:' + coreColor(fx.code) + '">' +
          '<span class="fx-num">' + pad(fx.id) + '</span>' +
          '<span class="fx-name">' + esc(fx.name) + '</span>' +
          '<span class="fx-core">' + esc(fx.coreName) + '</span>' +
          '<span class="fx-cap">' + esc(fx.capability) + '</span>' +
          '<span class="fx-add" aria-hidden="true"><span>+</span></span>' +
        '</button>';
      }).join('');
    }

    function updateRows() {
      $$('.fx-row', list).forEach((row) => {
        const fx = effectById(row.dataset.effect);
        if (!fx) return;
        const selected = isSelected(fx);
        row.classList.toggle('is-selected', selected);
        row.setAttribute('aria-pressed', selected ? 'true' : 'false');
      });
    }

    function chainLabel() {
      if (!state.selected.length) return 'Select up to four processors.';
      return state.selected.map((fx) => fx.name).join(' → ');
    }

    function renderChain() {
      if (!chainSlots) return;
      chainSlots.innerHTML = Array.from({ length: 4 }, (_, index) => {
        const fx = state.selected[index];
        if (!fx) {
          return '<div class="chain-slot empty">' +
            '<span class="chain-slot-num">' + pad(index + 1) + '</span>' +
            '<span class="chain-empty-copy">' + (index === 0 ? 'Choose a processor from the catalog' : 'Empty slot') + '</span>' +
          '</div>';
        }
        return '<div class="chain-slot filled" style="--core:' + coreColor(fx.code) + '">' +
          '<span class="chain-slot-num">' + pad(index + 1) + '</span>' +
          '<span><span class="chain-slot-name">' + esc(fx.name) + '</span><span class="chain-slot-core">' + esc(fx.coreName) + '</span></span>' +
          '<button class="chain-remove" type="button" data-remove="' + fx.id + '" aria-label="Remove ' + esc(fx.name) + '">×</button>' +
        '</div>';
      }).join('');
      if (chainCount) chainCount.textContent = state.selected.length + ' / 4';
      if (chainStatus) chainStatus.textContent = chainLabel();
      if (chainClear) chainClear.disabled = state.selected.length === 0;
    }

    function stopChainAudio() {
      clearTimeout(state.timer);
      state.timer = 0;
      if (state.audio) {
        try { state.audio.stop(); } catch (error) { /* ignore */ }
        try { state.audio.output.disconnect(); } catch (error) { /* ignore */ }
        state.audio = null;
      }
      if (chainPanel) chainPanel.classList.remove('playing');
      if (chainAudition) chainAudition.textContent = 'Audition chain';
      if (chainStatus) chainStatus.textContent = chainLabel();
    }

    function startChainAudio() {
      const ctx = audio.ensure();
      if (!ctx) {
        toast('Web Audio is not available in this browser.');
        return;
      }
      stopChainAudio();
      const master = ctx.createGain();
      master.gain.value = 0.7;
      const compressor = ctx.createDynamicsCompressor();
      compressor.threshold.value = -14;
      compressor.knee.value = 8;
      compressor.ratio.value = 4;
      compressor.attack.value = 0.004;
      compressor.release.value = 0.2;
      master.connect(compressor);
      compressor.connect(ctx.destination);
      const built = buildEffectChain(ctx, state.selected);
      built.output.connect(master);
      built.start();
      state.audio = built;
      scheduleFxPhrase(ctx, built.input, ctx.currentTime + 0.07);
      if (chainPanel) chainPanel.classList.add('playing');
      if (chainAudition) chainAudition.textContent = 'Stop audition';
      if (chainStatus) {
        chainStatus.textContent = state.selected.length
          ? 'Playing ' + state.selected.map((fx) => fx.name).join(' → ')
          : 'Playing clean signal';
      }
      state.timer = setTimeout(stopChainAudio, 3900);
    }

    function toggleEffect(fx) {
      const index = state.selected.findIndex((item) => item.id === fx.id);
      if (index >= 0) {
        state.selected.splice(index, 1);
      } else {
        if (state.selected.length >= 4) {
          toast('Four serial slots are already filled.');
          shake(chainPanel);
          return;
        }
        state.selected.push(fx);
      }
      stopChainAudio();
      renderChain();
      updateRows();
    }

    renderCoreChrome();
    renderFilters();
    renderList();
    renderChain();

    if (filters) {
      filters.addEventListener('click', (event) => {
        const chip = event.target.closest('[data-core]');
        if (!chip) return;
        state.core = chip.dataset.core;
        renderFilters();
        renderList();
      });
    }

    if (search) {
      search.addEventListener('input', () => {
        clearTimeout(searchTimer);
        searchTimer = setTimeout(() => {
          state.query = search.value;
          renderList();
        }, 110);
      });
    }

    list.addEventListener('click', (event) => {
      const clear = event.target.closest('[data-clear-effects]');
      if (clear) {
        state.core = 'all';
        state.query = '';
        if (search) search.value = '';
        renderFilters();
        renderList();
        return;
      }
      const row = event.target.closest('[data-effect]');
      if (!row) return;
      const fx = effectById(row.dataset.effect);
      if (fx) toggleEffect(fx);
    });

    if (chainSlots) {
      chainSlots.addEventListener('click', (event) => {
        const remove = event.target.closest('[data-remove]');
        if (!remove) return;
        const fx = effectById(remove.dataset.remove);
        if (fx) toggleEffect(fx);
        const next = chainSlots.querySelector('.chain-remove');
        if (next) next.focus();
        else if (chainAudition) chainAudition.focus();
      });
    }

    if (chainClear) {
      chainClear.addEventListener('click', () => {
        if (!state.selected.length) return;
        state.selected = [];
        stopChainAudio();
        renderChain();
        updateRows();
        toast('Chain cleared.');
        if (chainAudition) chainAudition.focus();
      });
    }

    if (chainAudition) {
      chainAudition.addEventListener('click', () => {
        if (state.audio) stopChainAudio();
        else startChainAudio();
      });
    }
  }

  /* ---------------- engine page ---------------- */

  function heroClap(ctx, dest, time, level) {
    const source = ctx.createBufferSource();
    source.buffer = getNoise(ctx);
    const band = ctx.createBiquadFilter();
    band.type = 'bandpass';
    band.frequency.value = 1750;
    band.Q.value = 1.1;
    const gain = ctx.createGain();
    gain.gain.setValueAtTime(0.0001, time);
    gain.gain.exponentialRampToValueAtTime(0.16 * (level == null ? 1 : level), time + 0.004);
    gain.gain.exponentialRampToValueAtTime(0.0001, time + 0.11);
    source.connect(band);
    band.connect(gain);
    gain.connect(dest);
    source.start(time);
    source.stop(time + 0.13);
  }

  function scheduleExportPhrase(ctx, dest, bpm, bars) {
    const beat = 60 / bpm;
    const bar = beat * 4;
    const progression = [
      { chord: [57, 60, 64], bass: 45 },
      { chord: [53, 57, 60], bass: 41 },
      { chord: [48, 52, 55], bass: 36 },
      { chord: [55, 59, 62], bass: 43 }
    ];
    for (let b = 0; b < bars; b += 1) {
      const item = progression[b % progression.length];
      const start = 0.05 + b * bar;
      heroKeys(ctx, dest, start, item.chord.map(midiToFreq), 0.9, bar * 0.92);
      heroBass(ctx, dest, start, midiToFreq(item.bass), 0.82, beat * 1.55);
      heroBass(ctx, dest, start + beat * 2, midiToFreq(item.bass), 0.7, beat * 1.55);
      for (let k = 0; k < 4; k += 1) {
        heroKick(ctx, dest, start + k * beat, k === 0 ? 0.9 : 0.56);
        heroHat(ctx, dest, start + k * beat + beat * 0.5, 0.42, false);
      }
      heroClap(ctx, dest, start + beat, 0.72);
      heroClap(ctx, dest, start + beat * 3, 0.72);
      for (let i = 0; i < 8; i += 1) {
        const midi = item.chord[i % item.chord.length] + 12;
        playFxNote(ctx, dest, start + i * beat * 0.5, midiToFreq(midi), beat * 0.4, 0.46);
      }
    }
  }

  function drawExportIdle(canvas) {
    const fit = fitCanvas(canvas);
    if (!fit) return;
    const { ctx, width, height } = fit;
    ctx.clearRect(0, 0, width, height);
    drawGrid(ctx, width, height);
    ctx.strokeStyle = 'rgba(101, 200, 240, 0.2)';
    ctx.lineWidth = 1;
    ctx.setLineDash([3, 5]);
    ctx.beginPath();
    ctx.moveTo(0, height / 2 + 0.5);
    ctx.lineTo(width, height / 2 + 0.5);
    ctx.stroke();
    ctx.setLineDash([]);
    ctx.fillStyle = 'rgba(95, 112, 134, 0.8)';
    ctx.font = '9px "IBM Plex Mono", monospace';
    ctx.fillText('AWAITING RENDER', 14, 20);
  }

  function drawExportWaveform(canvas, left, right) {
    const fit = fitCanvas(canvas);
    if (!fit) return;
    const { ctx, width, height } = fit;
    ctx.clearRect(0, 0, width, height);
    drawGrid(ctx, width, height);
    const amplitude = height * 0.4;
    const drawChannel = (data, color, alpha) => {
      if (!data) return;
      const block = Math.max(1, Math.floor(data.length / width));
      ctx.strokeStyle = color;
      ctx.globalAlpha = alpha;
      ctx.lineWidth = 1;
      ctx.beginPath();
      for (let x = 0; x < width; x += 1) {
        const start = x * block;
        const end = Math.min(data.length, start + block);
        if (end <= start) continue;
        let min = 1;
        let max = -1;
        for (let i = start; i < end; i += 1) {
          const v = data[i];
          if (v < min) min = v;
          if (v > max) max = v;
        }
        const top = height / 2 - max * amplitude;
        const bottom = height / 2 - min * amplitude;
        ctx.moveTo(x + 0.5, top);
        ctx.lineTo(x + 0.5, bottom);
      }
      ctx.stroke();
    };
    drawChannel(left, 'rgba(101, 200, 240, 0.9)', 1);
    drawChannel(right, 'rgba(239, 78, 34, 0.55)', 0.8);
    ctx.globalAlpha = 1;
  }

  function initEngine() {
    const toneToggle = $('#tone-toggle');
    if (toneToggle) {
      const waveInput = $('#tone-wave');
      const freqInput = $('#tone-freq');
      const gainInput = $('#tone-gain');
      const freqOut = $('#tone-freq-out');
      const gainOut = $('#tone-gain-out');
      const toneStatus = $('#tone-status');
      const toneLive = $('#tone-live');
      const tonePanel = $('#tone-panel');
      const tone = {
        ctx: null,
        osc: null,
        gain: null,
        analyser: null,
        playing: false
      };
      const scope = makeScope($('#tone-scope'), () => tone.analyser, () => tone.playing);

      function syncLabels() {
        if (freqOut) freqOut.textContent = Number(freqInput.value) + ' Hz';
        if (gainOut) gainOut.textContent = Number(gainInput.value) + ' dB';
      }

      function stopTone() {
        if (!tone.ctx || !tone.playing) return;
        const ctx = tone.ctx;
        const now = ctx.currentTime;
        if (tone.gain) {
          tone.gain.gain.cancelScheduledValues(now);
          tone.gain.gain.setTargetAtTime(0.0001, now, 0.01);
        }
        if (tone.osc) {
          try { tone.osc.stop(now + 0.08); } catch (error) { /* already stopped */ }
        }
        const oldOsc = tone.osc;
        const oldGain = tone.gain;
        const oldAnalyser = tone.analyser;
        tone.osc = null;
        tone.gain = null;
        tone.analyser = null;
        tone.playing = false;
        setTimeout(() => {
          try { oldOsc && oldOsc.disconnect(); } catch (error) { /* ignore */ }
          try { oldGain && oldGain.disconnect(); } catch (error) { /* ignore */ }
          try { oldAnalyser && oldAnalyser.disconnect(); } catch (error) { /* ignore */ }
        }, 160);
        toneToggle.setAttribute('aria-pressed', 'false');
        toneToggle.textContent = 'Play test tone';
        if (toneLive) {
          toneLive.textContent = 'Idle';
          toneLive.classList.remove('is-on');
        }
        if (toneStatus) toneStatus.textContent = 'Stopped';
        if (tonePanel) tonePanel.classList.remove('playing');
        scope.refresh();
      }

      function startTone() {
        const ctx = audio.ensure();
        if (!ctx) {
          toast('Web Audio is not available in this browser.');
          return;
        }
        const analyser = ctx.createAnalyser();
        analyser.fftSize = 2048;
        analyser.smoothingTimeConstant = 0.7;
        const gain = ctx.createGain();
        gain.gain.value = dbToGain(Number(gainInput.value));
        const osc = ctx.createOscillator();
        osc.type = waveInput ? waveInput.value : 'sawtooth';
        osc.frequency.value = Number(freqInput.value);
        osc.connect(gain);
        gain.connect(analyser);
        analyser.connect(ctx.destination);
        osc.start();
        tone.ctx = ctx;
        tone.osc = osc;
        tone.gain = gain;
        tone.analyser = analyser;
        tone.playing = true;
        toneToggle.setAttribute('aria-pressed', 'true');
        toneToggle.textContent = 'Stop test tone';
        if (toneLive) {
          toneLive.textContent = 'Live';
          toneLive.classList.add('is-on');
        }
        if (toneStatus) toneStatus.textContent = 'Live · browser oscillator';
        if (tonePanel) tonePanel.classList.add('playing');
        scope.refresh();
      }

      syncLabels();
      toneToggle.addEventListener('click', () => {
        if (tone.playing) stopTone();
        else startTone();
      });
      if (waveInput) {
        waveInput.addEventListener('change', () => {
          if (tone.osc) tone.osc.type = waveInput.value;
        });
      }
      if (freqInput) {
        freqInput.addEventListener('input', () => {
          syncLabels();
          if (tone.osc) tone.osc.frequency.setTargetAtTime(Number(freqInput.value), tone.ctx.currentTime, 0.02);
        });
      }
      if (gainInput) {
        gainInput.addEventListener('input', () => {
          syncLabels();
          if (tone.gain) tone.gain.gain.setTargetAtTime(dbToGain(Number(gainInput.value)), tone.ctx.currentTime, 0.02);
        });
      }
    }

    const flowNodes = $$('[data-flow]');
    const flowDetails = [
      {
        title: 'Voice pool',
        body: 'One fixed pool of 64 voices is shared across all channels. Allocation is round-robin; when the pool is full, the oldest or lowest-velocity voice is stolen. No voice allocates during playback.',
        tags: ['Round-robin', 'Voice stealing', 'Fixed capacity']
      },
      {
        title: 'Channel inserts',
        body: 'Every channel owns an eight-slot serial insert chain. Bypass is exact, parameters are smoothed, and denormal guards keep silent tails from becoming expensive. The chain is built off the audio thread and swapped at a block boundary.',
        tags: ['8 serial slots', 'Exact bypass', 'Smoothed parameters']
      },
      {
        title: 'Send buses',
        body: 'Four send buses exist in the project and engine model. They sum into the master path alongside the channel strips. The mixer currently exposes the routing in the model but does not draw a send knob on every strip.',
        tags: ['4 buses', 'Model + engine', 'UI pending']
      },
      {
        title: 'Strip sum',
        body: 'Sixty-four channel strips plus returns sum into the master path. Gain, pan, mute, solo and polarity are applied before the post-insert, pre-master peak meters. The master strip is pinned.',
        tags: ['64 strips', 'Gain / pan / polarity', 'Post-insert meters']
      },
      {
        title: 'Master inserts',
        body: 'The master owns its own eight-slot serial chain. Processors are the same original cores used on channels, with the same exact bypass and parameter smoothing.',
        tags: ['8 serial slots', 'Shared cores', 'Pinned master']
      },
      {
        title: 'Limiter',
        body: 'A look-ahead limiter sits at the end of the master chain. It bounds the output before the device, with the goal of catching peaks without asking the callback to allocate or wait.',
        tags: ['Look-ahead', 'Peak ceiling', 'No allocation']
      },
      {
        title: 'Meter',
        body: 'Peak meters are instantaneous: they show post-insert, pre-master levels and decay with the control block. There is no peak-hold and no loudness history yet.',
        tags: ['Instantaneous', 'No peak-hold', 'Control-rate decay']
      },
      {
        title: 'Device out',
        body: 'The default device stream is 48 kHz stereo float, rendered in 512-frame blocks — 93.75 blocks per second. Export accepts 8–192 kHz, with native WAV/AIFF and optional ffmpeg formats.',
        tags: ['48 kHz default', '512-frame blocks', 'Stereo f32']
      }
    ];
    const flowTitle = $('#flow-detail-title');
    const flowBody = $('#flow-detail-body');
    const flowTags = $('#flow-detail-tags');
    const flowIndex = $('#flow-index');

    function selectFlow(index) {
      const detail = flowDetails[index];
      if (!detail) return;
      flowNodes.forEach((node) => {
        const on = Number(node.dataset.flow) === index;
        node.classList.toggle('is-active', on);
        node.setAttribute('aria-pressed', on ? 'true' : 'false');
      });
      if (flowTitle) flowTitle.textContent = detail.title;
      if (flowBody) flowBody.textContent = detail.body;
      if (flowTags) flowTags.innerHTML = detail.tags.map((tag) => '<span>' + esc(tag) + '</span>').join('');
      if (flowIndex) flowIndex.textContent = pad(index + 1) + ' / ' + pad(flowDetails.length);
    }

    flowNodes.forEach((node) => {
      node.addEventListener('click', () => selectFlow(Number(node.dataset.flow)));
    });
    if (flowNodes.length) selectFlow(0);

    const exportButton = $('#export-render');
    if (exportButton) {
      const exportCanvas = $('#export-canvas');
      const exportFormatLabel = $('#export-format-label');
      const exportFrames = $('#export-frames');
      const exportDuration = $('#export-duration');
      const exportPeak = $('#export-peak');
      const exportSize = $('#export-size');
      const exportProgress = $('#export-progress');
      const exportStatus = $('#export-status');
      const exportDownload = $('#export-download');
      const rateSelect = $('#export-rate');
      const barsSelect = $('#export-bars');
      const exportControls = $$('#export-rate, #export-bars, input[name="export-format"]', exportButton.closest('.export-lab'));
      const setExportControlsDisabled = (disabled) => {
        exportControls.forEach((control) => { control.disabled = disabled; });
      };
      let lastRender = null;
      let exportUrl = null;

      drawExportIdle(exportCanvas);
      window.addEventListener('resize', () => {
        if (lastRender) drawExportWaveform(exportCanvas, lastRender[0], lastRender[1]);
        else drawExportIdle(exportCanvas);
      });

      exportButton.addEventListener('click', async () => {
        if (exportButton.disabled) return;
        const Ctor = window.OfflineAudioContext || window.webkitOfflineAudioContext;
        if (!Ctor) {
          toast('Offline audio rendering is not available in this browser.');
          return;
        }
        const formatInput = document.querySelector('input[name="export-format"]:checked');
        const format = formatInput ? formatInput.value : '16';
        const rate = Number(rateSelect ? rateSelect.value : 48000);
        const bars = Number(barsSelect ? barsSelect.value : 4);
        const formatName = format === 'float32' ? '32-bit float' : format + '-bit PCM';
        exportButton.disabled = true;
        setExportControlsDisabled(true);
        if (exportDownload) exportDownload.classList.remove('is-visible');
        exportButton.textContent = 'Rendering…';
        if (exportStatus) exportStatus.textContent = 'Rendering ' + bars + ' bars at ' + fmtInt(rate) + ' Hz…';
        if (exportProgress) exportProgress.style.width = '8%';
        let progress = 8;
        const progressTimer = setInterval(() => {
          progress = Math.min(88, progress + 6);
          if (exportProgress) exportProgress.style.width = progress + '%';
        }, 180);
        try {
          const bpm = 112;
          const beat = 60 / bpm;
          const bar = beat * 4;
          const duration = bars * bar + 1.05;
          const frames = Math.ceil(duration * rate);
          const offline = new Ctor(2, frames, rate);
          const master = offline.createGain();
          master.gain.value = 0.72;
          const compressor = offline.createDynamicsCompressor();
          compressor.threshold.value = -14;
          compressor.knee.value = 10;
          compressor.ratio.value = 3;
          compressor.attack.value = 0.004;
          compressor.release.value = 0.2;
          master.connect(compressor);
          compressor.connect(offline.destination);
          scheduleExportPhrase(offline, master, bpm, bars);
          const rendered = await offline.startRendering();
          const channels = [rendered.getChannelData(0), rendered.getChannelData(1)];
          let peak = measurePeak(channels);
          if (peak > 0.985) {
            scaleChannels(channels, 0.985 / peak);
            peak = 0.985;
          }
          const wav = encodeWav(channels, rate, format);
          const blob = new Blob([wav], { type: 'audio/wav' });
          if (exportUrl) URL.revokeObjectURL(exportUrl);
          exportUrl = URL.createObjectURL(blob);
          lastRender = channels;
          drawExportWaveform(exportCanvas, channels[0], channels[1]);
          if (exportFormatLabel) exportFormatLabel.textContent = formatName + ' · ' + fmtInt(rate) + ' Hz';
          if (exportFrames) exportFrames.textContent = fmtInt(frames);
          if (exportDuration) exportDuration.textContent = formatDuration(frames / rate);
          if (exportPeak) exportPeak.textContent = peak > 0 ? (20 * Math.log10(peak)).toFixed(1) + ' dBFS' : '−∞ dBFS';
          if (exportSize) exportSize.textContent = formatBytes(wav.byteLength);
          if (exportDownload) {
            exportDownload.href = exportUrl;
            exportDownload.download = 'redraw-export-lab-' + rate + '-' + format + '.wav';
            exportDownload.classList.add('is-visible');
          }
          if (exportStatus) exportStatus.textContent = 'Rendered ' + fmtInt(frames) + ' frames · ready to download.';
          if (exportProgress) exportProgress.style.width = '100%';
          toast('Rendered ' + fmtInt(frames) + ' frames · ' + formatBytes(wav.byteLength) + ' · ' + formatName);
        } catch (error) {
          console.error(error);
          if (exportStatus) exportStatus.textContent = 'Render failed. Try again.';
          if (exportProgress) exportProgress.style.width = '0';
          if (exportDownload) exportDownload.classList.remove('is-visible');
          toast('Render failed. Try again.');
        } finally {
          clearInterval(progressTimer);
          exportButton.disabled = false;
          setExportControlsDisabled(false);
          exportButton.textContent = 'Render WAV';
        }
      });
    }
  }

  initChrome();
  initReveal();
  if (PAGE === 'home') initHome();
  if (PAGE === 'instruments') initInstruments();
  if (PAGE === 'effects') initEffects();
  if (PAGE === 'engine') initEngine();
})();
