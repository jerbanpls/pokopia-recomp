import { api, esc, icon, refreshIcons, toast, errText } from '../util.js';

const hex = (c) => '0x' + (c >>> 0).toString(16).toUpperCase().padStart(8, '0');

function statusChip(s) {
  if (s.state === 'running') return `<span class="chip warn">${icon('loader-circle', 'spin')}Running</span>`;
  if (s.state === 'exited') {
    const code = s.code === null || s.code === undefined ? 'no exit code' : `code ${s.code} (${hex(s.code)})`;
    return s.crashed
      ? `<span class="chip danger">${icon('circle-alert')}Stopped abnormally, ${esc(code)}</span>`
      : `<span class="chip ok">${icon('circle-check')}Exited, ${esc(code)}</span>`;
  }
  return `<span class="chip">Ready</span>`;
}

function reportHtml(s) {
  if (s.state !== 'exited') return '';
  const files = s.files.filter((f) => s.crashed || f.name === 'pkp.log');
  return `
    <div class="card">
      <h2>${icon(s.crashed ? 'circle-alert' : 'file-text')}${s.crashed ? 'The game did not end cleanly' : 'Last run'}</h2>
      <p class="dim">${esc(s.message)}</p>
      ${files.length ? `<div class="row wrap" style="margin-top: var(--space-4)">${files
        .map((f) => `<button class="btn" data-open="${esc(f.name)}">${icon('file-text')}${esc(f.name)}${f.fresh && f.name !== 'pkp.log' ? ' (new)' : ''}</button>`)
        .join('')}</div>` : ''}
    </div>`;
}

export async function render(el, ctx) {
  const [cfg, ov, setup] = await Promise.all([api('get_config'), api('get_overview'), api('setup_status')]);
  const report = cfg.dump_dir ? await api('validate_dump', { path: cfg.dump_dir }) : null;
  const chosen = report && report.chosen;
  let status = await api('game_status');

  function draw() {
    const running = status.state === 'running';
    const problems = [];
    if (!cfg.dump_dir) problems.push('No game files chosen yet.');
    else if (!chosen) problems.push(`The game files at ${cfg.dump_dir} are not usable: ${report.errors.join(' ')}`);
    // a release that builds pkp.exe itself has none until Set up ran: that is not a misplaced launcher
    if (!ov.exe_exists && !ov.builds_exe) problems.push(`pkp.exe was not found in ${ov.game_dir}. Put the launcher next to pkp.exe, or set the game folder in Settings.`);
    if (chosen && (ov.exe_exists || ov.builds_exe) && !setup.complete) {
      problems.push(setup.progress.state === 'running' ? 'Setup is still running (Game files).'
        : setup.rebuild ? 'This update changes the game code: open Game files and press Set up again to rebuild pkp.exe (about 7 minutes). Your shaders and saves are kept.'
        : ov.exe_exists ? 'Setup is not finished yet: open Game files and press Set up.'
        : 'Set up has not run yet: open Game files and press Set up. It builds the game (pkp.exe) from your game files.');
    }
    const ready = problems.length === 0;

    el.innerHTML = `
      <div class="page">
        <div class="hero">
          <h1>Pokopia</h1>
          <p class="dim">Native PC port. Your game files stay where they are.</p>
        </div>

        ${problems.map((p) => `<div class="banner danger">${icon('circle-x')}<div>${esc(p)}</div></div>`).join('')}
        ${problems.length && (!chosen || !setup.complete) ? `<div><button class="btn primary" id="to-setup">${icon(chosen ? 'wrench' : 'folder-open')}${chosen ? 'Set up' : 'Choose game files'}</button></div>` : ''}

        <div class="row wrap">
          <button class="btn primary big" id="play" ${ready && !running ? '' : 'disabled'}>${icon('play')}${running ? 'Running' : 'Play'}</button>
          ${statusChip(status)}
        </div>

        ${reportHtml(status)}

        <div class="card">
          <h2>${icon('hard-drive')}Setup</h2>
          <dl class="kv">
            <dt>Game files</dt>
            <dd>${chosen ? `${chosen.kind === 'base' ? 'Base game' : 'Update ' + esc(chosen.version)}<div class="path">${esc(chosen.path)}</div>` : '<span class="faint">not set</span>'}</dd>
            <dt>Shaders</dt>
            <dd>${setup.complete ? 'ready' : '<span class="faint">not set up</span>'}</dd>
            <dt>Game folder</dt>
            <dd>${ov.exe_exists ? 'pkp.exe found' : `<span class="faint">${ov.builds_exe ? 'pkp.exe not built yet (Set up builds it)' : 'pkp.exe missing'}</span>`}<div class="path">${esc(ov.game_dir)}</div></dd>
          </dl>
          <div class="row" style="margin-top: var(--space-4)">
            <button class="btn" id="open-dir">${icon('folder-open')}Open game folder</button>
            <button class="btn quiet" id="change">Change game files</button>
          </div>
        </div>
      </div>`;
    refreshIcons();

    el.querySelector('#play')?.addEventListener('click', async () => {
      try {
        await api('play');
      } catch (e) {
        toast(errText(e), 'error');
      }
    });
    el.querySelector('#to-setup')?.addEventListener('click', () => ctx.go('setup'));
    el.querySelector('#change')?.addEventListener('click', () => ctx.go('setup'));
    el.querySelector('#open-dir')?.addEventListener('click', () => api('open_game_dir').catch((e) => toast(errText(e), 'error')));
    el.querySelectorAll('[data-open]').forEach((b) =>
      b.addEventListener('click', () => api('open_game_file', { name: b.dataset.open }).catch((e) => toast(errText(e), 'error'))),
    );
  }

  ctx.onStatus((s) => {
    status = s;
    draw();
  });
  draw();
}
