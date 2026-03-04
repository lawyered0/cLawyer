// --- Matters ---

/** In-memory cache: array of MatterInfo from the last loadMatters() call. */
var mattersCache = [];
/** Currently active matter ID (string) or null. */
var activeMatterId = null;
/** Currently selected matter in the detail panel. */
var selectedMatterId = null;
/** Last loaded matter document index entries. */
var currentMatterDocuments = [];
/** Last loaded matter templates. */
var currentMatterTemplates = [];
/** Last loaded dashboard summary for selected matter. */
var currentMatterDashboard = null;
/** Last loaded structured deadlines for selected matter. */
var currentMatterDeadlines = [];
/** Last loaded conversation threads bound to selected matter. */
var currentMatterThreads = [];
/** Currently selected subsection in the matter detail panel. */
var currentMatterDetailSection = 'overview';
/** Last loaded work data for selected matter. */
var currentMatterWork = {
  loaded: false,
  loading: false,
  error: '',
  tasks: [],
  notes: [],
  timeEntries: [],
  expenseEntries: [],
};
/** Last loaded finance data for selected matter. */
var currentMatterFinance = {
  loaded: false,
  loading: false,
  error: '',
  timeSummary: null,
  trustLedger: null,
  invoices: [],
};
/** Persisted key for grouped matters rendering preference. */
var MATTERS_GROUP_KEY = 'clawyer_matters_group_by_client';
/** Grouping preference in Matters tab. */
var mattersGroupByClient = (function() {
  try {
    return localStorage.getItem(MATTERS_GROUP_KEY) === '1';
  } catch (_) {
    return false;
  }
})();
/** Conflict-review state for create-matter intake flow. */
var matterCreateReviewState = {
  status: 'unreviewed',
  signature: null,
  matched: false,
  hits: [],
  checkedParties: [],
};
/** Busy flags for create/review actions. */
var matterCreateBusy = false;
var matterCreateReviewBusy = false;
/** Last focused element before opening the detail quick-action modal. */
var matterActionModalLastFocus = null;
/** Active quick-action modal type (task|note|time|expense|deposit). */
var matterActionModalType = null;
/** Busy state for matter quick-action modal submit. */
var matterActionModalBusy = false;

function createEmptyMatterWorkState() {
  return {
    loaded: false,
    loading: false,
    error: '',
    tasks: [],
    notes: [],
    timeEntries: [],
    expenseEntries: [],
  };
}

function createEmptyMatterFinanceState() {
  return {
    loaded: false,
    loading: false,
    error: '',
    timeSummary: null,
    trustLedger: null,
    invoices: [],
  };
}

function todayIsoDate() {
  var now = new Date();
  var month = String(now.getMonth() + 1).padStart(2, '0');
  var day = String(now.getDate()).padStart(2, '0');
  return now.getFullYear() + '-' + month + '-' + day;
}

function parseCsvList(raw) {
  if (!raw) return [];
  return raw
    .split(',')
    .map(function(v) { return v.trim(); })
    .filter(function(v) { return !!v; });
}

function getMatterSelectValue(selectId, otherInputId) {
  var select = byId(selectId);
  if (!select) return '';
  var value = (select.value || '').trim();
  if (value !== '__other__') return value;
  var other = byId(otherInputId);
  return other ? other.value.trim() : '';
}

function syncMatterSelectOtherFields() {
  var confidentiality = byId('matter-create-confidentiality');
  var confidentialityOther = byId('matter-create-confidentiality-other');
  var retention = byId('matter-create-retention');
  var retentionOther = byId('matter-create-retention-other');

  if (confidentiality && confidentialityOther) {
    var showConfOther = confidentiality.value === '__other__';
    confidentialityOther.classList.toggle('is-hidden', !showConfOther);
    confidentialityOther.required = showConfOther;
  }

  if (retention && retentionOther) {
    var showRetentionOther = retention.value === '__other__';
    retentionOther.classList.toggle('is-hidden', !showRetentionOther);
    retentionOther.required = showRetentionOther;
  }
}

function readMatterCreateFormValues() {
  var openedDate = byId('matter-create-opened-date')
    ? byId('matter-create-opened-date').value.trim()
    : (byId('matter-create-opened-at') ? byId('matter-create-opened-at').value.trim() : '');
  return {
    matter_id: byId('matter-create-id') ? byId('matter-create-id').value.trim() : '',
    client: byId('matter-create-client') ? byId('matter-create-client').value.trim() : '',
    confidentiality: getMatterSelectValue('matter-create-confidentiality', 'matter-create-confidentiality-other'),
    retention: getMatterSelectValue('matter-create-retention', 'matter-create-retention-other'),
    jurisdiction: byId('matter-create-jurisdiction') ? byId('matter-create-jurisdiction').value.trim() : '',
    practice_area: byId('matter-create-practice-area') ? byId('matter-create-practice-area').value.trim() : '',
    opened_date: openedDate,
    team: parseCsvList(byId('matter-create-team') ? byId('matter-create-team').value : ''),
    adversaries: parseCsvList(byId('matter-create-adversaries') ? byId('matter-create-adversaries').value : ''),
  };
}

function validateMatterCreateForm(formData) {
  if (!formData.matter_id || !formData.client || !formData.confidentiality || !formData.retention) {
    return 'Matter ID, client, confidentiality, and retention are required.';
  }
  if (formData.opened_date && !/^\d{4}-\d{2}-\d{2}$/.test(formData.opened_date)) {
    return 'Opened date must use YYYY-MM-DD.';
  }
  return null;
}

function matterCreateFormSignature(formData) {
  return JSON.stringify({
    matter_id: formData.matter_id,
    client: formData.client,
    confidentiality: formData.confidentiality,
    retention: formData.retention,
    jurisdiction: formData.jurisdiction,
    practice_area: formData.practice_area,
    opened_date: formData.opened_date,
    team: formData.team,
    adversaries: formData.adversaries,
  });
}

function openMatterCreateModal() {
  var modal = byId('matter-create-modal');
  if (!modal) return;
  matterCreateModalLastFocus = document.activeElement instanceof HTMLElement
    ? document.activeElement
    : null;
  var form = byId('matters-create-form');
  if (form) form.reset();
  resetMatterCreateReview('Run conflict review before creating this matter.', null);
  modal.classList.remove('is-hidden');
  syncMatterSelectOtherFields();
  syncMatterCreateActionState();
  var focusTarget = byId('matter-create-id');
  if (focusTarget && typeof focusTarget.focus === 'function') {
    requestAnimationFrame(function() {
      focusTarget.focus();
    });
  }
}

function closeMatterCreateModal() {
  var modal = byId('matter-create-modal');
  if (!modal) return;
  if (matterCreateBusy || matterCreateReviewBusy) return;
  modal.classList.add('is-hidden');
  var returnFocus = matterCreateModalLastFocus;
  if (!returnFocus || !document.contains(returnFocus)) {
    returnFocus = byId('matters-new-btn');
  }
  if (returnFocus && typeof returnFocus.focus === 'function') {
    returnFocus.focus();
  }
  matterCreateModalLastFocus = null;
}

function isMatterCreateModalOpen() {
  var modal = byId('matter-create-modal');
  return !!(modal && !modal.classList.contains('is-hidden'));
}

function getFocusableElements(container) {
  if (!container) return [];
  var focusableSelector = [
    'a[href]',
    'button:not([disabled])',
    'input:not([disabled])',
    'select:not([disabled])',
    'textarea:not([disabled])',
    '[tabindex]:not([tabindex="-1"])',
  ].join(', ');
  return Array.prototype.slice.call(container.querySelectorAll(focusableSelector))
    .filter(function(el) {
      return !!(el.offsetWidth || el.offsetHeight || el.getClientRects().length);
    });
}

function trapMatterCreateModalFocus(event) {
  var modal = byId('matter-create-modal');
  if (!modal || modal.classList.contains('is-hidden')) return;
  var dialog = modal.querySelector('.configure-modal');
  var focusables = getFocusableElements(dialog);
  if (!focusables.length) {
    event.preventDefault();
    if (dialog && typeof dialog.focus === 'function') dialog.focus();
    return;
  }
  var first = focusables[0];
  var last = focusables[focusables.length - 1];
  var active = document.activeElement;
  if (event.shiftKey) {
    if (active === first || !dialog.contains(active)) {
      event.preventDefault();
      last.focus();
    }
    return;
  }
  if (active === last) {
    event.preventDefault();
    first.focus();
  }
}

function parseErrorPayload(err) {
  if (!err || !err.message) return null;
  try {
    return JSON.parse(err.message);
  } catch (_) {
    return null;
  }
}

function decisionNeedsNote(decision) {
  return decision === 'waived' || decision === 'declined';
}

function setMatterCreateReviewStatus(message, tone) {
  var status = byId('matters-create-review-status');
  if (!status) return;
  status.textContent = message;
  status.classList.remove('state-success', 'state-warning', 'state-error');
  if (tone === 'success') status.classList.add('state-success');
  if (tone === 'warning') status.classList.add('state-warning');
  if (tone === 'error') status.classList.add('state-error');
}

function getMatterCreateDecision() {
  var select = byId('matter-create-conflict-decision');
  return select ? select.value : 'clear';
}

function getMatterCreateDecisionNote() {
  var note = byId('matter-create-conflict-note');
  return note ? note.value.trim() : '';
}

function canCreateMatterFromReview(formData) {
  if (matterCreateReviewState.status !== 'reviewed') return false;
  if (matterCreateReviewState.signature !== matterCreateFormSignature(formData)) return false;
  if (!matterCreateReviewState.matched) return true;
  var decision = getMatterCreateDecision();
  if (!decision) return false;
  if (decisionNeedsNote(decision) && !getMatterCreateDecisionNote()) return false;
  return true;
}

function renderMatterCreateHits(hits) {
  var container = byId('matters-create-review-hits');
  if (!container) return;
  if (!hits || !hits.length) {
    container.innerHTML = '';
    return;
  }

  var html = '<div class="matter-review-table-wrap"><table class="matter-review-table"><thead><tr>'
    + '<th>Party</th><th>Role</th><th>Matter</th><th>Status</th><th>Matched Via</th>'
    + '</tr></thead><tbody>';
  for (var i = 0; i < hits.length; i++) {
    var hit = hits[i];
    html += '<tr>';
    html += '<td>' + escapeHtml(hit.party || '') + '</td>';
    html += '<td>' + escapeHtml(hit.role || '') + '</td>';
    html += '<td>' + escapeHtml(hit.matter_id || '') + '</td>';
    html += '<td>' + escapeHtml(hit.matter_status || '') + '</td>';
    html += '<td>' + escapeHtml(hit.matched_via || '') + '</td>';
    html += '</tr>';
  }
  html += '</tbody></table></div>';
  container.innerHTML = html;
}

function syncMatterCreateActionState() {
  var formData = readMatterCreateFormValues();
  var reviewBtn = byId('matters-review-btn');
  var createBtn = byId('matters-create-btn');
  var controls = byId('matters-create-clearance-controls');
  var noteInput = byId('matter-create-conflict-note');
  var decision = getMatterCreateDecision();

  if (reviewBtn) {
    reviewBtn.disabled = matterCreateBusy || matterCreateReviewBusy;
    reviewBtn.textContent = matterCreateReviewBusy ? 'Reviewing…' : 'Review Conflicts';
  }

  if (controls) {
    var showControls = (matterCreateReviewState.status === 'reviewed' && matterCreateReviewState.matched);
    controls.classList.toggle('is-hidden', !showControls);
    controls.style.display = showControls ? 'grid' : 'none';
  }
  if (noteInput) {
    noteInput.required = matterCreateReviewState.matched && decisionNeedsNote(decision);
  }

  if (createBtn) {
    createBtn.disabled = matterCreateBusy
      || matterCreateReviewBusy
      || !canCreateMatterFromReview(formData);
    createBtn.textContent = matterCreateBusy ? 'Creating…' : 'Create & Activate';
  }
}

function resetMatterCreateReview(message, tone) {
  matterCreateReviewState.status = 'unreviewed';
  matterCreateReviewState.signature = null;
  matterCreateReviewState.matched = false;
  matterCreateReviewState.hits = [];
  matterCreateReviewState.checkedParties = [];
  renderMatterCreateHits([]);

  var decision = byId('matter-create-conflict-decision');
  if (decision) decision.value = 'clear';
  var note = byId('matter-create-conflict-note');
  if (note) note.value = '';

  setMatterCreateReviewStatus(message || 'Run conflict review before creating this matter.', tone || null);
  syncMatterCreateActionState();
}

function applyMatterCreateReviewResult(payload, signature) {
  var hits = (payload && Array.isArray(payload.hits)) ? payload.hits : [];
  matterCreateReviewState.status = 'reviewed';
  matterCreateReviewState.signature = signature;
  matterCreateReviewState.matched = !!(payload && payload.matched);
  matterCreateReviewState.hits = hits;
  matterCreateReviewState.checkedParties =
    (payload && Array.isArray(payload.checked_parties)) ? payload.checked_parties : [];

  if (matterCreateReviewState.matched) {
    setMatterCreateReviewStatus(
      hits.length + ' potential conflict' + (hits.length === 1 ? '' : 's')
      + ' found. Choose a decision to continue.',
      'warning'
    );
  } else {
    setMatterCreateReviewStatus('No conflicts detected. You can create this matter now.', 'success');
  }

  var decision = byId('matter-create-conflict-decision');
  if (decision) decision.value = 'clear';
  var note = byId('matter-create-conflict-note');
  if (note) note.value = '';
  renderMatterCreateHits(hits);
  syncMatterCreateActionState();
}

function handleMatterCreateFormMutation() {
  if (!matterCreateReviewState.signature) {
    syncMatterCreateActionState();
    return;
  }
  var currentSignature = matterCreateFormSignature(readMatterCreateFormValues());
  if (currentSignature !== matterCreateReviewState.signature) {
    resetMatterCreateReview('Matter form changed. Re-run conflict review before creating.', 'warning');
    return;
  }
  syncMatterCreateActionState();
}

function setMatterCreateReviewBusy(isBusy) {
  matterCreateReviewBusy = isBusy;
  var form = byId('matters-create-form');
  if (form) {
    var fields = form.querySelectorAll('input, textarea, select, button');
    for (var i = 0; i < fields.length; i++) {
      fields[i].disabled = isBusy || matterCreateBusy;
    }
  }
  syncMatterCreateActionState();
}

function setCreateMatterBusy(isBusy) {
  matterCreateBusy = isBusy;
  var form = document.getElementById('matters-create-form');
  if (form) {
    var fields = form.querySelectorAll('input, textarea, select, button');
    for (var i = 0; i < fields.length; i++) {
      fields[i].disabled = isBusy || matterCreateReviewBusy;
    }
  }
  syncMatterCreateActionState();
}

function setMattersGroupToggleFromState() {
  var checkbox = byId('matters-group-by-client');
  if (!checkbox) return;
  checkbox.checked = mattersGroupByClient;
}

function normalizeMatterClient(client) {
  var normalized = (client || '').trim();
  return normalized || 'Unspecified client';
}

function getMatterOpenedDate(matter) {
  if (!matter) return '';
  return matter.opened_date || matter.opened_at || '';
}

function buildGroupedMatters() {
  var groups = {};
  for (var i = 0; i < mattersCache.length; i++) {
    var matter = mattersCache[i];
    var key = normalizeMatterClient(matter.client);
    if (!groups[key]) groups[key] = [];
    groups[key].push({ matter: matter, index: i });
  }
  return Object.keys(groups)
    .sort(function(a, b) { return a.localeCompare(b); })
    .map(function(key) {
      groups[key].sort(function(a, b) { return a.matter.id.localeCompare(b.matter.id); });
      return { title: key, items: groups[key] };
    });
}

function renderMatterCardHtml(matter, index) {
  var isActive = matter.id === activeMatterId;
  var isSelected = matter.id === selectedMatterId;
  var html = '<div class="matter-card'
    + (isActive ? ' matter-card--active' : '')
    + (isSelected ? ' matter-card--selected' : '')
    + '">';
  html += '<div class="matter-card-header">';
  html += '<span class="matter-card-id">' + escapeHtml(matter.id) + '</span>';
  if (isActive) html += '<span class="matter-active-chip">Active</span>';
  html += '</div>';
  if (matter.client) {
    html += '<div class="matter-card-row"><span class="matter-card-label">Client</span><span>' + escapeHtml(matter.client) + '</span></div>';
  }
  if (matter.confidentiality) {
    html += '<div class="matter-card-row"><span class="matter-card-label">Confidentiality</span><span>' + escapeHtml(matter.confidentiality) + '</span></div>';
  }
  if (matter.team && matter.team.length) {
    html += '<div class="matter-card-row"><span class="matter-card-label">Team</span><span>' + escapeHtml(matter.team.join(', ')) + '</span></div>';
  }
  if (matter.adversaries && matter.adversaries.length) {
    html += '<div class="matter-card-row"><span class="matter-card-label">Adversaries</span><span>' + escapeHtml(matter.adversaries.join(', ')) + '</span></div>';
  }
  if (matter.retention) {
    html += '<div class="matter-card-row"><span class="matter-card-label">Retention</span><span>' + escapeHtml(matter.retention) + '</span></div>';
  }
  if (matter.jurisdiction) {
    html += '<div class="matter-card-row"><span class="matter-card-label">Jurisdiction</span><span>' + escapeHtml(matter.jurisdiction) + '</span></div>';
  }
  if (matter.practice_area) {
    html += '<div class="matter-card-row"><span class="matter-card-label">Practice area</span><span>' + escapeHtml(matter.practice_area) + '</span></div>';
  }
  var openedDate = getMatterOpenedDate(matter);
  if (openedDate) {
    html += '<div class="matter-card-row"><span class="matter-card-label">Opened</span><span>' + escapeHtml(openedDate) + '</span></div>';
  }
  html += '<div class="matter-card-actions">';
  if (!isActive) {
    html += '<button class="btn-ext activate" data-matter-action="select" data-matter-index="' + index + '">Select</button>';
  }
  html += '<button class="btn-ext" data-matter-action="detail" data-matter-index="' + index + '">Details</button>';
  html += '<button class="btn-ext" data-matter-action="browse" data-matter-index="' + index + '">Browse Files</button>';
  html += '</div>';
  html += '</div>';
  return html;
}

function populateMatterConflictSelector() {
  var select = byId('matters-conflict-matter');
  if (!select) return;
  var selected = select.value;
  var html = '';
  var activeLabel = activeMatterId
    ? ('Use active matter (' + activeMatterId + ')')
    : 'Use active matter (none)';
  html += '<option value="">' + escapeHtml(activeLabel) + '</option>';
  for (var i = 0; i < mattersCache.length; i++) {
    var matter = mattersCache[i];
    var label = matter.id + (matter.client ? (' — ' + matter.client) : '');
    html += '<option value="' + escapeHtml(matter.id) + '">' + escapeHtml(label) + '</option>';
  }
  select.innerHTML = html;
  if (selected) select.value = selected;
}

function renderMatterDetailPlaceholder(message) {
  var panel = byId('matter-detail-panel');
  if (!panel) return;
  panel.innerHTML = '<div class="empty-state">' + escapeHtml(message) + '</div>';
}

function renderMatterSectionToggle(section, label) {
  var activeClass = currentMatterDetailSection === section ? ' is-active' : '';
  return '<button class="btn-ext matter-detail-tab' + activeClass
    + '" data-matter-detail-section="' + escapeHtml(section) + '">' + escapeHtml(label) + '</button>';
}

function renderMatterDetailOverview(selectedMatter) {
  var html = '';

  if (selectedMatter) {
    var metadataRows = [];
    if (selectedMatter.client) metadataRows.push({ label: 'Client', value: selectedMatter.client });
    if (selectedMatter.confidentiality) metadataRows.push({ label: 'Confidentiality', value: selectedMatter.confidentiality });
    if (selectedMatter.retention) metadataRows.push({ label: 'Retention', value: selectedMatter.retention });
    if (selectedMatter.jurisdiction) metadataRows.push({ label: 'Jurisdiction', value: selectedMatter.jurisdiction });
    if (selectedMatter.practice_area) metadataRows.push({ label: 'Practice area', value: selectedMatter.practice_area });
    var selectedOpenedDate = getMatterOpenedDate(selectedMatter);
    if (selectedOpenedDate) metadataRows.push({ label: 'Opened', value: selectedOpenedDate });
    if (selectedMatter.team && selectedMatter.team.length) {
      metadataRows.push({ label: 'Team', value: selectedMatter.team.join(', ') });
    }
    if (selectedMatter.adversaries && selectedMatter.adversaries.length) {
      metadataRows.push({ label: 'Adversaries', value: selectedMatter.adversaries.join(', ') });
    }

    html += '<div class="matter-detail-section">';
    html += '<h5>Matter Metadata</h5>';
    if (!metadataRows.length) {
      html += '<div class="empty-state">No matter metadata available.</div>';
    } else {
      html += '<div class="matter-meta-grid">';
      for (var r = 0; r < metadataRows.length; r++) {
        html += '<div class="matter-meta-item">';
        html += '<span class="matter-meta-label">' + escapeHtml(metadataRows[r].label) + '</span>';
        html += '<span class="matter-meta-value">' + escapeHtml(metadataRows[r].value) + '</span>';
        html += '</div>';
      }
      html += '</div>';
    }
    html += '</div>';
  }

  html += '<div class="matter-detail-section">';
  html += '<h5>Workflow Scorecard</h5>';
  if (!currentMatterDashboard) {
    html += '<div class="empty-state">Scorecard unavailable.</div>';
  } else {
    html += '<div class="matter-scorecard-grid">';
    html += '<div class="matter-scorecard-card"><span class="label">Documents</span><span class="value">' + escapeHtml(String(currentMatterDashboard.document_count || 0)) + '</span></div>';
    html += '<div class="matter-scorecard-card"><span class="label">Drafts</span><span class="value">' + escapeHtml(String(currentMatterDashboard.draft_count || 0)) + '</span></div>';
    html += '<div class="matter-scorecard-card"><span class="label">Checklist</span><span class="value">' + escapeHtml(String(currentMatterDashboard.checklist_completed || 0)) + '/' + escapeHtml(String(currentMatterDashboard.checklist_total || 0)) + '</span></div>';
    html += '<div class="matter-scorecard-card"><span class="label">Deadlines</span><span class="value">' + escapeHtml(String(currentMatterDashboard.upcoming_deadlines_14d || 0)) + ' due / 14d</span></div>';
    html += '</div>';
    if (currentMatterDashboard.next_deadline) {
      var next = currentMatterDashboard.next_deadline;
      var nextLabel = next.title + ' (' + next.date + ')';
      html += '<div class="matter-deadline-next">Next deadline: ' + escapeHtml(nextLabel) + '</div>';
    }
    if ((currentMatterDashboard.overdue_deadlines || 0) > 0) {
      html += '<div class="matter-deadline-overdue">Overdue deadlines: ' + escapeHtml(String(currentMatterDashboard.overdue_deadlines)) + '</div>';
    }
  }
  html += '</div>';

  html += '<div class="matter-detail-section">';
  html += '<h5>Deadlines</h5>';
  if (!currentMatterDeadlines.length) {
    html += '<div class="empty-state">No deadlines parsed yet. Update deadlines/calendar.md to power reminders and filing prep.</div>';
  } else {
    html += '<div class="matter-item-list">';
    for (var d = 0; d < currentMatterDeadlines.length; d++) {
      var deadline = currentMatterDeadlines[d];
      var owner = deadline.owner ? ('Owner: ' + deadline.owner) : '';
      var status = deadline.status ? ('Status: ' + deadline.status) : '';
      var source = deadline.source ? ('Source: ' + deadline.source) : '';
      var meta = [owner, status, source].filter(function(v) { return !!v; }).join(' • ');
      html += '<div class="matter-item-row matter-item-row--deadline">';
      html += '<div class="matter-item-main">';
      html += '<span class="matter-item-path">' + escapeHtml(deadline.title) + '</span>';
      html += '<span class="matter-item-meta">' + escapeHtml(deadline.date + (meta ? (' • ' + meta) : '')) + '</span>';
      html += '</div>';
      if (deadline.is_overdue) {
        html += '<span class="matter-overdue-chip">Overdue</span>';
      }
      html += '</div>';
    }
    html += '</div>';
  }
  html += '</div>';

  html += '<div class="matter-detail-section">';
  html += '<h5>Documents</h5>';
  if (!currentMatterDocuments.length) {
    html += '<div class="empty-state">No indexed documents yet.</div>';
  } else {
    html += '<div class="matter-item-list">';
    for (var i = 0; i < currentMatterDocuments.length; i++) {
      var doc = currentMatterDocuments[i];
      html += '<div class="matter-item-row">';
      html += '<span class="matter-item-path">' + escapeHtml(doc.path) + '</span>';
      if (!doc.is_dir) {
        html += '<button data-matter-detail-action="open-doc" data-path="' + escapeHtml(doc.path) + '">Open</button>';
      }
      html += '</div>';
    }
    html += '</div>';
  }
  html += '</div>';

  html += '<div class="matter-detail-section">';
  html += '<h5>Conversations</h5>';
  if (!currentMatterThreads.length) {
    html += '<div class="empty-state">No conversations bound to this matter yet.</div>';
  } else {
    html += '<div class="matter-item-list">';
    for (var c = 0; c < currentMatterThreads.length; c++) {
      var convo = currentMatterThreads[c];
      var title = convo.title || ((convo.id || '').substring(0, 8));
      var updated = convo.updated_at ? ('Updated: ' + convo.updated_at) : '';
      var turns = 'Turns: ' + String(convo.turn_count || 0);
      var convoMeta = [turns, updated].filter(function(v) { return !!v; }).join(' • ');
      html += '<div class="matter-item-row">';
      html += '<div class="matter-item-main">';
      html += '<span class="matter-item-path">' + escapeHtml(title) + '</span>';
      html += '<span class="matter-item-meta">' + escapeHtml(convoMeta) + '</span>';
      html += '</div>';
      html += '<button data-matter-detail-action="open-thread" data-thread-id="' + escapeHtml(convo.id) + '">Open in Chat</button>';
      html += '</div>';
    }
    html += '</div>';
  }
  html += '</div>';

  html += '<div class="matter-detail-section">';
  html += '<h5>Templates</h5>';
  if (!currentMatterTemplates.length) {
    html += '<div class="empty-state">No templates found for this matter.</div>';
  } else {
    html += '<div class="matter-item-list">';
    for (var t = 0; t < currentMatterTemplates.length; t++) {
      var template = currentMatterTemplates[t];
      html += '<div class="matter-item-row">';
      html += '<span class="matter-item-path">' + escapeHtml(template.path) + '</span>';
      html += '<button data-matter-detail-action="preview-template" data-path="' + escapeHtml(template.path) + '">Preview</button>';
      html += '<button data-matter-detail-action="apply-template" data-template-name="' + escapeHtml(template.name) + '">Apply</button>';
      html += '</div>';
    }
    html += '</div>';
  }
  html += '</div>';
  return html;
}

function renderMatterDetailWork() {
  var html = '<div class="matter-detail-section">';
  html += '<h5>Capture Work</h5>';
  html += '<div class="matter-detail-inline-actions">';
  html += '<button class="btn-ext" data-matter-detail-action="create-task">+ Task</button>';
  html += '<button class="btn-ext" data-matter-detail-action="create-note">+ Note</button>';
  html += '<button class="btn-ext" data-matter-detail-action="create-time-entry">+ Time</button>';
  html += '<button class="btn-ext" data-matter-detail-action="create-expense-entry">+ Expense</button>';
  html += '</div>';
  html += '</div>';

  if (currentMatterWork.loading && !currentMatterWork.loaded) {
    return html + '<div class="empty-state">Loading matter work data…</div>';
  }
  if (currentMatterWork.error && !currentMatterWork.loaded) {
    return html + '<div class="empty-state error-state">Failed to load work data: '
      + escapeHtml(currentMatterWork.error) + '</div>';
  }
  if (!currentMatterWork.loaded) {
    return html + '<div class="empty-state">Open Work to view tasks, notes, time, and expenses.</div>';
  }

  html += '<div class="matter-detail-section">';
  html += '<h5>Tasks</h5>';
  if (!currentMatterWork.tasks.length) {
    html += '<div class="empty-state">No tasks yet.</div>';
  } else {
    html += '<div class="matter-item-list">';
    for (var t = 0; t < currentMatterWork.tasks.length; t++) {
      var task = currentMatterWork.tasks[t];
      var taskMeta = 'Status: ' + String(task.status || 'todo');
      if (task.assignee) taskMeta += ' • Assignee: ' + task.assignee;
      if (task.due_at) taskMeta += ' • Due: ' + task.due_at;
      html += '<div class="matter-item-row">';
      html += '<div class="matter-item-main">';
      html += '<span class="matter-item-path">' + escapeHtml(task.title || '') + '</span>';
      html += '<span class="matter-item-meta">' + escapeHtml(taskMeta) + '</span>';
      html += '</div>';
      html += '</div>';
    }
    html += '</div>';
  }
  html += '</div>';

  html += '<div class="matter-detail-section">';
  html += '<h5>Notes</h5>';
  if (!currentMatterWork.notes.length) {
    html += '<div class="empty-state">No notes yet.</div>';
  } else {
    html += '<div class="matter-item-list">';
    for (var n = 0; n < currentMatterWork.notes.length; n++) {
      var note = currentMatterWork.notes[n];
      var noteMeta = 'Author: ' + String(note.author || 'unknown');
      if (note.pinned) noteMeta += ' • Pinned';
      var preview = String(note.body || '').replace(/\s+/g, ' ').trim();
      if (preview.length > 160) preview = preview.substring(0, 157) + '...';
      html += '<div class="matter-item-row">';
      html += '<div class="matter-item-main">';
      html += '<span class="matter-item-path">' + escapeHtml(preview || '(empty note)') + '</span>';
      html += '<span class="matter-item-meta">' + escapeHtml(noteMeta) + '</span>';
      html += '</div>';
      html += '</div>';
    }
    html += '</div>';
  }
  html += '</div>';

  html += '<div class="matter-detail-section">';
  html += '<h5>Time Entries</h5>';
  if (!currentMatterWork.timeEntries.length) {
    html += '<div class="empty-state">No time entries yet.</div>';
  } else {
    html += '<div class="matter-item-list">';
    for (var ti = 0; ti < currentMatterWork.timeEntries.length; ti++) {
      var time = currentMatterWork.timeEntries[ti];
      var timeMeta = String(time.hours || '0') + 'h • ' + String(time.timekeeper || '');
      if (time.entry_date) timeMeta += ' • ' + time.entry_date;
      if (time.billable) timeMeta += ' • billable';
      html += '<div class="matter-item-row">';
      html += '<div class="matter-item-main">';
      html += '<span class="matter-item-path">' + escapeHtml(time.description || '') + '</span>';
      html += '<span class="matter-item-meta">' + escapeHtml(timeMeta) + '</span>';
      html += '</div>';
      html += '</div>';
    }
    html += '</div>';
  }
  html += '</div>';

  html += '<div class="matter-detail-section">';
  html += '<h5>Expenses</h5>';
  if (!currentMatterWork.expenseEntries.length) {
    html += '<div class="empty-state">No expense entries yet.</div>';
  } else {
    html += '<div class="matter-item-list">';
    for (var e = 0; e < currentMatterWork.expenseEntries.length; e++) {
      var expense = currentMatterWork.expenseEntries[e];
      var expenseMeta = '$' + String(expense.amount || '0') + ' • ' + String(expense.category || '');
      if (expense.entry_date) expenseMeta += ' • ' + expense.entry_date;
      if (expense.billable) expenseMeta += ' • billable';
      html += '<div class="matter-item-row">';
      html += '<div class="matter-item-main">';
      html += '<span class="matter-item-path">' + escapeHtml(expense.description || '') + '</span>';
      html += '<span class="matter-item-meta">' + escapeHtml(expenseMeta) + '</span>';
      html += '</div>';
      html += '</div>';
    }
    html += '</div>';
  }
  html += '</div>';

  return html;
}

function renderMatterDetailFinance() {
  var html = '<div class="matter-detail-section">';
  html += '<h5>Finance Actions</h5>';
  html += '<div class="matter-detail-inline-actions">';
  html += '<button class="btn-ext" data-matter-detail-action="record-trust-deposit">Record Trust Deposit</button>';
  html += '</div>';
  html += '</div>';

  if (currentMatterFinance.loading && !currentMatterFinance.loaded) {
    return html + '<div class="empty-state">Loading finance data…</div>';
  }
  if (currentMatterFinance.error && !currentMatterFinance.loaded) {
    return html + '<div class="empty-state error-state">Failed to load finance data: '
      + escapeHtml(currentMatterFinance.error) + '</div>';
  }
  if (!currentMatterFinance.loaded) {
    return html + '<div class="empty-state">Open Finance to view time summary, trust ledger, and invoices.</div>';
  }

  html += '<div class="matter-detail-section">';
  html += '<h5>Time Summary</h5>';
  if (!currentMatterFinance.timeSummary) {
    html += '<div class="empty-state">No time summary available.</div>';
  } else {
    var summary = currentMatterFinance.timeSummary;
    html += '<div class="matter-scorecard-grid">';
    html += '<div class="matter-scorecard-card"><span class="label">Total Hours</span><span class="value">' + escapeHtml(String(summary.total_hours || '0')) + '</span></div>';
    html += '<div class="matter-scorecard-card"><span class="label">Billable Hours</span><span class="value">' + escapeHtml(String(summary.billable_hours || '0')) + '</span></div>';
    html += '<div class="matter-scorecard-card"><span class="label">Unbilled Hours</span><span class="value">' + escapeHtml(String(summary.unbilled_hours || '0')) + '</span></div>';
    html += '<div class="matter-scorecard-card"><span class="label">Total Expenses</span><span class="value">' + escapeHtml(String(summary.total_expenses || '0')) + '</span></div>';
    html += '<div class="matter-scorecard-card"><span class="label">Billable Expenses</span><span class="value">' + escapeHtml(String(summary.billable_expenses || '0')) + '</span></div>';
    html += '<div class="matter-scorecard-card"><span class="label">Unbilled Expenses</span><span class="value">' + escapeHtml(String(summary.unbilled_expenses || '0')) + '</span></div>';
    html += '</div>';
  }
  html += '</div>';

  html += '<div class="matter-detail-section">';
  html += '<h5>Trust Ledger</h5>';
  if (!currentMatterFinance.trustLedger) {
    html += '<div class="empty-state">No trust ledger available.</div>';
  } else {
    html += '<div class="matter-finance-balance">Current balance: $'
      + escapeHtml(String(currentMatterFinance.trustLedger.balance || '0')) + '</div>';
    if (!currentMatterFinance.trustLedger.entries || !currentMatterFinance.trustLedger.entries.length) {
      html += '<div class="empty-state">No trust entries yet.</div>';
    } else {
      html += '<div class="matter-item-list">';
      for (var tl = 0; tl < currentMatterFinance.trustLedger.entries.length; tl++) {
        var entry = currentMatterFinance.trustLedger.entries[tl];
        var trustMeta = String(entry.entry_type || '') + ' • $' + String(entry.amount || '0');
        if (entry.recorded_by) trustMeta += ' • ' + entry.recorded_by;
        html += '<div class="matter-item-row">';
        html += '<div class="matter-item-main">';
        html += '<span class="matter-item-path">' + escapeHtml(entry.description || '') + '</span>';
        html += '<span class="matter-item-meta">' + escapeHtml(trustMeta) + '</span>';
        html += '</div>';
        html += '</div>';
      }
      html += '</div>';
    }
  }
  html += '</div>';

  html += '<div class="matter-detail-section">';
  html += '<h5>Invoices</h5>';
  if (!currentMatterFinance.invoices.length) {
    html += '<div class="empty-state">No invoices saved yet.</div>';
  } else {
    html += '<div class="matter-item-list">';
    for (var iv = 0; iv < currentMatterFinance.invoices.length; iv++) {
      var invoice = currentMatterFinance.invoices[iv];
      var invoiceMeta = 'Status: ' + String(invoice.status || 'draft') + ' • Total: $' + String(invoice.total || '0');
      if (invoice.due_date) invoiceMeta += ' • Due: ' + invoice.due_date;
      html += '<div class="matter-item-row">';
      html += '<div class="matter-item-main">';
      html += '<span class="matter-item-path">' + escapeHtml(invoice.invoice_number || invoice.id || '') + '</span>';
      html += '<span class="matter-item-meta">' + escapeHtml(invoiceMeta) + '</span>';
      html += '</div>';
      html += '<button data-matter-detail-action="open-invoice" data-invoice-id="' + escapeHtml(invoice.id) + '">View</button>';
      html += '</div>';
    }
    html += '</div>';
  }
  html += '</div>';

  return html;
}

function renderMatterDetail() {
  var panel = byId('matter-detail-panel');
  if (!panel) return;
  if (!selectedMatterId) {
    renderMatterDetailPlaceholder('Select a matter to view Overview, Work, and Finance details.');
    return;
  }

  var selectedMatter = null;
  for (var m = 0; m < mattersCache.length; m++) {
    if (mattersCache[m] && mattersCache[m].id === selectedMatterId) {
      selectedMatter = mattersCache[m];
      break;
    }
  }

  var html = '<div class="matter-detail-header">';
  html += '<h4>' + escapeHtml(selectedMatterId) + '</h4>';
  html += '<div class="matter-detail-header-actions">';
  html += '<button data-matter-detail-action="open-doc" data-path="' + escapeHtml('matters/' + selectedMatterId + '/matter.yaml') + '">Open matter.yaml</button>';
  html += '<button data-matter-detail-action="export-retrieval-packet">Export AI Packet</button>';
  html += '<button data-matter-detail-action="build-filing-package">Build Filing Package</button>';
  html += '</div>';
  html += '</div>';

  html += '<div class="matter-detail-tabs">';
  html += renderMatterSectionToggle('overview', 'Overview');
  html += renderMatterSectionToggle('work', 'Work');
  html += renderMatterSectionToggle('finance', 'Finance');
  html += '</div>';

  if (currentMatterDetailSection === 'work') {
    html += renderMatterDetailWork();
  } else if (currentMatterDetailSection === 'finance') {
    html += renderMatterDetailFinance();
  } else {
    html += renderMatterDetailOverview(selectedMatter);
  }

  panel.innerHTML = html;
}

function setMatterDetailSection(section) {
  if (!selectedMatterId) return;
  if (section !== 'overview' && section !== 'work' && section !== 'finance') return;
  if (currentMatterDetailSection === section) return;
  currentMatterDetailSection = section;
  renderMatterDetail();
  if (section === 'work') {
    loadMatterWorkDataIfNeeded(false);
    return;
  }
  if (section === 'finance') {
    loadMatterFinanceDataIfNeeded(false);
  }
}

function loadMatterWorkDataIfNeeded(force) {
  if (!selectedMatterId) return Promise.resolve();
  if (!force && (currentMatterWork.loaded || currentMatterWork.loading)) return Promise.resolve();
  var matterId = selectedMatterId;
  currentMatterWork.loading = true;
  currentMatterWork.error = '';
  if (currentMatterDetailSection === 'work') renderMatterDetail();
  var requestVersion = beginRequest('matterDetailWork');
  return Promise.all([
    apiFetch('/api/matters/' + encodeURIComponent(matterId) + '/tasks'),
    apiFetch('/api/matters/' + encodeURIComponent(matterId) + '/notes'),
    apiFetch('/api/matters/' + encodeURIComponent(matterId) + '/time'),
    apiFetch('/api/matters/' + encodeURIComponent(matterId) + '/expenses'),
  ]).then(function(results) {
    if (!isCurrentRequest('matterDetailWork', requestVersion)) return;
    if (selectedMatterId !== matterId) return;
    currentMatterWork.loaded = true;
    currentMatterWork.loading = false;
    currentMatterWork.error = '';
    currentMatterWork.tasks = (results[0] && results[0].tasks) ? results[0].tasks : [];
    currentMatterWork.notes = (results[1] && results[1].notes) ? results[1].notes : [];
    currentMatterWork.timeEntries = (results[2] && results[2].entries) ? results[2].entries : [];
    currentMatterWork.expenseEntries = (results[3] && results[3].entries) ? results[3].entries : [];
    if (currentMatterDetailSection === 'work') renderMatterDetail();
  }).catch(function(err) {
    if (!isCurrentRequest('matterDetailWork', requestVersion)) return;
    if (selectedMatterId !== matterId) return;
    currentMatterWork.loading = false;
    currentMatterWork.error = err.message;
    if (currentMatterDetailSection === 'work') renderMatterDetail();
  });
}

function loadMatterFinanceDataIfNeeded(force) {
  if (!selectedMatterId) return Promise.resolve();
  if (!force && (currentMatterFinance.loaded || currentMatterFinance.loading)) return Promise.resolve();
  var matterId = selectedMatterId;
  currentMatterFinance.loading = true;
  currentMatterFinance.error = '';
  if (currentMatterDetailSection === 'finance') renderMatterDetail();
  var requestVersion = beginRequest('matterDetailFinance');
  return Promise.all([
    apiFetch('/api/matters/' + encodeURIComponent(matterId) + '/time-summary'),
    apiFetch('/api/matters/' + encodeURIComponent(matterId) + '/trust/ledger'),
    apiFetch('/api/matters/' + encodeURIComponent(matterId) + '/invoices?limit=25'),
  ]).then(function(results) {
    if (!isCurrentRequest('matterDetailFinance', requestVersion)) return;
    if (selectedMatterId !== matterId) return;
    currentMatterFinance.loaded = true;
    currentMatterFinance.loading = false;
    currentMatterFinance.error = '';
    currentMatterFinance.timeSummary = results[0] || null;
    currentMatterFinance.trustLedger = results[1] || null;
    currentMatterFinance.invoices = (results[2] && results[2].invoices) ? results[2].invoices : [];
    if (currentMatterDetailSection === 'finance') renderMatterDetail();
  }).catch(function(err) {
    if (!isCurrentRequest('matterDetailFinance', requestVersion)) return;
    if (selectedMatterId !== matterId) return;
    currentMatterFinance.loading = false;
    currentMatterFinance.error = err.message;
    if (currentMatterDetailSection === 'finance') renderMatterDetail();
  });
}

function openMatterDetail(id) {
  if (!id) return;
  selectedMatterId = id;
  currentMatterDetailSection = 'overview';
  currentMatterDocuments = [];
  currentMatterTemplates = [];
  currentMatterDashboard = null;
  currentMatterDeadlines = [];
  currentMatterThreads = [];
  currentMatterWork = createEmptyMatterWorkState();
  currentMatterFinance = createEmptyMatterFinanceState();
  renderMatters();
  renderMatterDetailPlaceholder('Loading detail for ' + id + '…');

  var requestVersion = beginRequest('matterDetail');
  Promise.all([
    apiFetch('/api/matters/' + encodeURIComponent(id) + '/documents?include_templates=false'),
    apiFetch('/api/matters/' + encodeURIComponent(id) + '/dashboard'),
    apiFetch('/api/matters/' + encodeURIComponent(id) + '/deadlines'),
    apiFetch('/api/matters/' + encodeURIComponent(id) + '/templates'),
    apiFetch('/api/chat/threads?matter_id=' + encodeURIComponent(id)),
  ]).then(function (results) {
    if (!isCurrentRequest('matterDetail', requestVersion)) return;
    if (selectedMatterId !== id) return;
    var docsData = results[0];
    var dashboardData = results[1];
    var deadlinesData = results[2];
    var templatesData = results[3];
    var threadsData = results[4];
    currentMatterDocuments = (docsData && docsData.documents) ? docsData.documents : [];
    currentMatterDashboard = dashboardData || null;
    currentMatterDeadlines = (deadlinesData && deadlinesData.deadlines) ? deadlinesData.deadlines : [];
    currentMatterTemplates = (templatesData && templatesData.templates) ? templatesData.templates : [];
    currentMatterThreads = (threadsData && threadsData.threads) ? threadsData.threads : [];
    renderMatterDetail();
    renderMatters();
  }).catch(function (err) {
    if (!isCurrentRequest('matterDetail', requestVersion)) return;
    renderMatterDetailPlaceholder('Failed to load matter detail: ' + err.message);
  });
}

function openMatterPathInMemory(path) {
  if (!path) return;
  switchTab('memory');
  readMemoryFile(path);
}

function applyMatterTemplate(templateName) {
  if (!selectedMatterId || !templateName) return;
  apiFetch('/api/matters/' + encodeURIComponent(selectedMatterId) + '/templates/apply', {
    method: 'POST',
    body: { template_name: templateName },
  }).then(function (data) {
    showToast('Template applied to ' + data.path, 'success');
    openMatterDetail(selectedMatterId);
  }).catch(function (err) {
    showToast('Failed to apply template: ' + err.message, 'error');
  });
}

function buildMatterFilingPackage() {
  if (!selectedMatterId) return;
  apiFetch('/api/matters/' + encodeURIComponent(selectedMatterId) + '/filing-package', {
    method: 'POST',
  }).then(function (data) {
    if (!data || !data.path) return;
    showToast('Filing package created: ' + data.path, 'success');
    openMatterPathInMemory(data.path);
    openMatterDetail(selectedMatterId);
  }).catch(function (err) {
    showToast('Failed to build filing package: ' + err.message, 'error');
  });
}

function exportMatterRetrievalPacket(unredacted) {
  if (!selectedMatterId) return;
  apiFetch('/api/matters/' + encodeURIComponent(selectedMatterId) + '/exports/retrieval-packet', {
    method: 'POST',
    body: { unredacted: !!unredacted },
  }).then(function(data) {
    if (!data || !Array.isArray(data.files)) {
      showToast('Retrieval export created, but response is missing files list.', 'error');
      return;
    }
    showToast('Matter AI packet created (' + data.files.length + ' files)', 'success');
    if (data.warning) {
      showToast(data.warning, 'error');
    }
    if (data.files.length > 0) {
      openMatterPathInMemory(data.files[0]);
    }
    openMatterDetail(selectedMatterId);
  }).catch(function(err) {
    showToast('Failed to export AI packet: ' + err.message, 'error');
  });
}

function actionInputValue(id) {
  var el = byId(id);
  return el ? el.value.trim() : '';
}

function isMatterActionModalOpen() {
  var modal = byId('matter-action-modal');
  return !!(modal && !modal.classList.contains('is-hidden'));
}

function trapMatterActionModalFocus(event) {
  var modal = byId('matter-action-modal');
  if (!modal || modal.classList.contains('is-hidden')) return;
  var dialog = modal.querySelector('.configure-modal');
  var focusables = getFocusableElements(dialog);
  if (!focusables.length) {
    event.preventDefault();
    if (dialog && typeof dialog.focus === 'function') dialog.focus();
    return;
  }
  var first = focusables[0];
  var last = focusables[focusables.length - 1];
  var active = document.activeElement;
  if (event.shiftKey) {
    if (active === first || !dialog.contains(active)) {
      event.preventDefault();
      last.focus();
    }
    return;
  }
  if (active === last) {
    event.preventDefault();
    first.focus();
  }
}

function setMatterActionError(message) {
  var error = byId('matter-action-form-error');
  if (!error) return;
  if (!message) {
    error.textContent = '';
    error.classList.add('is-hidden');
    return;
  }
  error.textContent = message;
  error.classList.remove('is-hidden');
}

function setMatterActionModalBusy(isBusy) {
  matterActionModalBusy = isBusy;
  var form = byId('matter-action-form');
  if (!form) return;
  var fields = form.querySelectorAll('input, textarea, select, button');
  for (var i = 0; i < fields.length; i++) {
    fields[i].disabled = isBusy;
  }
  var submit = byId('matter-action-submit-btn');
  if (submit) submit.textContent = isBusy ? 'Saving…' : 'Save';
}

function actionSelectChecked(id) {
  var el = byId(id);
  return !!(el && el.checked);
}

function actionBodyForType(type) {
  if (type === 'task') {
    var title = actionInputValue('matter-action-task-title');
    if (!title) return { error: 'Task title is required.' };
    var taskBody = {
      title: title,
      status: actionInputValue('matter-action-task-status') || 'todo',
    };
    var description = actionInputValue('matter-action-task-description');
    if (description) taskBody.description = description;
    var assignee = actionInputValue('matter-action-task-assignee');
    if (assignee) taskBody.assignee = assignee;
    var dueAtRaw = actionInputValue('matter-action-task-due-at');
    if (dueAtRaw) {
      var dueAt = new Date(dueAtRaw);
      if (isNaN(dueAt.getTime())) return { error: 'Due date must be valid.' };
      taskBody.due_at = dueAt.toISOString();
    }
    return { path: '/tasks', body: taskBody };
  }
  if (type === 'note') {
    var author = actionInputValue('matter-action-note-author');
    var noteBody = actionInputValue('matter-action-note-body');
    if (!author || !noteBody) return { error: 'Author and note body are required.' };
    return {
      path: '/notes',
      body: {
        author: author,
        body: noteBody,
        pinned: actionSelectChecked('matter-action-note-pinned'),
      },
    };
  }
  if (type === 'time') {
    var timekeeper = actionInputValue('matter-action-timekeeper');
    var timeDescription = actionInputValue('matter-action-time-description');
    var hours = actionInputValue('matter-action-time-hours');
    var timeDate = actionInputValue('matter-action-time-date');
    if (!timekeeper || !timeDescription || !hours || !timeDate) {
      return { error: 'Timekeeper, description, hours, and date are required.' };
    }
    var timeBody = {
      timekeeper: timekeeper,
      description: timeDescription,
      hours: hours,
      entry_date: timeDate,
      billable: actionSelectChecked('matter-action-time-billable'),
    };
    var hourlyRate = actionInputValue('matter-action-time-rate');
    if (hourlyRate) timeBody.hourly_rate = hourlyRate;
    return { path: '/time', body: timeBody };
  }
  if (type === 'expense') {
    var submittedBy = actionInputValue('matter-action-expense-submitted-by');
    var expenseDescription = actionInputValue('matter-action-expense-description');
    var amount = actionInputValue('matter-action-expense-amount');
    var category = actionInputValue('matter-action-expense-category');
    var expenseDate = actionInputValue('matter-action-expense-date');
    if (!submittedBy || !expenseDescription || !amount || !category || !expenseDate) {
      return { error: 'Submitted by, description, amount, category, and date are required.' };
    }
    var expenseBody = {
      submitted_by: submittedBy,
      description: expenseDescription,
      amount: amount,
      category: category,
      entry_date: expenseDate,
      billable: actionSelectChecked('matter-action-expense-billable'),
    };
    var receiptPath = actionInputValue('matter-action-expense-receipt');
    if (receiptPath) expenseBody.receipt_path = receiptPath;
    return { path: '/expenses', body: expenseBody };
  }
  if (type === 'deposit') {
    var depositAmount = actionInputValue('matter-action-deposit-amount');
    var recordedBy = actionInputValue('matter-action-deposit-recorded-by');
    if (!depositAmount || !recordedBy) {
      return { error: 'Amount and recorded by are required.' };
    }
    var depositBody = {
      amount: depositAmount,
      recorded_by: recordedBy,
    };
    var depositDescription = actionInputValue('matter-action-deposit-description');
    if (depositDescription) depositBody.description = depositDescription;
    return { path: '/trust/deposit', body: depositBody };
  }
  return { error: 'Unsupported matter action type.' };
}

function openMatterActionModal(type) {
  if (!selectedMatterId) return;
  var modal = byId('matter-action-modal');
  var formBody = byId('matter-action-form-body');
  var title = byId('matter-action-modal-title');
  var submit = byId('matter-action-submit-btn');
  if (!modal || !formBody || !title || !submit) return;

  var html = '';
  var heading = '';
  if (type === 'task') {
    heading = 'Add Task';
    html += '<label>Title<input id="matter-action-task-title" type="text" required></label>';
    html += '<label>Description<textarea id="matter-action-task-description" rows="4"></textarea></label>';
    html += '<label>Status<select id="matter-action-task-status"><option value="todo">todo</option><option value="in_progress">in_progress</option><option value="blocked">blocked</option><option value="done">done</option><option value="cancelled">cancelled</option></select></label>';
    html += '<label>Assignee (optional)<input id="matter-action-task-assignee" type="text"></label>';
    html += '<label>Due at (optional)<input id="matter-action-task-due-at" type="datetime-local"></label>';
  } else if (type === 'note') {
    heading = 'Add Note';
    html += '<label>Author<input id="matter-action-note-author" type="text" value="Attorney" required></label>';
    html += '<label>Note<textarea id="matter-action-note-body" rows="6" required></textarea></label>';
    html += '<label class="matter-action-checkbox"><input id="matter-action-note-pinned" type="checkbox"> Pin note</label>';
  } else if (type === 'time') {
    heading = 'Add Time Entry';
    html += '<label>Timekeeper<input id="matter-action-timekeeper" type="text" value="Attorney" required></label>';
    html += '<label>Description<input id="matter-action-time-description" type="text" required></label>';
    html += '<label>Hours<input id="matter-action-time-hours" type="number" min="0" step="0.1" required></label>';
    html += '<label>Hourly rate (optional)<input id="matter-action-time-rate" type="number" min="0" step="0.01"></label>';
    html += '<label>Entry date<input id="matter-action-time-date" type="date" value="' + todayIsoDate() + '" required></label>';
    html += '<label class="matter-action-checkbox"><input id="matter-action-time-billable" type="checkbox" checked> Billable</label>';
  } else if (type === 'expense') {
    heading = 'Add Expense Entry';
    html += '<label>Submitted by<input id="matter-action-expense-submitted-by" type="text" value="Attorney" required></label>';
    html += '<label>Description<input id="matter-action-expense-description" type="text" required></label>';
    html += '<label>Amount<input id="matter-action-expense-amount" type="number" min="0" step="0.01" required></label>';
    html += '<label>Category<select id="matter-action-expense-category"><option value="filing_fee">filing_fee</option><option value="travel">travel</option><option value="postage">postage</option><option value="expert">expert</option><option value="copying">copying</option><option value="court_reporter">court_reporter</option><option value="other" selected>other</option></select></label>';
    html += '<label>Entry date<input id="matter-action-expense-date" type="date" value="' + todayIsoDate() + '" required></label>';
    html += '<label>Receipt path (optional)<input id="matter-action-expense-receipt" type="text" placeholder="matters/' + escapeHtml(selectedMatterId) + '/receipts/..."></label>';
    html += '<label class="matter-action-checkbox"><input id="matter-action-expense-billable" type="checkbox" checked> Billable</label>';
  } else if (type === 'deposit') {
    heading = 'Record Trust Deposit';
    html += '<label>Amount<input id="matter-action-deposit-amount" type="number" min="0" step="0.01" required></label>';
    html += '<label>Recorded by<input id="matter-action-deposit-recorded-by" type="text" value="Attorney" required></label>';
    html += '<label>Description (optional)<input id="matter-action-deposit-description" type="text" value="Trust deposit"></label>';
  } else {
    return;
  }

  matterActionModalType = type;
  matterActionModalLastFocus = document.activeElement instanceof HTMLElement
    ? document.activeElement
    : null;
  title.textContent = heading;
  submit.textContent = 'Save';
  formBody.innerHTML = html;
  setMatterActionError('');
  setMatterActionModalBusy(false);
  modal.classList.remove('is-hidden');

  var firstInput = formBody.querySelector('input, textarea, select');
  if (firstInput && typeof firstInput.focus === 'function') {
    requestAnimationFrame(function() { firstInput.focus(); });
  }
}

function closeMatterActionModal() {
  var modal = byId('matter-action-modal');
  if (!modal || matterActionModalBusy) return;
  modal.classList.add('is-hidden');
  matterActionModalType = null;
  setMatterActionError('');
  var returnFocus = matterActionModalLastFocus;
  if (!returnFocus || !document.contains(returnFocus)) {
    returnFocus = byId('matter-detail-panel');
  }
  if (returnFocus && typeof returnFocus.focus === 'function') {
    returnFocus.focus();
  }
  matterActionModalLastFocus = null;
}

function submitMatterActionForm() {
  if (!selectedMatterId || !matterActionModalType || matterActionModalBusy) return;
  var payload = actionBodyForType(matterActionModalType);
  if (payload.error) {
    setMatterActionError(payload.error);
    return;
  }
  setMatterActionError('');
  setMatterActionModalBusy(true);
  apiFetch('/api/matters/' + encodeURIComponent(selectedMatterId) + payload.path, {
    method: 'POST',
    body: payload.body,
  }).then(function() {
    if (matterActionModalType === 'task') showToast('Task created', 'success');
    if (matterActionModalType === 'note') showToast('Note created', 'success');
    if (matterActionModalType === 'time') showToast('Time entry created', 'success');
    if (matterActionModalType === 'expense') showToast('Expense entry created', 'success');
    if (matterActionModalType === 'deposit') showToast('Trust deposit recorded', 'success');

    var actionType = matterActionModalType;
    setMatterActionModalBusy(false);
    closeMatterActionModal();
    if (actionType === 'task' || actionType === 'note') {
      currentMatterDetailSection = 'work';
      renderMatterDetail();
      loadMatterWorkDataIfNeeded(true);
      return;
    }
    if (actionType === 'time' || actionType === 'expense') {
      currentMatterDetailSection = 'work';
      renderMatterDetail();
      loadMatterWorkDataIfNeeded(true);
      loadMatterFinanceDataIfNeeded(true);
      return;
    }
    if (actionType === 'deposit') {
      currentMatterDetailSection = 'finance';
      renderMatterDetail();
      loadMatterFinanceDataIfNeeded(true);
    }
  }).catch(function(err) {
    var payloadErr = parseErrorPayload(err);
    setMatterActionError(payloadErr && payloadErr.error ? payloadErr.error : err.message);
  }).finally(function() {
    setMatterActionModalBusy(false);
  });
}

function closeMatterInvoiceDetailModal() {
  var existing = byId('matter-invoice-detail-overlay');
  if (existing) existing.remove();
}

function openMatterInvoiceDetail(invoiceId) {
  apiFetch('/api/invoices/' + encodeURIComponent(invoiceId)).then(function(data) {
    closeMatterInvoiceDetailModal();
    var overlay = document.createElement('div');
    overlay.className = 'configure-overlay';
    overlay.id = 'matter-invoice-detail-overlay';

    var modal = document.createElement('div');
    modal.className = 'configure-modal matter-invoice-detail-modal';

    var header = document.createElement('div');
    header.className = 'matter-create-modal-header';
    var title = document.createElement('h3');
    var invoice = data && data.invoice ? data.invoice : null;
    title.textContent = invoice ? ('Invoice ' + (invoice.invoice_number || invoice.id || '')) : 'Invoice Detail';
    var closeBtn = document.createElement('button');
    closeBtn.type = 'button';
    closeBtn.className = 'btn-ext';
    closeBtn.textContent = 'Close';
    closeBtn.addEventListener('click', closeMatterInvoiceDetailModal);
    header.appendChild(title);
    header.appendChild(closeBtn);
    modal.appendChild(header);

    var body = document.createElement('div');
    body.className = 'matter-invoice-detail-body';
    var html = '';
    if (!invoice) {
      html = '<div class="empty-state">Invoice details unavailable.</div>';
    } else {
      html += '<div class="matter-meta-grid">';
      html += '<div class="matter-meta-item"><span class="matter-meta-label">Status</span><span class="matter-meta-value">' + escapeHtml(invoice.status || '') + '</span></div>';
      html += '<div class="matter-meta-item"><span class="matter-meta-label">Total</span><span class="matter-meta-value">$' + escapeHtml(invoice.total || '0') + '</span></div>';
      html += '<div class="matter-meta-item"><span class="matter-meta-label">Paid</span><span class="matter-meta-value">$' + escapeHtml(invoice.paid_amount || '0') + '</span></div>';
      html += '<div class="matter-meta-item"><span class="matter-meta-label">Due</span><span class="matter-meta-value">' + escapeHtml(invoice.due_date || 'N/A') + '</span></div>';
      html += '</div>';
      var items = data && data.line_items ? data.line_items : [];
      html += '<div class="matter-detail-section"><h5>Line Items</h5>';
      if (!items.length) {
        html += '<div class="empty-state">No line items.</div>';
      } else {
        html += '<div class="matter-item-list">';
        for (var i = 0; i < items.length; i++) {
          var item = items[i];
          var itemMeta = String(item.quantity || '0') + ' × $' + String(item.unit_price || '0') + ' = $' + String(item.amount || '0');
          html += '<div class="matter-item-row"><div class="matter-item-main">';
          html += '<span class="matter-item-path">' + escapeHtml(item.description || '') + '</span>';
          html += '<span class="matter-item-meta">' + escapeHtml(itemMeta) + '</span>';
          html += '</div></div>';
        }
        html += '</div>';
      }
      html += '</div>';
    }
    body.innerHTML = html;
    modal.appendChild(body);
    overlay.appendChild(modal);
    overlay.addEventListener('click', function(e) {
      if (e.target === overlay) closeMatterInvoiceDetailModal();
    });
    document.body.appendChild(overlay);
  }).catch(function(err) {
    showToast('Failed to load invoice detail: ' + err.message, 'error');
  });
}

function runMatterConflictCheck() {
  var text = byId('matters-conflict-text') ? byId('matters-conflict-text').value.trim() : '';
  var matterSelect = byId('matters-conflict-matter');
  var matterOverride = matterSelect ? matterSelect.value : '';
  var result = byId('matters-conflict-result');

  if (!text) {
    if (result) result.textContent = 'Enter text to run a conflict check.';
    return;
  }
  if (result) result.textContent = 'Checking…';

  apiFetch('/api/matters/conflicts/check', {
    method: 'POST',
    body: {
      text: text,
      matter_id: matterOverride || null,
    },
  }).then(function (data) {
    var context = data && data.matter_id ? data.matter_id : (activeMatterId || 'none');
    if (data && data.matched) {
      if (result) result.textContent = 'Potential conflict detected: ' + data.conflict + ' (context: ' + context + ')';
      showToast('Conflict detected: ' + data.conflict, 'error');
      return;
    }
    if (result) result.textContent = 'No conflict hit (context: ' + context + ').';
    showToast('No conflict hit detected.', 'success');
  }).catch(function (err) {
    if (result) result.textContent = 'Conflict check failed: ' + err.message;
  });
}

function clearMatterConflictCheck() {
  if (!confirm('Clear conflict check input and results?')) return;
  var text = byId('matters-conflict-text');
  var result = byId('matters-conflict-result');
  if (text) text.value = '';
  if (result) result.textContent = '';
}

/**
 * Load (or reload) the Matters tab: fetches list and active matter in
 * parallel, then renders the panel and updates the badge.
 */
function loadMatters() {
  var requestVersion = beginRequest('matters');
  Promise.all([
    apiFetch('/api/matters'),
    apiFetch('/api/matters/active'),
  ]).then(function (results) {
    if (!isCurrentRequest('matters', requestVersion)) return;
    var listData = results[0];
    var activeData = results[1];
    mattersCache = (listData && listData.matters) ? listData.matters : [];
    activeMatterId = (activeData && activeData.matter_id) ? activeData.matter_id : null;
    if (selectedMatterId && !mattersCache.some(function(m) { return m.id === selectedMatterId; })) {
      selectedMatterId = null;
      currentMatterDetailSection = 'overview';
      currentMatterDocuments = [];
      currentMatterTemplates = [];
      currentMatterDashboard = null;
      currentMatterDeadlines = [];
      currentMatterThreads = [];
      currentMatterWork = createEmptyMatterWorkState();
      currentMatterFinance = createEmptyMatterFinanceState();
    }
    renderMatters();
    updateMatterBadge();
    if (selectedMatterId) {
      openMatterDetail(selectedMatterId);
    } else {
      renderMatterDetailPlaceholder('Select a matter to view Overview, Work, and Finance details.');
    }
  }).catch(function (err) {
    if (!isCurrentRequest('matters', requestVersion)) return;
    var list = document.getElementById('matters-list');
    if (list) list.innerHTML = '<div class="empty-state error-state">Failed to load matters: ' + escapeHtml(err.message) + '</div>';
    renderMatterDetailPlaceholder('Failed to load matters.');
  });
}

/** Update the compact matter badge in the tab bar. */
function updateMatterBadge() {
  var badge = document.getElementById('matter-badge');
  var badgeLabel = document.getElementById('matter-badge-label');
  var chatBanner = document.getElementById('chat-active-matter-banner');
  var chatLabel = document.getElementById('chat-active-matter-label');
  if (!badge || !badgeLabel || !chatBanner || !chatLabel) return;

  if (!activeMatterId) {
    badge.classList.add('is-hidden');
    chatBanner.classList.add('is-hidden');
    return;
  }

  var activeMatter = null;
  for (var i = 0; i < mattersCache.length; i++) {
    if (mattersCache[i] && mattersCache[i].id === activeMatterId) {
      activeMatter = mattersCache[i];
      break;
    }
  }

  var badgeText = activeMatterId;
  if (activeMatter && activeMatter.client) {
    badgeText += ' · ' + activeMatter.client;
  }

  badgeLabel.textContent = badgeText;
  chatLabel.textContent = badgeText;
  badge.classList.remove('is-hidden');
  chatBanner.classList.remove('is-hidden');
}

/** Render the matters list and active-bar inside the Matters tab panel. */
function renderMatters() {
  setMattersGroupToggleFromState();
  populateMatterConflictSelector();

  var activeName = document.getElementById('matters-active-name');
  var clearBtn = document.getElementById('matters-clear-btn');
  if (activeName) activeName.textContent = activeMatterId || 'None';
  if (clearBtn) clearBtn.classList.toggle('is-hidden', !activeMatterId);

  var list = document.getElementById('matters-list');
  if (!list) return;

  if (mattersCache.length === 0) {
    list.innerHTML = '<div class="empty-state">No matters found yet. Use + New Matter to start one.</div>';
    return;
  }

  var html = '';
  if (mattersGroupByClient) {
    var groups = buildGroupedMatters();
    for (var g = 0; g < groups.length; g++) {
      var group = groups[g];
      html += '<section class="matter-group">';
      html += '<h5 class="matter-group-title">' + escapeHtml(group.title) + '</h5>';
      for (var j = 0; j < group.items.length; j++) {
        html += renderMatterCardHtml(group.items[j].matter, group.items[j].index);
      }
      html += '</section>';
    }
  } else {
    for (var i = 0; i < mattersCache.length; i++) {
      html += renderMatterCardHtml(mattersCache[i], i);
    }
  }
  list.innerHTML = html;
}

function reviewMatterCreateConflicts() {
  var formData = readMatterCreateFormValues();
  var validationError = validateMatterCreateForm(formData);
  if (validationError) {
    showToast(validationError, 'error');
    return;
  }

  var signature = matterCreateFormSignature(formData);
  setMatterCreateReviewStatus('Running conflict review…', null);
  setMatterCreateReviewBusy(true);

  apiFetch('/api/matters/conflict-check', {
    method: 'POST',
    body: {
      matter_id: formData.matter_id,
      client_names: [formData.client],
      adversary_names: formData.adversaries,
    },
  }).then(function(data) {
    applyMatterCreateReviewResult(data, signature);
    if (data && data.matched) {
      showToast('Potential conflicts found. Review required before creation.', 'error');
    } else {
      showToast('No conflicts detected for intake parties.', 'success');
    }
  }).catch(function(err) {
    var payload = parseErrorPayload(err);
    var message = payload && payload.error ? payload.error : err.message;
    setMatterCreateReviewStatus('Conflict review failed: ' + message, 'error');
    showToast('Conflict review failed: ' + message, 'error');
    syncMatterCreateActionState();
  }).finally(function() {
    setMatterCreateReviewBusy(false);
  });
}

function createMatterFromForm() {
  var formData = readMatterCreateFormValues();
  var validationError = validateMatterCreateForm(formData);
  if (validationError) {
    showToast(validationError, 'error');
    return;
  }
  if (!canCreateMatterFromReview(formData)) {
    showToast('Run conflict review for current form values before creating.', 'error');
    return;
  }

  var body = {
    matter_id: formData.matter_id,
    client: formData.client,
    confidentiality: formData.confidentiality,
    retention: formData.retention,
    team: formData.team,
    adversaries: formData.adversaries,
  };
  if (formData.jurisdiction) body.jurisdiction = formData.jurisdiction;
  if (formData.practice_area) body.practice_area = formData.practice_area;
  if (formData.opened_date) body.opened_date = formData.opened_date;
  if (matterCreateReviewState.matched) {
    body.conflict_decision = getMatterCreateDecision();
    var note = getMatterCreateDecisionNote();
    if (note) body.conflict_note = note;
  }

  setCreateMatterBusy(true);
  apiFetch('/api/matters', {
    method: 'POST',
    body: body,
  }).then(function(data) {
    var createdId = data && data.active_matter_id ? data.active_matter_id : formData.matter_id;
    activeMatterId = createdId;
    selectedMatterId = createdId;
    var form = byId('matters-create-form');
    if (form) form.reset();
    syncMatterSelectOtherFields();
    resetMatterCreateReview('Run conflict review before creating this matter.', null);
    closeMatterCreateModal();
    showToast('Matter created and activated: ' + createdId, 'success');
    loadMatters();
  }).catch(function(err) {
    var payload = parseErrorPayload(err);
    if (payload && payload.conflict_required && Array.isArray(payload.hits)) {
      applyMatterCreateReviewResult(
        {
          matched: true,
          hits: payload.hits,
          checked_parties: matterCreateReviewState.checkedParties,
        },
        matterCreateFormSignature(formData)
      );
      setMatterCreateReviewStatus(
        'Potential conflicts found. Select a decision and re-submit.',
        'warning'
      );
      showToast('Conflict decision required before matter creation.', 'error');
      return;
    }
    if (payload && payload.decision === 'declined') {
      applyMatterCreateReviewResult(
        {
          matched: true,
          hits: Array.isArray(payload.hits) ? payload.hits : matterCreateReviewState.hits,
          checked_parties: matterCreateReviewState.checkedParties,
        },
        matterCreateFormSignature(formData)
      );
      setMatterCreateReviewStatus(
        'Matter creation declined. You can change decision to clear or waived to proceed.',
        'error'
      );
      showToast('Matter creation declined by conflict decision.', 'error');
      return;
    }
    showToast('Failed to create matter: ' + (payload && payload.error ? payload.error : err.message), 'error');
  }).finally(function() {
    setCreateMatterBusy(false);
  });
}

resetMatterCreateReview('Run conflict review before creating this matter.', null);

/**
 * Set the active matter to `id`.
 * @param {string} id - Matter directory name (server sanitizes before storing).
 */
function selectMatter(id) {
  apiFetch('/api/matters/active', {
    method: 'POST',
    body: { matter_id: id },
  }).then(function () {
    activeMatterId = id;
    selectedMatterId = id;
    showToast('Active matter set to ' + id, 'success');
    updateMatterBadge();
    renderMatters();
    openMatterDetail(id);
  }).catch(function (err) {
    showToast('Failed to select matter: ' + err.message, 'error');
  });
}

/** Clear the active matter selection. */
function clearActiveMatter() {
  if (!confirm('Clear active matter?')) return;
  apiFetch('/api/matters/active', {
    method: 'POST',
    body: { matter_id: null },
  }).then(function () {
    activeMatterId = null;
    showToast('Active matter cleared', 'success');
    updateMatterBadge();
    renderMatters();
  }).catch(function (err) {
    showToast('Failed to clear matter: ' + err.message, 'error');
  });
}

/**
 * Jump to the Memory tab with the matter's directory pre-selected.
 * @param {string} id - Matter ID.
 */
function viewMatterInMemory(id) {
  switchTab('memory');
  openMemoryDirectory('matters/' + id);
}

function refreshActiveMatterState() {
  return apiFetch('/api/matters/active').then(function (data) {
    activeMatterId = (data && data.matter_id) ? data.matter_id : null;
    updateMatterBadge();
    populateMatterConflictSelector();
  }).catch(function () {});
}

// Fetch the active matter on startup so the badge appears immediately if set.
(function () {
  refreshActiveMatterState();
}());

