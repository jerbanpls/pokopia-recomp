import { listen } from '@tauri-apps/api/event';
import { api, esc, icon, refreshIcons, toast, errText } from '../util.js';

// one listener for the whole session; the page being shown sets the handler
let dlssHandler = null;
listen('dlss-progress', (ev) => dlssHandler && dlssHandler(ev.payload));
let nupHandler = null;
listen('nup-progress', (ev) => nupHandler && nupHandler(ev.payload));

function candidateHtml(c, chosen) {
  const title = c.kind === 'base' ? 'Base game' : c.kind === 'folder' ? 'Picked folder' : `Update ${esc(c.version)}`;
  const used = chosen && chosen.path === c.path;
  // an incomplete folder that another one makes unnecessary (an empty base/ next to a full update) is not an error
  const unneeded = chosen && !used && !c.ok;
  const chip = used
    ? `<span class="chip ok">${icon('circle-check')}Will be used</span>`
    : c.ok ? '<span class="chip">Valid, older</span>'
    : unneeded ? '<span class="chip">Not needed</span>'
    : `<span class="chip danger">${icon('circle-x')}Incomplete</span>`;
  return `
    <div class="candidate ${used ? 'chosen' : ''}">
      <div class="row between">
        <h3>${title}</h3>
        ${chip}
      </div>
      <div class="path">${esc(c.path)}</div>
      ${unneeded ? '' : `<ul class="checks">
        ${c.checks.map((k) => `<li class="${k.ok ? 'ok' : 'bad'}">${icon(k.ok ? 'circle-check' : 'circle-x')}${esc(k.label)}</li>`).join('')}
      </ul>`}
    </div>`;
}

function preflightHtml(pre) {
  if (!pre) return '';
  const rows = pre.checks
    .filter((c) => c.level !== 'ok')
    .map((c) => `<li class="${c.level === 'fail' ? 'bad' : 'warn'}">${icon(c.level === 'fail' ? 'circle-x' : 'triangle-alert')}<div><strong>${esc(c.title)}</strong><br>${esc(c.detail)}</div></li>`)
    .join('');
  return `<div class="banner ${pre.blocked ? 'danger' : 'warn'}" id="preflight">
      <div class="stack">
        <div><strong>${pre.blocked ? 'Set up cannot start yet' : 'Check these before you start'}</strong></div>
        <ul class="preflight">${rows}</ul>
        <div class="row">
          ${pre.blocked ? '' : `<button class="btn primary" id="setup-anyway">${icon('wrench')}Set up anyway</button>`}
          <button class="btn" id="setup-recheck">${icon('circle-check')}Check again</button>
        </div>
      </div>
    </div>`;
}

function shadersHtml(s, saved, gentle, pre) {
  const p = s.progress;
  const running = p.state === 'running';
  const pct = p.total ? Math.floor((100 * p.done) / p.total) : 0;
  let body = '';
  if (running) {
    body = `<p class="dim">${esc(p.message)}</p>
      <progress max="100" value="${p.total ? pct : ''}"></progress>
      <p class="faint">${p.total ? `${pct}%. ` : ''}You can pause and continue later; finished work is kept.</p>`;
  } else if (p.state === 'error') {
    body = `<div class="banner danger">${icon('circle-x')}<div style="white-space: pre-wrap">${esc(p.message)}</div></div>`;
  } else if (s.complete) {
    body = `<p class="dim">${icon('circle-check')} Done. pkp.exe is built and the game's shaders are in <code>shader_cache/spirv</code> next to it.</p>`;
  } else if (s.rebuild && p.state !== 'cancelled') {
    body = `<p class="dim">This update changes the game code, so pkp.exe has to be built again from your game files (about 7 minutes). The old pkp.exe stays until the new one is ready. Your shaders and saves are kept.</p>`;
  } else {
    body = `<p class="dim">${p.state === 'cancelled' ? esc(p.message) : 'Checks your game files, builds pkp.exe from them on this PC (the download contains no game code; the compile takes a while and needs a few GB free while it runs), then translates the game\'s shaders from your romfs (about 100,000 shaders; needs about 36 GB free while it runs and keeps about 26 GB). You can pause and continue later.'}</p>`;
  }
  const label = s.complete || (s.rebuild && p.state !== 'cancelled') ? 'Set up again' : s.started ? 'Continue setup' : 'Set up';
  return `
    <div class="card">
      <h2>${icon('wrench')}Set up</h2>
      <div class="stack">
        ${body}
        ${running ? '' : preflightHtml(pre)}
        ${running ? '' : `<label class="row"><input type="checkbox" id="gentle" ${gentle ? 'checked' : ''} /> <span>Use fewer cores (cooler, the PC stays usable; setup takes longer)</span></label>`}
        <div class="row">
          ${running
            ? `<button class="btn" id="setup-cancel">${icon('circle-x')}Pause</button>`
            : `<button class="btn ${s.complete ? '' : 'primary'}" id="setup-run" ${saved ? '' : 'disabled'}>${icon('wrench')}${label}</button>`}
          ${s.complete && !running ? `<button class="btn primary" id="to-play">${icon('play')}Go to Play</button>` : ''}
          ${s.started || s.complete || s.progress.state === 'error' ? `<button class="btn quiet" id="setup-log">${icon('file-text')}Open setup log</button>` : ''}
        </div>
      </div>
    </div>`;
}

function dlssHtml(d, busy, p) {
  if (!d.available) return '';
  const pct = p && p.total ? Math.min(100, Math.floor((100 * p.done) / p.total)) : 0;
  return `
    <div class="card" id="dlss-card">
      <h2>${icon('monitor')}NVIDIA DLSS (optional)</h2>
      <div class="stack">
        <p class="dim">FSR 3 is included and needs nothing more. DLSS and DLAA need an NVIDIA RTX GPU and one more file, <code>nvngx_dlss.dll</code>, which NVIDIA licenses separately, so it is not in the download. This button gets it from NVIDIA's own DLSS repository on GitHub (version 310.9.16, checked against a fixed hash) and puts it in <code>${esc(d.dir)}</code>. By downloading it you accept the NVIDIA RTX SDKs licence: <code>licenses/LICENSE.NVIDIA-RTX-SDKs.txt</code>, also at github.com/NVIDIA/DLSS.</p>
        ${busy ? `<p class="dim">${esc(p?.message || 'Starting...')}</p><progress max="100" value="${pct}"></progress>` : ''}
        <div class="row">
          ${d.present ? `<span class="chip ok">${icon('circle-check')}nvngx_dlss.dll is installed</span>` : ''}
          <button class="btn ${d.present ? '' : 'primary'}" id="dlss-get" ${busy ? 'disabled' : ''}>${icon('monitor')}${d.present ? 'Download again' : 'Download DLSS from NVIDIA'}</button>
        </div>
      </div>
    </div>`;
}

function nupPct(p) {
  return p && p.total ? Math.min(100, Math.floor((100 * p.done) / p.total)) : 0;
}

function nupMsg(p) {
  return p && p.total ? `Unpacking the .nup: ${nupPct(p)}%${p.file ? ` (${p.file})` : ''}` : 'Reading the .nup...';
}

export async function render(el, ctx) {
  const cfg = await api('get_config');
  let path = cfg.dump_dir || '';
  let saved = path;
  let report = path ? await api('validate_dump', { path }) : null;
  let setup = await api('setup_status');
  let dlss = await api('dlss_status');
  let dlssBusy = false;
  let dlssProgress = null;
  let nupBusy = false;
  let nupProgress = null;
  let gentle = !!cfg.gentle_build;
  let pre = null;

  function draw() {
    const ok = report && report.chosen;
    el.innerHTML = `
      <div class="page">
        <div class="hero">
          <h1>${ctx.firstRun ? 'Welcome. Set up your game files.' : 'Game files'}</h1>
          <p class="dim">Pokopia Launcher does not include the game. You need your own legally dumped copy, already decrypted. The launcher only reads this folder and never writes to it (a .nup is unpacked into its own copy).</p>
        </div>

        <div class="card">
          <h2>${icon('folder-search')}Dump folder</h2>
          <div class="stack">
            <div class="row">
              <input type="text" id="path" spellcheck="false" placeholder="/home/you/Games/Pokopia_dump" value="${esc(path)}" />
              <button class="btn" id="browse" ${nupBusy ? 'disabled' : ''}>${icon('folder-open')}Browse</button>
              <button class="btn" id="browse-nup" ${nupBusy ? 'disabled' : ''}>${icon('file-archive')}.nup</button>
            </div>
            ${nupBusy ? `<p class="dim" id="nup-msg">${nupMsg(nupProgress)}</p><progress max="100" value="${nupPct(nupProgress)}"></progress><button class="btn" id="cancel-nup">Cancel unpacking</button>` : ''}
            <p class="faint">Or pick a <code>.nup</code> package instead of a folder: it is unpacked once into <code>nup_dump/</code> next to the game and used from there (needs the free space of the full game).</p>
            <p class="faint">Expected layout: <code>base/</code> and <code>updates/&lt;version&gt;/</code>, each with <code>exefs/</code> (main, sdk, subsdk0, rtld, main.npdm) and <code>romfs/</code>. The highest valid update is used, otherwise the base game.</p>
          </div>
        </div>

        ${report && report.errors.length ? `<div class="banner danger">${icon('circle-x')}<div>${report.errors.map(esc).join('<br>')}</div></div>` : ''}
        ${report && report.candidates.length ? `<div class="stack">${report.candidates.map((c) => candidateHtml(c, report.chosen)).join('')}</div>` : ''}

        <div class="row">
          <button class="btn ${ok && saved !== path ? 'primary' : ''}" id="use" ${ok && setup.progress.state !== 'running' ? '' : 'disabled'}>${icon('circle-check')}Use these game files</button>
          ${ctx.firstRun ? '' : '<button class="btn quiet" id="cancel">Back</button>'}
        </div>

        ${shadersHtml(setup, ok && saved === path, gentle, pre)}

        ${dlssHtml(dlss, dlssBusy, dlssProgress)}
      </div>`;
    refreshIcons();

    const input = el.querySelector('#path');
    const check = async () => {
      path = input.value.trim();
      report = path ? await api('validate_dump', { path }) : null;
      draw();
    };
    input.addEventListener('change', check);
    input.addEventListener('keydown', (e) => e.key === 'Enter' && check());
    el.querySelector('#browse').addEventListener('click', async () => {
      const picked = await api('pick_folder', { title: 'Choose your decrypted Pokopia dump folder', start: input.value || null });
      if (picked) {
        input.value = picked;
        check();
      }
    });
    el.querySelector('#browse-nup').addEventListener('click', async () => {
      const picked = await api('pick_nup', { start: null });
      if (!picked) return;
      nupBusy = true;
      nupProgress = null;
      draw();
      try {
        path = await api('import_nup', { path: picked });
        report = await api('validate_dump', { path });
      } catch (e) {
        toast(errText(e), 'error');
      }
      nupBusy = false;
      draw();
    });
    el.querySelector('#cancel-nup')?.addEventListener('click', () => api('cancel_nup'));
    el.querySelector('#use').addEventListener('click', async () => {
      try {
        await api('save_dump', { path });
        saved = path;
        toast('Game files saved. Next: Set up.', 'ok');
        draw();
      } catch (e) {
        toast(errText(e), 'error');
      }
    });
    el.querySelector('#cancel')?.addEventListener('click', () => ctx.go('play'));
    el.querySelector('#to-play')?.addEventListener('click', () => ctx.go('play'));
    const startSetup = async () => {
      try {
        pre = null;
        await api('start_setup', { gentle });
        setup = await api('setup_status');
        draw();
      } catch (e) {
        toast(errText(e), 'error');
      }
    };
    // pre-flight: all fine starts at once; warnings can be continued past, a failed check (disk space) cannot
    const runPreflight = async () => {
      try {
        pre = await api('preflight');
      } catch (e) {
        pre = null;
        toast(errText(e), 'error');
        return startSetup();
      }
      if (pre.blocked || pre.warnings) {
        if (pre.suggest_gentle) gentle = true;
        draw();
        return;
      }
      startSetup();
    };
    el.querySelector('#setup-run')?.addEventListener('click', runPreflight);
    el.querySelector('#setup-recheck')?.addEventListener('click', runPreflight);
    el.querySelector('#setup-anyway')?.addEventListener('click', startSetup);
    el.querySelector('#gentle')?.addEventListener('change', async (e) => {
      gentle = e.target.checked;
      try {
        await api('save_config', { config: { ...(await api('get_config')), gentle_build: gentle } });
      } catch (err) {
        toast(errText(err), 'error');
      }
    });
    el.querySelector('#setup-log')?.addEventListener('click', () => api('open_setup_log').catch((e) => toast(errText(e), 'error')));
    el.querySelector('#setup-cancel')?.addEventListener('click', () => api('cancel_setup'));
    el.querySelector('#dlss-get')?.addEventListener('click', async () => {
      dlssBusy = true;
      dlssProgress = null;
      draw();
      try {
        await api('fetch_dlss');
        toast('nvngx_dlss.dll downloaded and verified. Restart the game to use DLSS.', 'ok');
      } catch (e) {
        toast(errText(e), 'error');
      }
      dlssBusy = false;
      dlss = await api('dlss_status');
      draw();
    });
  }

  nupHandler = (p) => {
    nupProgress = p;
    if (!nupBusy) return;
    const msg = el.querySelector('#nup-msg');
    if (msg) {
      msg.textContent = nupMsg(p);
      const bar = msg.nextElementSibling;
      if (bar) bar.value = nupPct(p);
    }
  };

  dlssHandler = (p) => {
    dlssProgress = p;
    const card = el.querySelector('#dlss-card');
    if (card && dlssBusy) {
      card.outerHTML = dlssHtml(dlss, true, p);
      refreshIcons();
    }
  };

  ctx.onSetup(async (p) => {
    const was = setup.progress.state;
    setup = p.state === 'running' ? { ...setup, progress: p } : await api('setup_status');
    // progress ticks only touch the setup card's text, so the path field keeps its focus
    if (was === 'running' && p.state === 'running' && el.querySelector('progress')) {
      const card = el.querySelector('#setup-cancel')?.closest('.card');
      if (card) {
        card.outerHTML = shadersHtml(setup, true, gentle, null);
        refreshIcons();
        el.querySelector('#setup-cancel')?.addEventListener('click', () => api('cancel_setup'));
        return;
      }
    }
    draw();
  });
  draw();
}
