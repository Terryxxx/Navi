const icons = {
  check: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"></polyline></svg>`,
  warning: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"></path><line x1="12" y1="9" x2="12" y2="13"></line><line x1="12" y1="17" x2="12.01" y2="17"></line></svg>`,
  info: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"></circle><line x1="12" y1="16" x2="12" y2="12"></line><line x1="12" y1="8" x2="12.01" y2="8"></line></svg>`
};

const eventTypeMap = {
  stop: { type: 'success', title: 'Task Complete', icon: 'check' },
  permission: { type: 'warning', title: 'Permission Needed', icon: 'warning' },
  idle: { type: 'info', title: 'Waiting for Input', icon: 'info' },
  info: { type: 'info', title: 'Notification', icon: 'info' }
};

const audioCtx = new (window.AudioContext || window.webkitAudioContext)();
const soundBuffers = {};

const lowPass = audioCtx.createBiquadFilter();
lowPass.type = 'lowpass';
lowPass.frequency.value = 2000;
const masterGain = audioCtx.createGain();
masterGain.gain.value = 0.15;
lowPass.connect(masterGain);
masterGain.connect(audioCtx.destination);

async function loadSound(name, url) {
  try {
    const res = await fetch(url);
    const buf = await res.arrayBuffer();
    soundBuffers[name] = await audioCtx.decodeAudioData(buf);
  } catch (e) {
    console.error(`[Navi] Failed to load sound ${name}:`, e);
  }
}

loadSound('success', './sounds/success.wav');
loadSound('error', './sounds/error.mp3');

function playSound(name) {
  if (!soundBuffers[name]) return;
  if (audioCtx.state === 'suspended') audioCtx.resume();
  const source = audioCtx.createBufferSource();
  source.buffer = soundBuffers[name];
  source.playbackRate.value = 0.98 + Math.random() * 0.04;
  source.connect(lowPass);
  source.start(0);
}

const eventSoundMap = {
  success: 'success',
  warning: 'error',
  info: 'success'
};

function sanitizeString(val, maxLen = 200) {
  if (typeof val !== 'string') return '';
  return val.slice(0, maxLen);
}

const island = document.getElementById('island');
const iconEl = document.getElementById('icon');
const titleEl = document.getElementById('title');
const messageEl = document.getElementById('message');
const projectEl = document.getElementById('project');

const queue = [];
let isShowing = false;

function showNext() {
  if (queue.length === 0) { isShowing = false; return; }
  isShowing = true;

  const data = queue.shift();

  const eventInfo = eventTypeMap[data.event] || eventTypeMap.stop;
  const type = data.type || eventInfo.type;

  iconEl.innerHTML = icons[eventInfo.icon] || icons.check;
  titleEl.textContent = sanitizeString(data.title || eventInfo.title);
  messageEl.textContent = sanitizeString(data.message || '');
  projectEl.textContent = sanitizeString(data.project || '', 60);

  island.className = 'island expanded type-' + type;

  playSound(eventSoundMap[type] || 'success');

  const duration = (typeof data.duration === 'number' && data.duration > 0) ? data.duration : 4500;
  setTimeout(() => {
    // Freeze current width so the collapse animates from the actual rendered size, not `auto`.
    const currentWidth = island.getBoundingClientRect().width;
    island.style.width = currentWidth + 'px';
    island.offsetHeight; // force reflow so the width above is committed before the class change

    island.classList.remove('expanded');
    island.classList.add('collapsed');
    island.style.width = '';

    setTimeout(showNext, 600); // wait out the collapse transition before the next pill
  }, duration);
}

function initTauri() {
  if (window.__TAURI__) {
    window.__TAURI__.event.listen('show-notification', (e) => {
      queue.push(e.payload);
      if (!isShowing) showNext();
    });
  } else {
    setTimeout(initTauri, 50);
  }
}

document.addEventListener('DOMContentLoaded', initTauri);
