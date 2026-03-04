// --- Logs ---

const LOG_MAX_ENTRIES = 2000;
let logsPaused = false;
let logBuffer = []; // buffer while paused

function connectLogSSE() {
  if (logEventSource) logEventSource.close();

  logEventSource = new EventSource('/api/logs/events?token=' + encodeURIComponent(token));

  logEventSource.addEventListener('log', (e) => {
    const entry = JSON.parse(e.data);
    if (logsPaused) {
      logBuffer.push(entry);
      return;
    }
    prependLogEntry(entry);
  });

  logEventSource.onerror = () => {
    // Silent reconnect
  };
}

function prependLogEntry(entry) {
  const output = document.getElementById('logs-output');

  // Level filter
  const levelFilter = document.getElementById('logs-level-filter').value;
  const targetFilter = document.getElementById('logs-target-filter').value.trim().toLowerCase();

  const div = document.createElement('div');
  div.className = 'log-entry level-' + entry.level;
  div.setAttribute('data-level', entry.level);
  div.setAttribute('data-target', entry.target);

  const ts = document.createElement('span');
  ts.className = 'log-ts';
  ts.textContent = entry.timestamp.substring(11, 23);
  div.appendChild(ts);

  const lvl = document.createElement('span');
  lvl.className = 'log-level';
  lvl.textContent = entry.level.padEnd(5);
  div.appendChild(lvl);

  const tgt = document.createElement('span');
  tgt.className = 'log-target';
  tgt.textContent = entry.target;
  div.appendChild(tgt);

  const msg = document.createElement('span');
  msg.className = 'log-msg';
  msg.textContent = entry.message;
  div.appendChild(msg);

  div.addEventListener('click', () => div.classList.toggle('expanded'));

  // Apply current filters as visibility
  const matchesLevel = levelFilter === 'all' || entry.level === levelFilter;
  const matchesTarget = !targetFilter || entry.target.toLowerCase().includes(targetFilter);
  if (!matchesLevel || !matchesTarget) {
    div.style.display = 'none';
  }

  output.prepend(div);

  // Cap entries (remove oldest at the bottom)
  while (output.children.length > LOG_MAX_ENTRIES) {
    output.removeChild(output.lastChild);
  }

  // Auto-scroll to top (newest entries are at the top)
  if (document.getElementById('logs-autoscroll').checked) {
    output.scrollTop = 0;
  }
}

function toggleLogsPause() {
  logsPaused = !logsPaused;
  const btn = document.getElementById('logs-pause-btn');
  btn.textContent = logsPaused ? 'Resume' : 'Pause';

  if (!logsPaused) {
    // Flush buffer: oldest-first + prepend naturally puts newest at top
    for (const entry of logBuffer) {
      prependLogEntry(entry);
    }
    logBuffer = [];
  }
}

function clearLogs() {
  if (!confirm('Clear all logs?')) return;
  document.getElementById('logs-output').innerHTML = '';
  logBuffer = [];
}

// Re-apply filters when level or target changes
document.getElementById('logs-level-filter').addEventListener('change', applyLogFilters);
document.getElementById('logs-target-filter').addEventListener('input', applyLogFilters);

function applyLogFilters() {
  const levelFilter = document.getElementById('logs-level-filter').value;
  const targetFilter = document.getElementById('logs-target-filter').value.trim().toLowerCase();
  const entries = document.querySelectorAll('#logs-output .log-entry');
  for (const el of entries) {
    const matchesLevel = levelFilter === 'all' || el.getAttribute('data-level') === levelFilter;
    const matchesTarget = !targetFilter || el.getAttribute('data-target').toLowerCase().includes(targetFilter);
    el.style.display = (matchesLevel && matchesTarget) ? '' : 'none';
  }
}

// --- Server-side log level control ---

function setServerLogLevel(level) {
  apiFetch('/api/logs/level', {
    method: 'PUT',
    body: { level: level },
  })
    .then(data => {
      document.getElementById('logs-server-level').value = data.level;
    })
    .catch(err => console.error('Failed to set server log level:', err));
}

function loadServerLogLevel() {
  apiFetch('/api/logs/level')
    .then(data => {
      document.getElementById('logs-server-level').value = data.level;
    })
    .catch(() => {}); // ignore if not available
}

let legalAuditOffset = 0;
let legalAuditNextOffset = null;
let legalAuditLimit = 50;
let legalAuditTotal = 0;
let legalAuditEvents = [];

function toRfc3339FromLocalInput(raw) {
  if (!raw) return null;
  var dt = new Date(raw);
  if (isNaN(dt.getTime())) return null;
  return dt.toISOString();
}

function buildLegalAuditQuery(offset) {
  var params = new URLSearchParams();
  var eventType = byId('legal-audit-event-type') ? byId('legal-audit-event-type').value.trim() : '';
  var fromRaw = byId('legal-audit-from') ? byId('legal-audit-from').value : '';
  var toRaw = byId('legal-audit-to') ? byId('legal-audit-to').value : '';
  var limitRaw = byId('legal-audit-limit') ? parseInt(byId('legal-audit-limit').value, 10) : 50;
  legalAuditLimit = (!isNaN(limitRaw) && limitRaw > 0) ? Math.min(200, limitRaw) : 50;

  params.set('offset', String(Math.max(0, offset || 0)));
  params.set('limit', String(legalAuditLimit));
  if (eventType) params.set('event_type', eventType);
  var fromIso = toRfc3339FromLocalInput(fromRaw);
  var toIso = toRfc3339FromLocalInput(toRaw);
  if (fromIso) params.set('from', fromIso);
  if (toIso) params.set('to', toIso);
  return params.toString();
}

function loadLegalAudit(offset) {
  var requestVersion = beginRequest('legalAudit');
  legalAuditOffset = Math.max(0, offset || 0);
  var list = byId('legal-audit-list');
  if (list) list.innerHTML = '<div class="empty-state">Loading legal audit events…</div>';
  var query = buildLegalAuditQuery(legalAuditOffset);
  apiFetch('/api/legal/audit?' + query).then(function(data) {
    if (!isCurrentRequest('legalAudit', requestVersion)) return;
    legalAuditEvents = (data && data.events) ? data.events : [];
    legalAuditNextOffset = (data && data.next_offset != null) ? data.next_offset : null;
    legalAuditTotal = data && typeof data.total === 'number' ? data.total : 0;
    renderLegalAuditList(data || {});
  }).catch(function(err) {
    if (!isCurrentRequest('legalAudit', requestVersion)) return;
    legalAuditEvents = [];
    legalAuditNextOffset = null;
    legalAuditTotal = 0;
    if (list) {
      list.innerHTML = '<div class="empty-state error-state">Failed to load legal audit: ' + escapeHtml(err.message) + '</div>';
    }
    var pageMeta = byId('legal-audit-page-meta');
    if (pageMeta) pageMeta.textContent = '';
    var detail = byId('legal-audit-detail');
    if (detail) detail.textContent = '';
  });
}

function renderLegalAuditList(data) {
  var list = byId('legal-audit-list');
  var pageMeta = byId('legal-audit-page-meta');
  var prevBtn = byId('legal-audit-prev-btn');
  var nextBtn = byId('legal-audit-next-btn');

  if (prevBtn) prevBtn.disabled = legalAuditOffset <= 0;
  if (nextBtn) nextBtn.disabled = legalAuditNextOffset == null;

  var parseErrors = data && data.parse_errors ? data.parse_errors : 0;
  var truncated = !!(data && data.truncated);
  if (pageMeta) {
    var start = legalAuditTotal === 0 ? 0 : legalAuditOffset + 1;
    var end = legalAuditOffset + legalAuditEvents.length;
    var meta = 'Showing ' + start + '-' + end + ' of ' + legalAuditTotal;
    if (parseErrors > 0) meta += ' · parse errors: ' + parseErrors;
    if (truncated) meta += ' · truncated';
    pageMeta.textContent = meta;
  }

  if (!list) return;
  if (!legalAuditEvents.length) {
    list.innerHTML = '<div class="empty-state">No matching legal audit events.</div>';
    var detail = byId('legal-audit-detail');
    if (detail) detail.textContent = '';
    return;
  }

  var html = '';
  for (var i = 0; i < legalAuditEvents.length; i++) {
    var event = legalAuditEvents[i];
    html += '<div class="legal-audit-row">';
    html += '<span class="legal-audit-ts">' + escapeHtml(formatDate(event.ts)) + '</span>';
    html += '<span class="legal-audit-type">' + escapeHtml(event.event_type || '') + '</span>';
    html += '<button data-audit-index="' + i + '">View</button>';
    html += '</div>';
  }
  list.innerHTML = html;
}

function renderLegalAuditDetail(event) {
  var detail = byId('legal-audit-detail');
  if (!detail) return;
  detail.textContent = JSON.stringify(event, null, 2);
}

