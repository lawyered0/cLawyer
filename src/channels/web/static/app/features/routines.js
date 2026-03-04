// --- Routines ---

let currentRoutineId = null;

delegate(byId('routines-tbody'), 'click', 'button[data-routine-action]', function(event, button) {
  event.preventDefault();
  const action = button.getAttribute('data-routine-action');
  const id = button.getAttribute('data-routine-id');
  if (!id) return;
  if (action === 'toggle') {
    toggleRoutine(id);
    return;
  }
  if (action === 'run') {
    triggerRoutine(id);
    return;
  }
  if (action === 'delete') {
    deleteRoutine(id, button.getAttribute('data-routine-name') || id);
  }
});

delegate(byId('routines-tbody'), 'click', 'tr.routine-row[data-routine-id]', function(event, row) {
  if (event.target.closest('button[data-routine-action]')) return;
  const id = row.getAttribute('data-routine-id');
  if (id) openRoutineDetail(id);
});

delegate(byId('routine-detail'), 'click', 'button[data-routine-detail-action],a[data-routine-job-id]', function(event, target) {
  event.preventDefault();
  if (target.matches('button[data-routine-detail-action="back"]')) {
    closeRoutineDetail();
    return;
  }
  if (target.matches('a[data-routine-job-id]')) {
    const jobId = target.getAttribute('data-routine-job-id');
    if (!jobId) return;
    switchTab('jobs');
    openJobDetail(jobId);
  }
});

function loadRoutines() {
  const requestVersion = beginRequest('routinesList');
  beginRequest('routineDetail');
  currentRoutineId = null;

  // Restore list view if detail was open
  const detail = document.getElementById('routine-detail');
  if (detail) detail.style.display = 'none';
  const table = document.getElementById('routines-table');
  if (table) table.style.display = '';

  Promise.all([
    apiFetch('/api/routines/summary'),
    apiFetch('/api/routines'),
  ]).then(([summary, listData]) => {
    if (!isCurrentRequest('routinesList', requestVersion)) return;
    renderRoutinesSummary(summary);
    renderRoutinesList(listData.routines);
  }).catch(() => {
    if (!isCurrentRequest('routinesList', requestVersion)) return;
  });
}

function renderRoutinesSummary(s) {
  document.getElementById('routines-summary').innerHTML = ''
    + summaryCard('Total', s.total, '')
    + summaryCard('Enabled', s.enabled, 'active')
    + summaryCard('Disabled', s.disabled, '')
    + summaryCard('Failing', s.failing, 'failed')
    + summaryCard('Runs Today', s.runs_today, 'completed');
}

function renderRoutinesList(routines) {
  const tbody = document.getElementById('routines-tbody');
  const empty = document.getElementById('routines-empty');

  if (!routines || routines.length === 0) {
    tbody.innerHTML = '';
    empty.style.display = 'block';
    return;
  }

  empty.style.display = 'none';
  tbody.innerHTML = routines.map((r) => {
    const statusClass = r.status === 'active' ? 'completed'
      : r.status === 'failing' ? 'failed'
      : 'pending';

    const toggleLabel = r.enabled ? 'Disable' : 'Enable';
    const toggleClass = r.enabled ? 'btn-cancel' : 'btn-restart';
    const escapedId = escapeHtml(r.id);
    const escapedName = escapeHtml(r.name);

    return '<tr class="routine-row" data-routine-id="' + escapedId + '">'
      + '<td>' + escapedName + '</td>'
      + '<td>' + escapeHtml(r.trigger_summary) + '</td>'
      + '<td>' + escapeHtml(r.action_type) + '</td>'
      + '<td>' + formatRelativeTime(r.last_run_at) + '</td>'
      + '<td>' + formatRelativeTime(r.next_fire_at) + '</td>'
      + '<td>' + r.run_count + '</td>'
      + '<td><span class="badge ' + statusClass + '">' + escapeHtml(r.status) + '</span></td>'
      + '<td>'
      + '<button class="' + toggleClass + '" type="button" data-routine-action="toggle" data-routine-id="' + escapedId + '">' + toggleLabel + '</button> '
      + '<button class="btn-restart" type="button" data-routine-action="run" data-routine-id="' + escapedId + '">Run</button> '
      + '<button class="btn-cancel" type="button" data-routine-action="delete" data-routine-id="' + escapedId + '" data-routine-name="' + escapedName + '">Delete</button>'
      + '</td>'
      + '</tr>';
  }).join('');
}

function openRoutineDetail(id) {
  const requestVersion = beginRequest('routineDetail');
  currentRoutineId = id;
  apiFetch('/api/routines/' + id).then((routine) => {
    if (!isCurrentRequest('routineDetail', requestVersion)) return;
    renderRoutineDetail(routine);
  }).catch((err) => {
    if (!isCurrentRequest('routineDetail', requestVersion)) return;
    showToast('Failed to load routine: ' + err.message, 'error');
  });
}

function closeRoutineDetail() {
  currentRoutineId = null;
  loadRoutines();
}

function renderRoutineDetail(routine) {
  const table = document.getElementById('routines-table');
  if (table) table.style.display = 'none';
  document.getElementById('routines-empty').style.display = 'none';

  const detail = document.getElementById('routine-detail');
  detail.style.display = 'block';

  const statusClass = !routine.enabled ? 'pending'
    : routine.consecutive_failures > 0 ? 'failed'
    : 'completed';
  const statusLabel = !routine.enabled ? 'disabled'
    : routine.consecutive_failures > 0 ? 'failing'
    : 'active';

  let html = '<div class="job-detail-header">'
    + '<button class="btn-back" type="button" data-routine-detail-action="back">&larr; Back</button>'
    + '<h2>' + escapeHtml(routine.name) + '</h2>'
    + '<span class="badge ' + statusClass + '">' + escapeHtml(statusLabel) + '</span>'
    + '</div>';

  // Metadata grid
  html += '<div class="job-meta-grid">'
    + metaItem('Routine ID', routine.id)
    + metaItem('Enabled', routine.enabled ? 'Yes' : 'No')
    + metaItem('Run Count', routine.run_count)
    + metaItem('Failures', routine.consecutive_failures)
    + metaItem('Last Run', formatDate(routine.last_run_at))
    + metaItem('Next Fire', formatDate(routine.next_fire_at))
    + metaItem('Created', formatDate(routine.created_at))
    + '</div>';

  // Description
  if (routine.description) {
    html += '<div class="job-description"><h3>Description</h3>'
      + '<div class="job-description-body">' + escapeHtml(routine.description) + '</div></div>';
  }

  // Trigger config
  html += '<div class="job-description"><h3>Trigger</h3>'
    + '<pre class="action-json">' + escapeHtml(JSON.stringify(routine.trigger, null, 2)) + '</pre></div>';

  // Action config
  html += '<div class="job-description"><h3>Action</h3>'
    + '<pre class="action-json">' + escapeHtml(JSON.stringify(routine.action, null, 2)) + '</pre></div>';

  // Recent runs
  if (routine.recent_runs && routine.recent_runs.length > 0) {
    html += '<div class="job-timeline-section"><h3>Recent Runs</h3>'
      + '<table class="routines-table"><thead><tr>'
      + '<th>Trigger</th><th>Started</th><th>Completed</th><th>Status</th><th>Summary</th><th>Tokens</th>'
      + '</tr></thead><tbody>';
    for (const run of routine.recent_runs) {
      const runStatusClass = run.status === 'Ok' ? 'completed'
        : run.status === 'Failed' ? 'failed'
        : run.status === 'Attention' ? 'stuck'
        : 'in_progress';
      html += '<tr>'
        + '<td>' + escapeHtml(run.trigger_type) + '</td>'
        + '<td>' + formatDate(run.started_at) + '</td>'
        + '<td>' + formatDate(run.completed_at) + '</td>'
        + '<td><span class="badge ' + runStatusClass + '">' + escapeHtml(run.status) + '</span></td>'
        + '<td>' + escapeHtml(run.result_summary || '-')
          + (run.job_id ? ' <a href="#" data-routine-job-id="' + escapeHtml(run.job_id) + '">[view job]</a>' : '')
          + '</td>'
        + '<td>' + (run.tokens_used != null ? run.tokens_used : '-') + '</td>'
        + '</tr>';
    }
    html += '</tbody></table></div>';
  }

  detail.innerHTML = html;
}

function triggerRoutine(id) {
  apiFetch('/api/routines/' + id + '/trigger', { method: 'POST' })
    .then(() => showToast('Routine triggered', 'success'))
    .catch((err) => showToast('Trigger failed: ' + err.message, 'error'));
}

function toggleRoutine(id) {
  apiFetch('/api/routines/' + id + '/toggle', { method: 'POST' })
    .then((res) => {
      showToast('Routine ' + (res.status || 'toggled'), 'success');
      if (currentRoutineId) openRoutineDetail(currentRoutineId);
      else loadRoutines();
    })
    .catch((err) => showToast('Toggle failed: ' + err.message, 'error'));
}

function deleteRoutine(id, name) {
  if (!confirm('Delete routine "' + name + '"?')) return;
  apiFetch('/api/routines/' + id, { method: 'DELETE' })
    .then(() => {
      showToast('Routine deleted', 'success');
      if (currentRoutineId === id) closeRoutineDetail();
      else loadRoutines();
    })
    .catch((err) => showToast('Delete failed: ' + err.message, 'error'));
}

function formatRelativeTime(isoString) {
  if (!isoString) return '-';
  const d = new Date(isoString);
  const now = Date.now();
  const diffMs = now - d.getTime();
  const absDiff = Math.abs(diffMs);
  const future = diffMs < 0;

  if (absDiff < 60000) return future ? 'in <1m' : '<1m ago';
  if (absDiff < 3600000) {
    const m = Math.floor(absDiff / 60000);
    return future ? 'in ' + m + 'm' : m + 'm ago';
  }
  if (absDiff < 86400000) {
    const h = Math.floor(absDiff / 3600000);
    return future ? 'in ' + h + 'h' : h + 'h ago';
  }
  const days = Math.floor(absDiff / 86400000);
  return future ? 'in ' + days + 'd' : days + 'd ago';
}

