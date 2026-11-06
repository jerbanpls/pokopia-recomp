import { api, esc, icon, refreshIcons, toast, errText } from '../util.js';

// Link Play: play alone, host on this PC (the launcher runs the relay), or visit a host. The choice lives in pkp.ini
// ([net] server / room_code), which the game reads; the host's own code and the visitor's last entries are also kept in
// localStorage so switching between the ways does not lose them. Flow modelled on pokopia-native's Link Play window.
const DEFAULT_PORT = 39660;
const LS = 'pkp.linkplay';

const stored = () => { try { return JSON.parse(localStorage.getItem(LS)) || {}; } catch { return {}; } };
const keep = (patch) => { try { localStorage.setItem(LS, JSON.stringify({ ...stored(), ...patch })); } catch { /* private window: fine */ } };

const isLoopback = (s) => /^(127\.0\.0\.1|localhost)(:\d+)?$/i.test(s.trim());
const edit = (key, value) => ({ file: 'pkp', section: 'net', key, value });

export async function render(el) {
  const data = await api('read_settings');
  let server = (data.pkp['net.server'] || '').trim();
  const iniCode = (data.pkp['net.room_code'] || '').trim();
  const mem = stored();
  let mode = !server ? 'alone' : isLoopback(server) ? 'host' : 'visit';
  let relay = await api('relay_status');
  let lan = null;
  // the host's code: the one in pkp.ini while hosting, else the remembered one
  let hostCode = mode === 'host' && iniCode && iniCode.toLowerCase() !== 'new' ? iniCode : mem.hostCode || '';
  let visitAddr = mode === 'visit' ? server : mem.visitAddr || '';
  let visitCode = mode === 'visit' ? iniCode : mem.visitCode || '';
  let picked = mode;   // the radio selection; Visit is applied by its Save button
  // [pkp] link_interact_everywhere (pkp_linkplay.cpp): every player of a session needs the same value, the game refuses the others
  let interact = /^(1|true|yes|on)$/i.test((data.pkp['pkp.link_interact_everywhere'] || '').trim());

  async function loadLan() {
    try { lan = await api('lan_report'); } catch { lan = null; }
  }

  const write = (edits) => api('write_settings', { edits });

  function addressList() {
    const port = relay ? relay.addr.split(':').pop() : DEFAULT_PORT;
    const rows = [];
    if (lan) {
      lan.lan.forEach((a, i) => rows.push({ addr: `${a.ip}:${port}`, note: `${i === 0 ? 'same Wi-Fi or network' : 'other network'} (${a.adapter})` }));
      lan.overlay.forEach((a) => rows.push({ addr: `${a.ip}:${port}`, note: `${a.label} (${a.adapter})` }));
    }
    return rows;
  }

  function hostHtml() {
    const running = !!relay;
    const rows = addressList();
    const hint = lan && lan.firewall_hint ? `<div class="dim">${esc(lan.firewall_hint)}</div>` : '';
    const warn = lan && lan.warning ? `<div class="banner warn">${icon('triangle-alert')}<div>${esc(lan.warning)}${hint}</div></div>` : '';
    const noAddr = lan && !lan.lan.length && !lan.overlay.length
      ? `<div class="banner warn">${icon('triangle-alert')}<div>No usable network address found. Connect to a network, then reopen this page.${hint}</div></div>` : '';
    const stopped = mode === 'host' && !running
      ? `<div class="banner info">${icon('info')}<div>The relay is not running (the launcher was restarted). Press Start hosting again.</div></div>` : '';
    return `
      <div class="card">
        <h2>${icon('server')}Host on this PC</h2>
        ${stopped}
        <p class="dim">The launcher runs the Link Play relay while it stays open. Everybody, you too, connects to it.</p>
        <div class="row wrap" style="margin: var(--space-4) 0">
          ${running
            ? `<span class="chip ok">${icon('circle-check')}Hosting on ${esc(relay.addr)}, ${relay.clients} connected</span><button class="btn" id="stop">Stop hosting</button>`
            : `<button class="btn primary" id="start">${icon('play')}Start hosting</button>`}
        </div>
        ${running ? `${warn}${noAddr}
        <div class="field"><label class="name">Address to give out<span class="hint">Visitors type one of these. The first is for your own network.</span></label>
          <div class="stack">${rows.length ? rows.map((r) => `<div class="row wrap"><code>${esc(r.addr)}</code><span class="faint">${esc(r.note)}</span><button class="btn quiet" data-copy="${esc(r.addr)}">${icon('copy')}Copy</button></div>`).join('') : '<span class="faint">none found</span>'}</div></div>
        <div class="field"><label class="name">Room code<span class="hint">Keeps strangers on a shared relay out. Visitors enter it too. Stays the same until you press New code.</span></label>
          <div class="control"><code>${esc(hostCode)}</code><button class="btn quiet" id="copy-code">${icon('copy')}Copy</button><button class="btn quiet" id="new-code">${icon('refresh-cw')}New code</button></div></div>` : ''}
      </div>`;
  }

  function visitHtml() {
    return `
      <div class="card">
        <h2>${icon('wifi')}Visit a host</h2>
        <p class="dim">Ask the host for their address and room code (their Link Play page shows both).</p>
        <div class="field"><label class="name">Host address<span class="hint">Like 192.168.1.20:${DEFAULT_PORT}. The port is optional.</span></label>
          <div class="control"><input type="text" id="v-addr" spellcheck="false" placeholder="address:${DEFAULT_PORT}" value="${esc(visitAddr)}"></div></div>
        <div class="field"><label class="name">Room code<span class="hint">XXXX-XXXX-XXXX-XXXX. Leave empty if the host has none.</span></label>
          <div class="control"><input type="text" id="v-code" spellcheck="false" placeholder="XXXX-XXXX-XXXX-XXXX" value="${esc(visitCode)}"></div></div>
        <div class="row" style="margin-top: var(--space-4)"><button class="btn primary" id="v-save">${icon('save')}Save</button>
          ${mode === 'visit' ? `<span class="chip ok">${icon('circle-check')}Visiting ${esc(server)}</span>` : ''}</div>
      </div>`;
  }

  function draw() {
    const opt = (id, title, sub) => `<label class="candidate"><input type="radio" name="mode" value="${id}" ${picked === id ? 'checked' : ''}><div><b>${title}</b><div class="dim">${sub}</div></div></label>`;
    el.innerHTML = `
      <div class="page">
        <div class="hero"><h1>Link Play</h1><p class="dim">Play with other people, on the same network or over the internet.</p></div>
        <div class="card">
          <h2>${icon('users')}How do you want to play?</h2>
          <div class="stack">
            ${opt('alone', 'Alone', 'No Link Play. Local wireless in the game finds nobody.')}
            ${opt('host', 'Host on this PC', 'Others join you. The launcher runs the relay and gives you the address.')}
            ${opt('visit', 'Visit', "Join a host's relay with their address and room code.")}
          </div>
        </div>
        ${picked === 'host' ? hostHtml() : ''}
        ${picked === 'visit' ? visitHtml() : ''}
        ${picked !== 'alone' ? `<div class="card">
          <label class="candidate"><input type="checkbox" id="interact" ${interact ? 'checked' : ''}><div><b>Interact everywhere (experimental)</b>
            <div class="dim">Outside Palette Town the game puts everyone in Spectator Mode. This lets visitors use moves and items, pick up, place and build in the story areas too. Every player must turn it on, or they cannot join.</div></div></label>
        </div>` : ''}
        ${picked !== 'alone' ? `<div class="banner info">${icon('info')}<div>
          In the game, open Link Play and pick <b>Local connection</b> (Online connection is Nintendo's service). Each player needs their own save: a copied save cannot join its original.${picked === 'host' ? ' Keep the launcher open while you host.' : ''} Changes apply the next time the game starts.</div></div>` : ''}
      </div>`;
    refreshIcons();
    bind();
  }

  async function setAlone() {
    await write([edit('server', '')]);
    server = '';
    mode = 'alone';
  }

  async function startHost() {
    if (!hostCode) hostCode = await api('room_code_new');
    relay = await api('relay_start', { port: DEFAULT_PORT, password: null });
    server = `127.0.0.1:${relay.addr.split(':').pop()}`;
    await write([edit('server', server), edit('room_code', hostCode)]);
    keep({ hostCode });
    mode = 'host';
    await loadLan();
  }

  async function saveVisit() {
    const addr = el.querySelector('#v-addr').value.trim();
    const raw = el.querySelector('#v-code').value.trim();
    if (!addr || /\s/.test(addr)) return toast('Type the host address, like 192.168.1.20:39660.', 'error');
    if (isLoopback(addr)) return toast('That is this PC. Pick Host on this PC instead.', 'error');
    try {
      const code = raw ? await api('room_code_check', { code: raw }) : '';
      await write([edit('server', addr), edit('room_code', code)]);
      server = addr; mode = 'visit'; visitAddr = addr; visitCode = code;
      keep({ visitAddr, visitCode });
      toast('Saved. Start the game and pick Local connection.', 'ok');
      draw();
    } catch (e) { toast(errText(e), 'error'); }
  }

  const copy = async (text) => {
    try { await navigator.clipboard.writeText(text); toast('Copied.', 'ok'); } catch { toast('Cannot copy here. Select the text and press Ctrl+C.', 'error'); }
  };

  function bind() {
    el.querySelectorAll('input[name="mode"]').forEach((r) => r.addEventListener('change', async () => {
      picked = r.value;
      try {
        if (picked === 'alone') {
          if (relay) { await api('relay_stop'); relay = null; }
          await setAlone();
          toast('Link Play is off.', 'ok');
        } else if (picked === 'host') {
          await loadLan();
          if (!hostCode) hostCode = await api('room_code_new');
        }
      } catch (e) { toast(errText(e), 'error'); }
      draw();
    }));
    el.querySelector('#start')?.addEventListener('click', async () => {
      try { await startHost(); } catch (e) { toast(errText(e), 'error'); }
      draw();
    });
    el.querySelector('#stop')?.addEventListener('click', async () => {
      try {
        await api('relay_stop');
        relay = null;
        await setAlone();
        picked = 'alone';
      } catch (e) { toast(errText(e), 'error'); }
      draw();
    });
    el.querySelectorAll('[data-copy]').forEach((b) => b.addEventListener('click', () => copy(b.dataset.copy)));
    el.querySelector('#copy-code')?.addEventListener('click', () => copy(hostCode));
    el.querySelector('#new-code')?.addEventListener('click', async () => {
      try {
        hostCode = await api('room_code_new');
        await write([edit('room_code', hostCode)]);
        keep({ hostCode });
        toast('New room code. Tell your visitors; it applies the next time the game starts.', 'ok');
      } catch (e) { toast(errText(e), 'error'); }
      draw();
    });
    el.querySelector('#v-save')?.addEventListener('click', saveVisit);
    el.querySelector('#interact')?.addEventListener('change', async (ev) => {
      const on = ev.target.checked;
      try {
        await write([{ file: 'pkp', section: 'pkp', key: 'link_interact_everywhere', value: on ? 'true' : 'false' }]);
        interact = on;
        toast(on ? 'Interact everywhere is on. Every player needs it on; it applies the next time the game starts.' : 'Interact everywhere is off.', 'ok');
      } catch (e) { ev.target.checked = !on; toast(errText(e), 'error'); }
    });
  }

  if (picked === 'host') await loadLan();
  draw();
}
