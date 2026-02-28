// cLawyer Web Gateway - Client

let token = '';
let eventSource = null;
let logEventSource = null;
let currentTab = 'chat';
let currentThreadId = null;
let assistantThreadId = null;
let hasMore = false;
let oldestTimestamp = null;
let loadingOlder = false;
let sseHasConnectedBefore = false;
let jobEvents = new Map(); // job_id -> Array of events
let jobListRefreshTimer = null;
const JOB_EVENTS_CAP = 500;
const MEMORY_SEARCH_QUERY_MAX_LENGTH = 100;

function byId(id) {
  return document.getElementById(id);
}

function bindClick(id, handler) {
  const el = byId(id);
  if (el) el.addEventListener('click', handler);
}

function bindChange(id, handler) {
  const el = byId(id);
  if (el) el.addEventListener('change', handler);
}

function bindKeydown(id, handler) {
  const el = byId(id);
  if (el) el.addEventListener('keydown', handler);
}

function delegate(container, eventType, selector, handler) {
  if (!container) return;
  container.addEventListener(eventType, function(event) {
    const target = event.target.closest(selector);
    if (!target || !container.contains(target)) return;
    handler(event, target);
  });
}

const requestVersions = {
  memoryTree: 0,
  memorySearch: 0,
  memoryRead: 0,
  memoryDirectory: 0,
  jobsList: 0,
  jobDetail: 0,
  routinesList: 0,
  routineDetail: 0,
  matters: 0,
  matterDetail: 0,
  legalAudit: 0,
  settings: 0,
  extensions: 0,
  skills: 0,
  gatewayStatus: 0,
};

function beginRequest(key) {
  requestVersions[key] = (requestVersions[key] || 0) + 1;
  return requestVersions[key];
}

function isCurrentRequest(key, version) {
  return requestVersions[key] === version;
}

// --- Tool Activity State ---
let _activeGroup = null;
let _activeToolCards = {};
let _activityThinking = null;

// --- Auth ---

function authenticate() {
  token = document.getElementById('token-input').value.trim();
  if (!token) {
    document.getElementById('auth-error').textContent = 'Token required';
    return;
  }

  // Test the token against the health-ish endpoint (chat/threads requires auth)
  apiFetch('/api/chat/threads')
    .then(() => {
      sessionStorage.setItem('clawyer_token', token);
      document.getElementById('auth-screen').style.display = 'none';
      document.getElementById('app').style.display = 'flex';
      // Strip token and log_level from URL so they're not visible in the address bar
      const cleaned = new URL(window.location);
      const urlLogLevel = cleaned.searchParams.get('log_level');
      cleaned.searchParams.delete('token');
      cleaned.searchParams.delete('log_level');
      window.history.replaceState({}, '', cleaned.pathname + cleaned.search);
      connectSSE();
      connectLogSSE();
      startGatewayStatusPolling();
      checkTeeStatus();
      loadThreads();
      loadMemoryTree();
      loadJobs();
      refreshActiveMatterState();
      // Apply URL log_level param if present, otherwise just sync the dropdown
      if (urlLogLevel) {
        setServerLogLevel(urlLogLevel);
      } else {
        loadServerLogLevel();
      }
    })
    .catch(() => {
      sessionStorage.removeItem('clawyer_token');
      document.getElementById('auth-screen').style.display = '';
      document.getElementById('app').style.display = 'none';
      document.getElementById('auth-error').textContent = 'Invalid token';
    });
}

bindKeydown('token-input', (e) => {
  if (e.key === 'Enter') authenticate();
});
bindClick('auth-connect-btn', authenticate);

// Auto-authenticate from URL param or saved session
(function autoAuth() {
  const params = new URLSearchParams(window.location.search);
  const urlToken = params.get('token');
  if (urlToken) {
    document.getElementById('token-input').value = urlToken;
    authenticate();
    return;
  }
  const saved = sessionStorage.getItem('clawyer_token');
  if (saved) {
    document.getElementById('token-input').value = saved;
    // Hide auth screen immediately to prevent flash, authenticate() will
    // restore it if the token turns out to be invalid.
    document.getElementById('auth-screen').style.display = 'none';
    document.getElementById('app').style.display = 'flex';
    authenticate();
  }
})();

(function bindStaticUiEvents() {
  bindClick('matter-badge', function() { switchTab('matters'); });
  bindClick('thread-new-btn', createNewThread);
  bindClick('thread-toggle-btn', toggleThreadSidebar);
  bindClick('assistant-thread', switchToAssistant);
  bindClick('send-btn', sendMessage);
  bindClick('memory-edit-btn', startMemoryEdit);
  bindClick('memory-upload-btn', triggerMemoryUpload);
  bindClick('memory-save-btn', saveMemoryEdit);
  bindClick('memory-cancel-btn', cancelMemoryEdit);
  bindChange('logs-server-level', function(e) { setServerLogLevel(e.target.value); });
  bindClick('logs-pause-btn', toggleLogsPause);
  bindClick('logs-clear-btn', clearLogs);
  bindClick('wasm-install-btn', installWasmExtension);
  bindClick('mcp-install-btn', addMcpServer);
  bindClick('skill-search-btn', searchClawHub);
  bindClick('skill-install-btn', installSkillFromForm);
  bindClick('matters-clear-btn', clearActiveMatter);
  bindClick('legal-audit-refresh-btn', function() { loadLegalAudit(0); });
  bindClick('legal-audit-prev-btn', function() {
    var next = Math.max(0, legalAuditOffset - legalAuditLimit);
    loadLegalAudit(next);
  });
  bindClick('legal-audit-next-btn', function() {
    if (legalAuditNextOffset == null) return;
    loadLegalAudit(legalAuditNextOffset);
  });
  bindChange('matters-group-by-client', function(e) {
    mattersGroupByClient = !!(e && e.target && e.target.checked);
    localStorage.setItem(MATTERS_GROUP_KEY, mattersGroupByClient ? '1' : '0');
    renderMatters();
  });
  var mattersCreateForm = byId('matters-create-form');
  if (mattersCreateForm) {
    mattersCreateForm.addEventListener('submit', function(e) {
      e.preventDefault();
      createMatterFromForm();
    });
    mattersCreateForm.addEventListener('input', handleMatterCreateFormMutation);
    mattersCreateForm.addEventListener('change', handleMatterCreateFormMutation);
  }
  bindClick('matters-review-btn', function(e) {
    e.preventDefault();
    reviewMatterCreateConflicts();
  });
  bindChange('matter-create-conflict-decision', function() {
    syncMatterCreateActionState();
  });
  var conflictNote = byId('matter-create-conflict-note');
  if (conflictNote) {
    conflictNote.addEventListener('input', function() {
      syncMatterCreateActionState();
    });
  }
  var mattersConflictForm = byId('matters-conflict-form');
  if (mattersConflictForm) {
    mattersConflictForm.addEventListener('submit', function(e) {
      e.preventDefault();
      runMatterConflictCheck();
    });
  }
  delegate(byId('matters-list'), 'click', 'button[data-matter-action]', function(e, button) {
    e.preventDefault();
    var idx = parseInt(button.getAttribute('data-matter-index'), 10);
    if (isNaN(idx) || !mattersCache[idx]) return;
    var matter = mattersCache[idx];
    var action = button.getAttribute('data-matter-action');
    if (action === 'select') {
      selectMatter(matter.id);
      return;
    }
    if (action === 'browse') {
      viewMatterInMemory(matter.id);
      return;
    }
    if (action === 'detail') {
      openMatterDetail(matter.id);
    }
  });
  delegate(byId('matter-detail-panel'), 'click', 'button[data-matter-detail-action]', function(e, button) {
    e.preventDefault();
    var action = button.getAttribute('data-matter-detail-action');
    if (action === 'open-doc') {
      var path = button.getAttribute('data-path');
      if (path) openMatterPathInMemory(path);
      return;
    }
    if (action === 'preview-template') {
      var templatePath = button.getAttribute('data-path');
      if (templatePath) openMatterPathInMemory(templatePath);
      return;
    }
    if (action === 'apply-template') {
      var templateName = button.getAttribute('data-template-name');
      if (templateName) applyMatterTemplate(templateName);
      return;
    }
    if (action === 'build-filing-package') {
      buildMatterFilingPackage();
    }
  });
  delegate(byId('legal-audit-list'), 'click', 'button[data-audit-index]', function(e, button) {
    e.preventDefault();
    var idx = parseInt(button.getAttribute('data-audit-index'), 10);
    if (isNaN(idx) || !legalAuditEvents[idx]) return;
    renderLegalAuditDetail(legalAuditEvents[idx]);
  });
})();

// --- API helper ---

function apiFetch(path, options) {
  const opts = options || {};
  opts.headers = opts.headers || {};
  opts.headers['Authorization'] = 'Bearer ' + token;
  if (opts.body instanceof FormData) {
    // Let the browser set Content-Type + multipart boundary automatically.
    // Do NOT set Content-Type manually or the boundary will be missing.
  } else if (opts.body && typeof opts.body === 'object') {
    opts.headers['Content-Type'] = 'application/json';
    opts.body = JSON.stringify(opts.body);
  }
  return fetch(path, opts).then((res) => {
    if (!res.ok) {
      return res.text().then(function(body) {
        throw new Error(body || (res.status + ' ' + res.statusText));
      });
    }
    if (res.status === 204) return null;
    return res.text().then(function(body) {
      if (!body) return null;
      try {
        return JSON.parse(body);
      } catch (_) {
        return body;
      }
    });
  });
}

// --- SSE ---

function connectSSE() {
  if (eventSource) eventSource.close();

  eventSource = new EventSource('/api/chat/events?token=' + encodeURIComponent(token));

  eventSource.onopen = () => {
    document.getElementById('sse-dot').classList.remove('disconnected');
    document.getElementById('sse-status').textContent = 'Connected';
    if (sseHasConnectedBefore && currentThreadId) {
      finalizeActivityGroup();
      loadHistory();
    }
    sseHasConnectedBefore = true;
  };

  eventSource.onerror = () => {
    document.getElementById('sse-dot').classList.add('disconnected');
    document.getElementById('sse-status').textContent = 'Reconnecting...';
  };

  eventSource.addEventListener('response', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    finalizeActivityGroup();
    addMessage('assistant', data.content);
    setStatus('');
    enableChatInput();
    // Refresh thread list so new titles appear after first message
    loadThreads();
  });

  eventSource.addEventListener('thinking', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    showActivityThinking(data.message);
  });

  eventSource.addEventListener('tool_started', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    addToolCard(data.name);
  });

  eventSource.addEventListener('tool_completed', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    completeToolCard(data.name, data.success);
  });

  eventSource.addEventListener('tool_result', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    setToolCardOutput(data.name, data.preview);
  });

  eventSource.addEventListener('stream_chunk', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    finalizeActivityGroup();
    appendToLastAssistant(data.content);
  });

  eventSource.addEventListener('status', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    setStatus(data.message);
    // "Done" and "Awaiting approval" are terminal signals from the agent:
    // the agentic loop finished, so re-enable input as a safety net in case
    // the response SSE event is empty or lost.
    if (data.message === 'Done' || data.message === 'Awaiting approval') {
      finalizeActivityGroup();
      enableChatInput();
    }
  });

  eventSource.addEventListener('job_started', (e) => {
    const data = JSON.parse(e.data);
    showJobCard(data);
  });

  eventSource.addEventListener('approval_needed', (e) => {
    const data = JSON.parse(e.data);
    if (!isCurrentThread(data.thread_id)) return;
    showApproval(data);
  });

  eventSource.addEventListener('auth_required', (e) => {
    const data = JSON.parse(e.data);
    showAuthCard(data);
  });

  eventSource.addEventListener('auth_completed', (e) => {
    const data = JSON.parse(e.data);
    removeAuthCard(data.extension_name);
    showToast(data.message, 'success');
    enableChatInput();
  });

  eventSource.addEventListener('error', (e) => {
    if (e.data) {
      const data = JSON.parse(e.data);
      if (!isCurrentThread(data.thread_id)) return;
      finalizeActivityGroup();
      addMessage('system', 'Error: ' + data.message);
      enableChatInput();
    }
  });

  // Job event listeners (activity stream for all sandbox jobs)
  const jobEventTypes = [
    'job_message', 'job_tool_use', 'job_tool_result',
    'job_status', 'job_result'
  ];
  for (const evtType of jobEventTypes) {
    eventSource.addEventListener(evtType, (e) => {
      const data = JSON.parse(e.data);
      const jobId = data.job_id;
      if (!jobId) return;
      if (!jobEvents.has(jobId)) jobEvents.set(jobId, []);
      const events = jobEvents.get(jobId);
      events.push({ type: evtType, data: data, ts: Date.now() });
      // Cap per-job events to prevent memory leak
      while (events.length > JOB_EVENTS_CAP) events.shift();
      // If the Activity tab is currently visible for this job, refresh it
      refreshActivityTab(jobId);
      // Auto-refresh job list when on jobs tab (debounced)
      if ((evtType === 'job_result' || evtType === 'job_status') && currentTab === 'jobs' && !currentJobId) {
        clearTimeout(jobListRefreshTimer);
        jobListRefreshTimer = setTimeout(loadJobs, 200);
      }
      // Clean up finished job events after a viewing window
      if (evtType === 'job_result') {
        setTimeout(() => jobEvents.delete(jobId), 60000);
      }
    });
  }
}

// Check if an SSE event belongs to the currently viewed thread.
// Events without a thread_id (legacy) are always shown.
function isCurrentThread(threadId) {
  if (!threadId) return true;
  if (!currentThreadId) return true;
  return threadId === currentThreadId;
}

// --- Chat ---

function sendMessage() {
  const input = document.getElementById('chat-input');
  const sendBtn = document.getElementById('send-btn');
  if (!currentThreadId) {
    console.warn('sendMessage: no thread selected, ignoring');
    setStatus('Waiting for thread to load...');
    return;
  }
  const content = input.value.trim();
  if (!content) return;

  addMessage('user', content);
  input.value = '';
  autoResizeTextarea(input);
  sendBtn.disabled = true;
  input.disabled = true;

  apiFetch('/api/chat/send', {
    method: 'POST',
    body: { content, thread_id: currentThreadId || undefined },
  }).catch((err) => {
    addMessage('system', 'Failed to send: ' + err.message);
    setStatus('');
    enableChatInput();
  });
}

function enableChatInput() {
  // Don't re-enable until a thread is selected (prevents orphan messages)
  if (!currentThreadId) return;
  const input = document.getElementById('chat-input');
  const sendBtn = document.getElementById('send-btn');
  sendBtn.disabled = false;
  input.disabled = false;
  input.focus();
}

function sendApprovalAction(requestId, action) {
  apiFetch('/api/chat/approval', {
    method: 'POST',
    body: { request_id: requestId, action: action, thread_id: currentThreadId },
  }).catch((err) => {
    addMessage('system', 'Failed to send approval: ' + err.message);
  });

  // Disable buttons and show confirmation on the card
  const card = document.querySelector('.approval-card[data-request-id="' + requestId + '"]');
  if (card) {
    const buttons = card.querySelectorAll('.approval-actions button');
    buttons.forEach((btn) => {
      btn.disabled = true;
    });
    const actions = card.querySelector('.approval-actions');
    const label = document.createElement('span');
    label.className = 'approval-resolved';
    const labelText = action === 'approve' ? 'Approved' : action === 'always' ? 'Always approved' : 'Denied';
    label.textContent = labelText;
    actions.appendChild(label);
  }
}

function renderMarkdown(text) {
  if (typeof marked !== 'undefined') {
    let html = marked.parse(text);
    // Sanitize HTML output to prevent XSS from tool output or LLM responses.
    html = sanitizeRenderedHtml(html);
    // Inject copy buttons into <pre> blocks
    html = html.replace(
      /<pre>/g,
      '<pre class="code-block-wrapper"><button class="copy-btn" type="button" data-copy-code="1">Copy</button>'
    );
    return html;
  }
  return escapeHtml(text);
}

// Strip dangerous HTML elements and attributes from rendered markdown.
// This prevents XSS from tool output or prompt injection in LLM responses.
function sanitizeRenderedHtml(html) {
  html = html.replace(/<script\b[^<]*(?:(?!<\/script>)<[^<]*)*<\/script>/gi, '');
  html = html.replace(/<iframe\b[^>]*>[\s\S]*?<\/iframe>/gi, '');
  html = html.replace(/<object\b[^>]*>[\s\S]*?<\/object>/gi, '');
  html = html.replace(/<embed\b[^>]*\/?>/gi, '');
  html = html.replace(/<form\b[^>]*>[\s\S]*?<\/form>/gi, '');
  html = html.replace(/<style\b[^>]*>[\s\S]*?<\/style>/gi, '');
  html = html.replace(/<link\b[^>]*\/?>/gi, '');
  html = html.replace(/<base\b[^>]*\/?>/gi, '');
  html = html.replace(/<meta\b[^>]*\/?>/gi, '');
  // Remove event handler attributes (onclick, onerror, onload, etc.)
  html = html.replace(/\s+on\w+\s*=\s*"[^"]*"/gi, '');
  html = html.replace(/\s+on\w+\s*=\s*'[^']*'/gi, '');
  html = html.replace(/\s+on\w+\s*=\s*[^\s>]+/gi, '');
  // Remove javascript: and data: URLs in href/src attributes
  html = html.replace(/(href|src|action)\s*=\s*["']?\s*javascript\s*:/gi, '$1="');
  html = html.replace(/(href|src|action)\s*=\s*["']?\s*data\s*:/gi, '$1="');
  return html;
}

function copyCodeBlock(btn) {
  const pre = btn.parentElement;
  const code = pre.querySelector('code');
  const text = code ? code.textContent : pre.textContent;
  navigator.clipboard.writeText(text).then(() => {
    btn.textContent = 'Copied!';
    setTimeout(() => { btn.textContent = 'Copy'; }, 1500);
  });
}

function addMessage(role, content) {
  const container = document.getElementById('chat-messages');
  const div = document.createElement('div');
  div.className = 'message ' + role;
  if (role === 'user') {
    div.textContent = content;
  } else {
    div.setAttribute('data-raw', content);
    div.innerHTML = renderMarkdown(content);
  }
  container.appendChild(div);
  container.scrollTop = container.scrollHeight;
}

function appendToLastAssistant(chunk) {
  const container = document.getElementById('chat-messages');
  const messages = container.querySelectorAll('.message.assistant');
  if (messages.length > 0) {
    const last = messages[messages.length - 1];
    const raw = (last.getAttribute('data-raw') || '') + chunk;
    last.setAttribute('data-raw', raw);
    last.innerHTML = renderMarkdown(raw);
    container.scrollTop = container.scrollHeight;
  } else {
    addMessage('assistant', chunk);
  }
}

function setStatus(text) {
  const el = document.getElementById('chat-status');
  if (!text) {
    el.innerHTML = '';
    return;
  }
  el.innerHTML = escapeHtml(text);
}

// --- Inline Tool Activity Cards ---

function getOrCreateActivityGroup() {
  if (_activeGroup) return _activeGroup;
  const container = document.getElementById('chat-messages');
  const group = document.createElement('div');
  group.className = 'activity-group';
  container.appendChild(group);
  container.scrollTop = container.scrollHeight;
  _activeGroup = group;
  _activeToolCards = {};
  return group;
}

function showActivityThinking(message) {
  const group = getOrCreateActivityGroup();
  if (_activityThinking) {
    // Already exists — just update text and un-hide
    _activityThinking.style.display = '';
    _activityThinking.querySelector('.activity-thinking-text').textContent = message;
  } else {
    _activityThinking = document.createElement('div');
    _activityThinking.className = 'activity-thinking';
    _activityThinking.innerHTML =
      '<span class="activity-thinking-dots">'
      + '<span class="activity-thinking-dot"></span>'
      + '<span class="activity-thinking-dot"></span>'
      + '<span class="activity-thinking-dot"></span>'
      + '</span>'
      + '<span class="activity-thinking-text"></span>';
    group.appendChild(_activityThinking);
    _activityThinking.querySelector('.activity-thinking-text').textContent = message;
  }
  const container = document.getElementById('chat-messages');
  container.scrollTop = container.scrollHeight;
}

function removeActivityThinking() {
  if (_activityThinking) {
    _activityThinking.remove();
    _activityThinking = null;
  }
}

function addToolCard(name) {
  // Hide thinking instead of destroying — it may reappear between tool rounds
  if (_activityThinking) _activityThinking.style.display = 'none';
  const group = getOrCreateActivityGroup();

  const card = document.createElement('div');
  card.className = 'activity-tool-card';
  card.setAttribute('data-tool-name', name);
  card.setAttribute('data-status', 'running');

  const header = document.createElement('div');
  header.className = 'activity-tool-header';

  const icon = document.createElement('span');
  icon.className = 'activity-tool-icon';
  icon.innerHTML = '<div class="spinner"></div>';

  const toolName = document.createElement('span');
  toolName.className = 'activity-tool-name';
  toolName.textContent = name;

  const duration = document.createElement('span');
  duration.className = 'activity-tool-duration';
  duration.textContent = '';

  const chevron = document.createElement('span');
  chevron.className = 'activity-tool-chevron';
  chevron.innerHTML = '&#9656;';

  header.appendChild(icon);
  header.appendChild(toolName);
  header.appendChild(duration);
  header.appendChild(chevron);

  const body = document.createElement('div');
  body.className = 'activity-tool-body';
  body.style.display = 'none';

  const output = document.createElement('pre');
  output.className = 'activity-tool-output';
  body.appendChild(output);

  header.addEventListener('click', () => {
    const isOpen = body.style.display !== 'none';
    body.style.display = isOpen ? 'none' : 'block';
    chevron.classList.toggle('expanded', !isOpen);
  });

  card.appendChild(header);
  card.appendChild(body);
  group.appendChild(card);

  const startTime = Date.now();
  const timerInterval = setInterval(() => {
    const elapsed = (Date.now() - startTime) / 1000;
    if (elapsed > 300) { clearInterval(timerInterval); return; }
    duration.textContent = elapsed < 10 ? elapsed.toFixed(1) + 's' : Math.floor(elapsed) + 's';
  }, 100);

  if (!_activeToolCards[name]) _activeToolCards[name] = [];
  _activeToolCards[name].push({ card, startTime, timer: timerInterval, duration, icon, finalDuration: null });

  const container = document.getElementById('chat-messages');
  container.scrollTop = container.scrollHeight;
}

function completeToolCard(name, success) {
  const entries = _activeToolCards[name];
  if (!entries || entries.length === 0) return;
  // Find first running card
  let entry = null;
  for (let i = 0; i < entries.length; i++) {
    if (entries[i].card.getAttribute('data-status') === 'running') {
      entry = entries[i];
      break;
    }
  }
  if (!entry) entry = entries[entries.length - 1];

  clearInterval(entry.timer);
  const elapsed = (Date.now() - entry.startTime) / 1000;
  entry.finalDuration = elapsed;
  entry.duration.textContent = elapsed < 10 ? elapsed.toFixed(1) + 's' : Math.floor(elapsed) + 's';
  entry.icon.innerHTML = success
    ? '<span class="activity-icon-success">&#10003;</span>'
    : '<span class="activity-icon-fail">&#10007;</span>';
  entry.card.setAttribute('data-status', success ? 'success' : 'fail');
}

function setToolCardOutput(name, preview) {
  const entries = _activeToolCards[name];
  if (!entries || entries.length === 0) return;
  // Find first card with empty output
  let entry = null;
  for (let i = 0; i < entries.length; i++) {
    const out = entries[i].card.querySelector('.activity-tool-output');
    if (out && !out.textContent) {
      entry = entries[i];
      break;
    }
  }
  if (!entry) entry = entries[entries.length - 1];

  const output = entry.card.querySelector('.activity-tool-output');
  if (output) {
    const truncated = preview.length > 2000 ? preview.substring(0, 2000) + '\n... (truncated)' : preview;
    output.textContent = truncated;
  }
}

function finalizeActivityGroup() {
  removeActivityThinking();
  if (!_activeGroup) return;

  // Stop all timers
  for (const name in _activeToolCards) {
    const entries = _activeToolCards[name];
    for (let i = 0; i < entries.length; i++) {
      clearInterval(entries[i].timer);
    }
  }

  // Count tools and total duration
  let toolCount = 0;
  let totalDuration = 0;
  for (const tname in _activeToolCards) {
    const tentries = _activeToolCards[tname];
    for (let j = 0; j < tentries.length; j++) {
      const entry = tentries[j];
      toolCount++;
      if (entry.finalDuration !== null) {
        totalDuration += entry.finalDuration;
      } else {
        // Tool was still running when finalized
        totalDuration += (Date.now() - entry.startTime) / 1000;
      }
    }
  }

  if (toolCount === 0) {
    // No tools were used — remove the empty group
    _activeGroup.remove();
    _activeGroup = null;
    _activeToolCards = {};
    return;
  }

  // Wrap existing cards into a hidden container
  const cardsContainer = document.createElement('div');
  cardsContainer.className = 'activity-cards-container';
  cardsContainer.style.display = 'none';

  const cards = _activeGroup.querySelectorAll('.activity-tool-card');
  for (let k = 0; k < cards.length; k++) {
    cardsContainer.appendChild(cards[k]);
  }

  // Build summary line
  const durationStr = totalDuration < 10 ? totalDuration.toFixed(1) + 's' : Math.floor(totalDuration) + 's';
  const toolWord = toolCount === 1 ? 'tool' : 'tools';
  const summary = document.createElement('div');
  summary.className = 'activity-summary';
  summary.innerHTML = '<span class="activity-summary-chevron">&#9656;</span>'
    + '<span class="activity-summary-text">Used ' + toolCount + ' ' + toolWord + '</span>'
    + '<span class="activity-summary-duration">(' + durationStr + ')</span>';

  summary.addEventListener('click', () => {
    const isOpen = cardsContainer.style.display !== 'none';
    cardsContainer.style.display = isOpen ? 'none' : 'block';
    summary.querySelector('.activity-summary-chevron').classList.toggle('expanded', !isOpen);
  });

  // Clear group and add summary + hidden cards
  _activeGroup.innerHTML = '';
  _activeGroup.classList.add('collapsed');
  _activeGroup.appendChild(summary);
  _activeGroup.appendChild(cardsContainer);

  _activeGroup = null;
  _activeToolCards = {};
}

function showApproval(data) {
  const container = document.getElementById('chat-messages');
  const card = document.createElement('div');
  card.className = 'approval-card';
  card.setAttribute('data-request-id', data.request_id);

  const header = document.createElement('div');
  header.className = 'approval-header';
  header.textContent = 'Tool requires approval';
  card.appendChild(header);

  const toolName = document.createElement('div');
  toolName.className = 'approval-tool-name';
  toolName.textContent = data.tool_name;
  card.appendChild(toolName);

  if (data.description) {
    const desc = document.createElement('div');
    desc.className = 'approval-description';
    desc.textContent = data.description;
    card.appendChild(desc);
  }

  if (data.parameters) {
    const paramsToggle = document.createElement('button');
    paramsToggle.className = 'approval-params-toggle';
    paramsToggle.textContent = 'Show parameters';
    const paramsBlock = document.createElement('pre');
    paramsBlock.className = 'approval-params';
    paramsBlock.textContent = data.parameters;
    paramsBlock.style.display = 'none';
    paramsToggle.addEventListener('click', () => {
      const visible = paramsBlock.style.display !== 'none';
      paramsBlock.style.display = visible ? 'none' : 'block';
      paramsToggle.textContent = visible ? 'Show parameters' : 'Hide parameters';
    });
    card.appendChild(paramsToggle);
    card.appendChild(paramsBlock);
  }

  const actions = document.createElement('div');
  actions.className = 'approval-actions';

  const approveBtn = document.createElement('button');
  approveBtn.className = 'approve';
  approveBtn.textContent = 'Approve';
  approveBtn.addEventListener('click', () => sendApprovalAction(data.request_id, 'approve'));

  const alwaysBtn = document.createElement('button');
  alwaysBtn.className = 'always';
  alwaysBtn.textContent = 'Always';
  alwaysBtn.addEventListener('click', () => sendApprovalAction(data.request_id, 'always'));

  const denyBtn = document.createElement('button');
  denyBtn.className = 'deny';
  denyBtn.textContent = 'Deny';
  denyBtn.addEventListener('click', () => sendApprovalAction(data.request_id, 'deny'));

  actions.appendChild(approveBtn);
  actions.appendChild(alwaysBtn);
  actions.appendChild(denyBtn);
  card.appendChild(actions);

  container.appendChild(card);
  container.scrollTop = container.scrollHeight;
}

function showJobCard(data) {
  const container = document.getElementById('chat-messages');
  const card = document.createElement('div');
  card.className = 'job-card';

  const icon = document.createElement('span');
  icon.className = 'job-card-icon';
  icon.textContent = '\u2692';
  card.appendChild(icon);

  const info = document.createElement('div');
  info.className = 'job-card-info';

  const title = document.createElement('div');
  title.className = 'job-card-title';
  title.textContent = data.title || 'Sandbox Job';
  info.appendChild(title);

  const id = document.createElement('div');
  id.className = 'job-card-id';
  id.textContent = (data.job_id || '').substring(0, 8);
  info.appendChild(id);

  card.appendChild(info);

  const viewBtn = document.createElement('button');
  viewBtn.className = 'job-card-view';
  viewBtn.textContent = 'View Job';
  viewBtn.addEventListener('click', () => {
    switchTab('jobs');
    openJobDetail(data.job_id);
  });
  card.appendChild(viewBtn);

  if (data.browse_url) {
    const browseBtn = document.createElement('a');
    browseBtn.className = 'job-card-browse';
    browseBtn.href = data.browse_url;
    browseBtn.target = '_blank';
    browseBtn.textContent = 'Browse';
    card.appendChild(browseBtn);
  }

  container.appendChild(card);
  container.scrollTop = container.scrollHeight;
}

// --- Auth card ---

function showAuthCard(data) {
  // Remove any existing card for this extension first
  removeAuthCard(data.extension_name);

  const container = document.getElementById('chat-messages');
  const card = document.createElement('div');
  card.className = 'auth-card';
  card.setAttribute('data-extension-name', data.extension_name);

  const header = document.createElement('div');
  header.className = 'auth-header';
  header.textContent = 'Authentication required for ' + data.extension_name;
  card.appendChild(header);

  if (data.instructions) {
    const instr = document.createElement('div');
    instr.className = 'auth-instructions';
    instr.textContent = data.instructions;
    card.appendChild(instr);
  }

  const links = document.createElement('div');
  links.className = 'auth-links';

  if (data.auth_url) {
    const oauthBtn = document.createElement('button');
    oauthBtn.className = 'auth-oauth';
    oauthBtn.textContent = 'Authenticate with ' + data.extension_name;
    oauthBtn.addEventListener('click', () => {
      window.open(data.auth_url, '_blank', 'width=600,height=700');
    });
    links.appendChild(oauthBtn);
  }

  if (data.setup_url) {
    const setupLink = document.createElement('a');
    setupLink.href = data.setup_url;
    setupLink.target = '_blank';
    setupLink.textContent = 'Get your token';
    links.appendChild(setupLink);
  }

  if (links.children.length > 0) {
    card.appendChild(links);
  }

  // Token input
  const tokenRow = document.createElement('div');
  tokenRow.className = 'auth-token-input';

  const tokenInput = document.createElement('input');
  tokenInput.type = 'password';
  tokenInput.placeholder = 'Paste your API key or token';
  tokenInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') submitAuthToken(data.extension_name, tokenInput.value);
  });
  tokenRow.appendChild(tokenInput);
  card.appendChild(tokenRow);

  // Error display (hidden initially)
  const errorEl = document.createElement('div');
  errorEl.className = 'auth-error';
  errorEl.style.display = 'none';
  card.appendChild(errorEl);

  // Action buttons
  const actions = document.createElement('div');
  actions.className = 'auth-actions';

  const submitBtn = document.createElement('button');
  submitBtn.className = 'auth-submit';
  submitBtn.textContent = 'Submit';
  submitBtn.addEventListener('click', () => submitAuthToken(data.extension_name, tokenInput.value));

  const cancelBtn = document.createElement('button');
  cancelBtn.className = 'auth-cancel';
  cancelBtn.textContent = 'Cancel';
  cancelBtn.addEventListener('click', () => cancelAuth(data.extension_name));

  actions.appendChild(submitBtn);
  actions.appendChild(cancelBtn);
  card.appendChild(actions);

  container.appendChild(card);
  container.scrollTop = container.scrollHeight;
  tokenInput.focus();
}

function removeAuthCard(extensionName) {
  const card = document.querySelector('.auth-card[data-extension-name="' + extensionName + '"]');
  if (card) card.remove();
}

function submitAuthToken(extensionName, tokenValue) {
  if (!tokenValue || !tokenValue.trim()) return;

  // Disable submit button while in flight
  const card = document.querySelector('.auth-card[data-extension-name="' + extensionName + '"]');
  if (card) {
    const btns = card.querySelectorAll('button');
    btns.forEach((b) => { b.disabled = true; });
  }

  apiFetch('/api/chat/auth-token', {
    method: 'POST',
    body: { extension_name: extensionName, token: tokenValue.trim() },
  }).then((result) => {
    if (result.success) {
      removeAuthCard(extensionName);
      addMessage('system', result.message);
    } else {
      showAuthCardError(extensionName, result.message);
    }
  }).catch((err) => {
    showAuthCardError(extensionName, 'Failed: ' + err.message);
  });
}

function cancelAuth(extensionName) {
  apiFetch('/api/chat/auth-cancel', {
    method: 'POST',
    body: { extension_name: extensionName },
  }).catch(() => {});
  removeAuthCard(extensionName);
  enableChatInput();
}

function showAuthCardError(extensionName, message) {
  const card = document.querySelector('.auth-card[data-extension-name="' + extensionName + '"]');
  if (!card) return;
  // Re-enable buttons
  const btns = card.querySelectorAll('button');
  btns.forEach((b) => { b.disabled = false; });
  // Show error
  const errorEl = card.querySelector('.auth-error');
  if (errorEl) {
    errorEl.textContent = message;
    errorEl.style.display = 'block';
  }
}

function loadHistory(before) {
  let historyUrl = '/api/chat/history?limit=50';
  if (currentThreadId) {
    historyUrl += '&thread_id=' + encodeURIComponent(currentThreadId);
  }
  if (before) {
    historyUrl += '&before=' + encodeURIComponent(before);
  }

  const isPaginating = !!before;
  if (isPaginating) loadingOlder = true;

  apiFetch(historyUrl).then((data) => {
    const container = document.getElementById('chat-messages');

    if (!isPaginating) {
      // Fresh load: clear and render
      container.innerHTML = '';
      for (const turn of data.turns) {
        addMessage('user', turn.user_input);
        if (turn.response) {
          addMessage('assistant', turn.response);
        }
      }
    } else {
      // Pagination: prepend older messages
      const savedHeight = container.scrollHeight;
      const fragment = document.createDocumentFragment();
      for (const turn of data.turns) {
        const userDiv = createMessageElement('user', turn.user_input);
        fragment.appendChild(userDiv);
        if (turn.response) {
          const assistantDiv = createMessageElement('assistant', turn.response);
          fragment.appendChild(assistantDiv);
        }
      }
      container.insertBefore(fragment, container.firstChild);
      // Restore scroll position so the user doesn't jump
      container.scrollTop = container.scrollHeight - savedHeight;
    }

    hasMore = data.has_more || false;
    oldestTimestamp = data.oldest_timestamp || null;
  }).catch(() => {
    // No history or no active thread
  }).finally(() => {
    loadingOlder = false;
    removeScrollSpinner();
  });
}

// Create a message DOM element without appending it (for prepend operations)
function createMessageElement(role, content) {
  const div = document.createElement('div');
  div.className = 'message ' + role;
  if (role === 'user') {
    div.textContent = content;
  } else {
    div.setAttribute('data-raw', content);
    div.innerHTML = renderMarkdown(content);
  }
  return div;
}

function removeScrollSpinner() {
  const spinner = document.getElementById('scroll-load-spinner');
  if (spinner) spinner.remove();
}

// --- Threads ---

function loadThreads() {
  apiFetch('/api/chat/threads').then((data) => {
    // Pinned assistant thread
    if (data.assistant_thread) {
      assistantThreadId = data.assistant_thread.id;
      const el = document.getElementById('assistant-thread');
      const isActive = currentThreadId === assistantThreadId;
      el.className = 'assistant-item' + (isActive ? ' active' : '');
      const meta = document.getElementById('assistant-meta');
      const count = data.assistant_thread.turn_count || 0;
      meta.textContent = count > 0 ? count + ' turns' : '';
    }

    // Regular threads
    const list = document.getElementById('thread-list');
    list.innerHTML = '';
    const threads = data.threads || [];
    for (const thread of threads) {
      const item = document.createElement('div');
      item.className = 'thread-item' + (thread.id === currentThreadId ? ' active' : '');
      const label = document.createElement('span');
      label.className = 'thread-label';
      label.textContent = thread.title || thread.id.substring(0, 8);
      label.title = thread.title ? thread.title + ' (' + thread.id + ')' : thread.id;
      item.appendChild(label);
      const meta = document.createElement('span');
      meta.className = 'thread-meta';
      meta.textContent = (thread.turn_count || 0) + ' turns';
      item.appendChild(meta);
      item.addEventListener('click', () => switchThread(thread.id));
      list.appendChild(item);
    }

    // Default to assistant thread on first load if no thread selected
    if (!currentThreadId && assistantThreadId) {
      switchToAssistant();
    }

    // Enable chat input once a thread is available
    if (currentThreadId) {
      enableChatInput();
    }
  }).catch(() => {});
}

function switchToAssistant() {
  if (!assistantThreadId) return;
  finalizeActivityGroup();
  currentThreadId = assistantThreadId;
  hasMore = false;
  oldestTimestamp = null;
  loadHistory();
  loadThreads();
}

function switchThread(threadId) {
  finalizeActivityGroup();
  currentThreadId = threadId;
  hasMore = false;
  oldestTimestamp = null;
  loadHistory();
  loadThreads();
}

function createNewThread() {
  apiFetch('/api/chat/thread/new', { method: 'POST' }).then((data) => {
    currentThreadId = data.id || null;
    document.getElementById('chat-messages').innerHTML = '';
    setStatus('');
    loadThreads();
  }).catch((err) => {
    showToast('Failed to create thread: ' + err.message, 'error');
  });
}

function toggleThreadSidebar() {
  const sidebar = document.getElementById('thread-sidebar');
  sidebar.classList.toggle('collapsed');
  const btn = document.getElementById('thread-toggle-btn');
  btn.innerHTML = sidebar.classList.contains('collapsed') ? '&raquo;' : '&laquo;';
}

// Chat input auto-resize and keyboard handling
const chatInput = document.getElementById('chat-input');
chatInput.addEventListener('keydown', (e) => {
  if (e.key === 'Enter' && !e.shiftKey) {
    e.preventDefault();
    sendMessage();
  }
});
chatInput.addEventListener('input', () => autoResizeTextarea(chatInput));

// Disable send until a thread is selected (loadThreads will enable it)
chatInput.disabled = true;
document.getElementById('send-btn').disabled = true;

// Infinite scroll: load older messages when scrolled near the top
document.getElementById('chat-messages').addEventListener('scroll', function () {
  if (this.scrollTop < 100 && hasMore && !loadingOlder) {
    loadingOlder = true;
    // Show spinner at top
    const spinner = document.createElement('div');
    spinner.id = 'scroll-load-spinner';
    spinner.className = 'scroll-load-spinner';
    spinner.innerHTML = '<div class="spinner"></div> Loading older messages...';
    this.insertBefore(spinner, this.firstChild);
    loadHistory(oldestTimestamp);
  }
});

delegate(byId('chat-messages'), 'click', 'button[data-copy-code]', function(event, button) {
  event.preventDefault();
  copyCodeBlock(button);
});

function autoResizeTextarea(el) {
  el.style.height = 'auto';
  el.style.height = Math.min(el.scrollHeight, 120) + 'px';
}

// --- Tabs ---

document.querySelectorAll('.tab-bar button[data-tab]').forEach((btn) => {
  btn.addEventListener('click', () => {
    const tab = btn.getAttribute('data-tab');
    switchTab(tab);
  });
});

function switchTab(tab) {
  currentTab = tab;
  document.querySelectorAll('.tab-bar button[data-tab]').forEach((b) => {
    b.classList.toggle('active', b.getAttribute('data-tab') === tab);
  });
  document.querySelectorAll('.tab-panel').forEach((p) => {
    p.classList.toggle('active', p.id === 'tab-' + tab);
  });

  if (tab === 'memory') loadMemoryTree();
  if (tab === 'jobs') loadJobs();
  if (tab === 'routines') loadRoutines();
  if (tab === 'logs') {
    applyLogFilters();
    loadLegalAudit(0);
  }
  if (tab === 'extensions') loadExtensions();
  if (tab === 'skills') loadSkills();
  if (tab === 'matters') loadMatters();
  if (tab === 'settings') loadSettings();
}

// --- Memory (filesystem tree) ---

let memorySearchTimeout = null;
let currentMemoryPath = null;
let currentMemoryContent = null;
// Tree state: nested nodes persisted across renders
// { name, path, is_dir, children: [] | null, expanded: bool, loaded: bool }
let memoryTreeState = null;

document.getElementById('memory-search').addEventListener('input', (e) => {
  clearTimeout(memorySearchTimeout);
  const query = e.target.value.trim();
  if (!query) {
    loadMemoryTree();
    return;
  }
  memorySearchTimeout = setTimeout(() => searchMemory(query), 300);
});

delegate(
  byId('memory-breadcrumb-path'),
  'click',
  'a[data-memory-nav-root],a[data-memory-nav-path]',
  function(event, link) {
    event.preventDefault();
    if (link.hasAttribute('data-memory-nav-root')) {
      loadMemoryTree();
      return;
    }
    const encoded = link.getAttribute('data-memory-nav-path');
    if (!encoded) return;
    readMemoryFile(decodeURIComponent(encoded));
  }
);

function loadMemoryTree() {
  const requestVersion = beginRequest('memoryTree');
  beginRequest('memorySearch');
  // Only load top-level on first load (or refresh)
  apiFetch('/api/memory/list?path=').then((data) => {
    if (!isCurrentRequest('memoryTree', requestVersion)) return;
    memoryTreeState = data.entries.map((e) => ({
      name: e.name,
      path: e.path,
      is_dir: e.is_dir,
      children: e.is_dir ? null : undefined,
      expanded: false,
      loaded: false,
    }));
    renderTree();
  }).catch(() => {
    if (!isCurrentRequest('memoryTree', requestVersion)) return;
  });
}

function renderTree() {
  const container = document.getElementById('memory-tree');
  container.innerHTML = '';
  if (!memoryTreeState || memoryTreeState.length === 0) {
    container.innerHTML = '<div class="tree-item" style="color:var(--text-secondary)">No files in workspace</div>';
    return;
  }
  renderNodes(memoryTreeState, container, 0);
}

function renderNodes(nodes, container, depth) {
  for (const node of nodes) {
    const row = document.createElement('div');
    row.className = 'tree-row';
    row.style.paddingLeft = (depth * 16 + 8) + 'px';

    if (node.is_dir) {
      const arrow = document.createElement('span');
      arrow.className = 'expand-arrow' + (node.expanded ? ' expanded' : '');
      arrow.textContent = '\u25B6';
      arrow.addEventListener('click', (e) => {
        e.stopPropagation();
        toggleExpand(node);
      });
      row.appendChild(arrow);

      const label = document.createElement('span');
      label.className = 'tree-label dir';
      label.textContent = node.name;
      label.addEventListener('click', () => toggleExpand(node));
      row.appendChild(label);
    } else {
      const spacer = document.createElement('span');
      spacer.className = 'expand-arrow-spacer';
      row.appendChild(spacer);

      const label = document.createElement('span');
      label.className = 'tree-label file';
      label.textContent = node.name;
      label.addEventListener('click', () => readMemoryFile(node.path));
      row.appendChild(label);
    }

    container.appendChild(row);

    if (node.is_dir && node.expanded && node.children) {
      const childContainer = document.createElement('div');
      childContainer.className = 'tree-children';
      renderNodes(node.children, childContainer, depth + 1);
      container.appendChild(childContainer);
    }
  }
}

function toggleExpand(node) {
  if (node.expanded) {
    node.expanded = false;
    renderTree();
    return;
  }

  if (node.loaded) {
    node.expanded = true;
    renderTree();
    return;
  }

  // Lazy-load children
  apiFetch('/api/memory/list?path=' + encodeURIComponent(node.path)).then((data) => {
    node.children = data.entries.map((e) => ({
      name: e.name,
      path: e.path,
      is_dir: e.is_dir,
      children: e.is_dir ? null : undefined,
      expanded: false,
      loaded: false,
    }));
    node.loaded = true;
    node.expanded = true;
    renderTree();
  }).catch(() => {});
}

function readMemoryFile(path) {
  const requestVersion = beginRequest('memoryRead');
  beginRequest('memoryDirectory');
  currentMemoryPath = path;
  // Update breadcrumb
  document.getElementById('memory-breadcrumb-path').innerHTML = buildBreadcrumb(path);
  document.getElementById('memory-edit-btn').style.display = 'inline-block';

  // Exit edit mode if active
  cancelMemoryEdit();

  apiFetch('/api/memory/read?path=' + encodeURIComponent(path)).then((data) => {
    if (!isCurrentRequest('memoryRead', requestVersion)) return;
    currentMemoryContent = data.content;
    const viewer = document.getElementById('memory-viewer');
    // Render markdown if it's a .md file
    if (path.endsWith('.md')) {
      viewer.innerHTML = '<div class="memory-rendered">' + renderMarkdown(data.content) + '</div>';
      viewer.classList.add('rendered');
    } else {
      viewer.textContent = data.content;
      viewer.classList.remove('rendered');
    }
  }).catch((err) => {
    if (!isCurrentRequest('memoryRead', requestVersion)) return;
    currentMemoryContent = null;
    document.getElementById('memory-viewer').innerHTML = '<div class="empty">Error: ' + escapeHtml(err.message) + '</div>';
  });
}

function openMemoryDirectory(path) {
  const requestVersion = beginRequest('memoryDirectory');
  beginRequest('memoryRead');
  currentMemoryPath = null;
  currentMemoryContent = null;
  cancelMemoryEdit();
  document.getElementById('memory-edit-btn').style.display = 'none';
  document.getElementById('memory-breadcrumb-path').textContent = 'workspace / ' + path + ' /';

  const viewer = document.getElementById('memory-viewer');
  viewer.classList.remove('rendered');
  viewer.innerHTML = '<div class="empty">Loading directory…</div>';

  apiFetch('/api/memory/list?path=' + encodeURIComponent(path)).then((data) => {
    if (!isCurrentRequest('memoryDirectory', requestVersion)) return;
    const entries = (data && data.entries) ? data.entries : [];
    if (entries.length === 0) {
      viewer.innerHTML = '<div class="empty">No files found in ' + escapeHtml(path) + '.</div>';
      return;
    }

    const container = document.createElement('div');
    const hint = document.createElement('div');
    hint.className = 'empty';
    hint.style.marginBottom = '10px';
    hint.textContent = 'Select a file to view or edit.';
    container.appendChild(hint);

    entries.forEach((entry) => {
      const row = document.createElement('div');
      row.style.marginBottom = '8px';
      const btn = document.createElement('button');
      btn.className = 'btn-ext';
      btn.textContent = entry.is_dir ? '[dir] ' + entry.name : entry.name;
      btn.addEventListener('click', () => {
        if (entry.is_dir) {
          openMemoryDirectory(entry.path);
        } else {
          readMemoryFile(entry.path);
        }
      });
      row.appendChild(btn);
      container.appendChild(row);
    });

    viewer.innerHTML = '';
    viewer.appendChild(container);
  }).catch((err) => {
    if (!isCurrentRequest('memoryDirectory', requestVersion)) return;
    viewer.innerHTML = '<div class="empty">Error: ' + escapeHtml(err.message) + '</div>';
  });
}

function startMemoryEdit() {
  if (!currentMemoryPath || currentMemoryContent === null) return;
  document.getElementById('memory-viewer').style.display = 'none';
  const editor = document.getElementById('memory-editor');
  editor.style.display = 'flex';
  const textarea = document.getElementById('memory-edit-textarea');
  textarea.value = currentMemoryContent;
  textarea.focus();
}

function cancelMemoryEdit() {
  document.getElementById('memory-viewer').style.display = '';
  document.getElementById('memory-editor').style.display = 'none';
}

function saveMemoryEdit() {
  if (!currentMemoryPath) return;
  const content = document.getElementById('memory-edit-textarea').value;
  apiFetch('/api/memory/write', {
    method: 'POST',
    body: { path: currentMemoryPath, content: content },
  }).then(() => {
    showToast('Saved ' + currentMemoryPath, 'success');
    cancelMemoryEdit();
    readMemoryFile(currentMemoryPath);
  }).catch((err) => {
    showToast('Save failed: ' + err.message, 'error');
  });
}

function buildBreadcrumb(path) {
  const parts = path.split('/');
  let html = '<a href="#" data-memory-nav-root="1">workspace</a>';
  let current = '';
  for (const part of parts) {
    current += (current ? '/' : '') + part;
    html += ' / <a href="#" data-memory-nav-path="' + encodeURIComponent(current) + '">'
      + escapeHtml(part) + '</a>';
  }
  return html;
}

function searchMemory(query) {
  const normalizedQuery = normalizeSearchQuery(query);
  if (!normalizedQuery) return;
  const requestVersion = beginRequest('memorySearch');

  apiFetch('/api/memory/search', {
    method: 'POST',
    body: { query: normalizedQuery, limit: 20 },
  }).then((data) => {
    if (!isCurrentRequest('memorySearch', requestVersion)) return;
    const tree = document.getElementById('memory-tree');
    tree.innerHTML = '';
    if (data.results.length === 0) {
      tree.innerHTML = '<div class="tree-item" style="color:var(--text-secondary)">No results</div>';
      return;
    }
    for (const result of data.results) {
      const item = document.createElement('div');
      item.className = 'search-result';
      const snippet = snippetAround(result.content, normalizedQuery, 120);
      item.innerHTML = '<div class="path">' + escapeHtml(result.path) + '</div>'
        + '<div class="snippet">' + highlightQuery(snippet, normalizedQuery) + '</div>';
      item.addEventListener('click', () => readMemoryFile(result.path));
      tree.appendChild(item);
    }
  }).catch(() => {
    if (!isCurrentRequest('memorySearch', requestVersion)) return;
  });
}

function normalizeSearchQuery(query) {
  return (typeof query === 'string' ? query : '').slice(0, MEMORY_SEARCH_QUERY_MAX_LENGTH);
}

function snippetAround(text, query, len) {
  const normalizedQuery = normalizeSearchQuery(query);
  const lower = text.toLowerCase();
  const idx = lower.indexOf(normalizedQuery.toLowerCase());
  if (idx < 0) return text.substring(0, len);
  const start = Math.max(0, idx - Math.floor(len / 2));
  const end = Math.min(text.length, start + len);
  let s = text.substring(start, end);
  if (start > 0) s = '...' + s;
  if (end < text.length) s = s + '...';
  return s;
}

function highlightQuery(text, query) {
  if (!query) return escapeHtml(text);
  const escaped = escapeHtml(text);
  const normalizedQuery = normalizeSearchQuery(query);
  const queryEscaped = normalizedQuery.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const re = new RegExp('(' + queryEscaped + ')', 'gi');
  return escaped.replace(re, '<mark>$1</mark>');
}
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
      list.innerHTML = '<div class="empty-state" style="color:var(--error)">Failed to load legal audit: ' + escapeHtml(err.message) + '</div>';
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

// --- Extensions ---

function loadExtensions() {
  const requestVersion = beginRequest('extensions');
  const extList = document.getElementById('extensions-list');
  const wasmList = document.getElementById('available-wasm-list');
  const mcpList = document.getElementById('mcp-servers-list');
  const toolsTbody = document.getElementById('tools-tbody');
  const toolsEmpty = document.getElementById('tools-empty');

  // Fetch all three in parallel
  Promise.all([
    apiFetch('/api/extensions').catch(() => ({ extensions: [] })),
    apiFetch('/api/extensions/tools').catch(() => ({ tools: [] })),
    apiFetch('/api/extensions/registry').catch(function(err) { console.warn('registry fetch failed:', err); return { entries: [] }; }),
  ]).then(([extData, toolData, registryData]) => {
    if (!isCurrentRequest('extensions', requestVersion)) return;
    // Render installed extensions
    if (extData.extensions.length === 0) {
      extList.innerHTML = '<div class="empty-state">No extensions installed</div>';
    } else {
      extList.innerHTML = '';
      for (const ext of extData.extensions) {
        extList.appendChild(renderExtensionCard(ext));
      }
    }

    // Split registry entries by kind
    var wasmEntries = registryData.entries.filter(function(e) { return e.kind !== 'mcp_server' && !e.installed; });
    var mcpEntries = registryData.entries.filter(function(e) { return e.kind === 'mcp_server'; });

    // Available WASM extensions
    if (wasmEntries.length === 0) {
      wasmList.innerHTML = '<div class="empty-state">No additional WASM extensions available</div>';
    } else {
      wasmList.innerHTML = '';
      for (const entry of wasmEntries) {
        wasmList.appendChild(renderAvailableExtensionCard(entry));
      }
    }

    // MCP servers (show both installed and uninstalled)
    if (mcpEntries.length === 0) {
      mcpList.innerHTML = '<div class="empty-state">No MCP servers available</div>';
    } else {
      mcpList.innerHTML = '';
      for (const entry of mcpEntries) {
        var installedExt = extData.extensions.find(function(e) { return e.name === entry.name; });
        mcpList.appendChild(renderMcpServerCard(entry, installedExt));
      }
    }

    // Render tools
    if (toolData.tools.length === 0) {
      toolsTbody.innerHTML = '';
      toolsEmpty.style.display = 'block';
    } else {
      toolsEmpty.style.display = 'none';
      toolsTbody.innerHTML = toolData.tools.map((t) =>
        '<tr><td>' + escapeHtml(t.name) + '</td><td>' + escapeHtml(t.description) + '</td></tr>'
      ).join('');
    }
  }).catch(() => {
    if (!isCurrentRequest('extensions', requestVersion)) return;
  });
}

function renderAvailableExtensionCard(entry) {
  const card = document.createElement('div');
  card.className = 'ext-card ext-available';

  const header = document.createElement('div');
  header.className = 'ext-header';

  const name = document.createElement('span');
  name.className = 'ext-name';
  name.textContent = entry.display_name;
  header.appendChild(name);

  const kind = document.createElement('span');
  kind.className = 'ext-kind kind-' + entry.kind;
  kind.textContent = entry.kind;
  header.appendChild(kind);

  card.appendChild(header);

  const desc = document.createElement('div');
  desc.className = 'ext-desc';
  desc.textContent = entry.description;
  card.appendChild(desc);

  if (entry.keywords && entry.keywords.length > 0) {
    const kw = document.createElement('div');
    kw.className = 'ext-keywords';
    kw.textContent = entry.keywords.join(', ');
    card.appendChild(kw);
  }

  const actions = document.createElement('div');
  actions.className = 'ext-actions';

  const installBtn = document.createElement('button');
  installBtn.className = 'btn-ext install';
  installBtn.textContent = 'Install';
  installBtn.addEventListener('click', function() {
    installBtn.disabled = true;
    installBtn.textContent = 'Installing...';
    apiFetch('/api/extensions/install', {
      method: 'POST',
      body: { name: entry.name, kind: entry.kind },
    }).then(function(res) {
      if (res.success) {
        showToast('Installed ' + entry.display_name, 'success');
      } else {
        showToast('Install: ' + (res.message || 'unknown error'), 'error');
      }
      loadExtensions();
    }).catch(function(err) {
      showToast('Install failed: ' + err.message, 'error');
      loadExtensions();
    });
  });
  actions.appendChild(installBtn);

  card.appendChild(actions);
  return card;
}

function renderMcpServerCard(entry, installedExt) {
  var card = document.createElement('div');
  card.className = 'ext-card' + (installedExt ? '' : ' ext-available');

  var header = document.createElement('div');
  header.className = 'ext-header';

  var name = document.createElement('span');
  name.className = 'ext-name';
  name.textContent = entry.display_name;
  header.appendChild(name);

  var kind = document.createElement('span');
  kind.className = 'ext-kind kind-mcp_server';
  kind.textContent = 'mcp_server';
  header.appendChild(kind);

  if (installedExt) {
    var authDot = document.createElement('span');
    authDot.className = 'ext-auth-dot ' + (installedExt.authenticated ? 'authed' : 'unauthed');
    authDot.title = installedExt.authenticated ? 'Authenticated' : 'Not authenticated';
    header.appendChild(authDot);
  }

  card.appendChild(header);

  var desc = document.createElement('div');
  desc.className = 'ext-desc';
  desc.textContent = entry.description;
  card.appendChild(desc);

  var actions = document.createElement('div');
  actions.className = 'ext-actions';

  if (installedExt) {
    if (!installedExt.active) {
      var activateBtn = document.createElement('button');
      activateBtn.className = 'btn-ext activate';
      activateBtn.textContent = 'Activate';
      activateBtn.addEventListener('click', function() { activateExtension(installedExt.name); });
      actions.appendChild(activateBtn);
    } else {
      var activeLabel = document.createElement('span');
      activeLabel.className = 'ext-active-label';
      activeLabel.textContent = 'Active';
      actions.appendChild(activeLabel);
    }
    var removeBtn = document.createElement('button');
    removeBtn.className = 'btn-ext remove';
    removeBtn.textContent = 'Remove';
    removeBtn.addEventListener('click', function() { removeExtension(installedExt.name); });
    actions.appendChild(removeBtn);
  } else {
    var installBtn = document.createElement('button');
    installBtn.className = 'btn-ext install';
    installBtn.textContent = 'Install';
    installBtn.addEventListener('click', function() {
      installBtn.disabled = true;
      installBtn.textContent = 'Installing...';
      apiFetch('/api/extensions/install', {
        method: 'POST',
        body: { name: entry.name, kind: entry.kind },
      }).then(function(res) {
        if (res.success) {
          showToast('Installed ' + entry.display_name, 'success');
        } else {
          showToast('Install: ' + (res.message || 'unknown error'), 'error');
        }
        loadExtensions();
      }).catch(function(err) {
        showToast('Install failed: ' + err.message, 'error');
        loadExtensions();
      });
    });
    actions.appendChild(installBtn);
  }

  card.appendChild(actions);
  return card;
}

function renderExtensionCard(ext) {
  const card = document.createElement('div');
  card.className = 'ext-card';

  const header = document.createElement('div');
  header.className = 'ext-header';

  const name = document.createElement('span');
  name.className = 'ext-name';
  name.textContent = ext.name;
  header.appendChild(name);

  const kind = document.createElement('span');
  kind.className = 'ext-kind kind-' + ext.kind;
  kind.textContent = ext.kind;
  header.appendChild(kind);

  const authDot = document.createElement('span');
  authDot.className = 'ext-auth-dot ' + (ext.authenticated ? 'authed' : 'unauthed');
  authDot.title = ext.authenticated ? 'Authenticated' : 'Not authenticated';
  header.appendChild(authDot);

  card.appendChild(header);

  if (ext.description) {
    const desc = document.createElement('div');
    desc.className = 'ext-desc';
    desc.textContent = ext.description;
    card.appendChild(desc);
  }

  if (ext.url) {
    const url = document.createElement('div');
    url.className = 'ext-url';
    url.textContent = ext.url;
    url.title = ext.url;
    card.appendChild(url);
  }

  if (ext.tools.length > 0) {
    const tools = document.createElement('div');
    tools.className = 'ext-tools';
    tools.textContent = 'Tools: ' + ext.tools.join(', ');
    card.appendChild(tools);
  }

  const actions = document.createElement('div');
  actions.className = 'ext-actions';

  if (!ext.active) {
    const activateBtn = document.createElement('button');
    activateBtn.className = 'btn-ext activate';
    activateBtn.textContent = 'Activate';
    activateBtn.addEventListener('click', () => activateExtension(ext.name));
    actions.appendChild(activateBtn);
  } else {
    const activeLabel = document.createElement('span');
    activeLabel.className = 'ext-active-label';
    activeLabel.textContent = 'Active';
    actions.appendChild(activeLabel);
  }

  if (ext.needs_setup) {
    const configBtn = document.createElement('button');
    configBtn.className = 'btn-ext configure';
    configBtn.textContent = ext.authenticated ? 'Reconfigure' : 'Configure';
    configBtn.addEventListener('click', () => showConfigureModal(ext.name));
    actions.appendChild(configBtn);
  }

  const removeBtn = document.createElement('button');
  removeBtn.className = 'btn-ext remove';
  removeBtn.textContent = 'Remove';
  removeBtn.addEventListener('click', () => removeExtension(ext.name));
  actions.appendChild(removeBtn);

  card.appendChild(actions);

  // For WASM channels, check for pending pairing requests.
  // Show even when inactive — pairing requests can arrive via webhooks
  // before the channel is fully activated.
  if (ext.kind === 'wasm_channel') {
    const pairingSection = document.createElement('div');
    pairingSection.className = 'ext-pairing';
    card.appendChild(pairingSection);
    loadPairingRequests(ext.name, pairingSection);
  }

  return card;
}

function activateExtension(name) {
  apiFetch('/api/extensions/' + encodeURIComponent(name) + '/activate', { method: 'POST' })
    .then((res) => {
      if (res.success) {
        loadExtensions();
        return;
      }

      if (res.auth_url) {
        showToast('Opening authentication for ' + name, 'info');
        window.open(res.auth_url, '_blank');
      } else if (res.awaiting_token) {
        showConfigureModal(name);
      } else {
        showToast('Activate failed: ' + res.message, 'error');
      }
      loadExtensions();
    })
    .catch((err) => showToast('Activate failed: ' + err.message, 'error'));
}

function removeExtension(name) {
  if (!confirm('Remove extension "' + name + '"?')) return;
  apiFetch('/api/extensions/' + encodeURIComponent(name) + '/remove', { method: 'POST' })
    .then((res) => {
      if (!res.success) {
        showToast('Remove failed: ' + res.message, 'error');
      } else {
        showToast('Removed ' + name, 'success');
      }
      loadExtensions();
    })
    .catch((err) => showToast('Remove failed: ' + err.message, 'error'));
}

function showConfigureModal(name) {
  apiFetch('/api/extensions/' + encodeURIComponent(name) + '/setup')
    .then((setup) => {
      if (!setup.secrets || setup.secrets.length === 0) {
        showToast('No configuration needed for ' + name, 'info');
        return;
      }
      renderConfigureModal(name, setup.secrets);
    })
    .catch((err) => showToast('Failed to load setup: ' + err.message, 'error'));
}

function renderConfigureModal(name, secrets) {
  closeConfigureModal();
  const overlay = document.createElement('div');
  overlay.className = 'configure-overlay';
  overlay.addEventListener('click', (e) => {
    if (e.target === overlay) closeConfigureModal();
  });

  const modal = document.createElement('div');
  modal.className = 'configure-modal';

  const header = document.createElement('h3');
  header.textContent = 'Configure ' + name;
  modal.appendChild(header);

  const form = document.createElement('div');
  form.className = 'configure-form';

  const fields = [];
  for (const secret of secrets) {
    const field = document.createElement('div');
    field.className = 'configure-field';

    const label = document.createElement('label');
    label.textContent = secret.prompt;
    if (secret.optional) {
      const opt = document.createElement('span');
      opt.className = 'field-optional';
      opt.textContent = ' (optional)';
      label.appendChild(opt);
    }
    field.appendChild(label);

    const inputRow = document.createElement('div');
    inputRow.className = 'configure-input-row';

    const input = document.createElement('input');
    input.type = 'password';
    input.name = secret.name;
    input.placeholder = secret.provided ? '(already set — leave empty to keep)' : '';
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') submitConfigureModal(name, fields);
    });
    inputRow.appendChild(input);

    if (secret.provided) {
      const badge = document.createElement('span');
      badge.className = 'field-provided';
      badge.textContent = 'Set';
      inputRow.appendChild(badge);
    }
    if (secret.auto_generate && !secret.provided) {
      const hint = document.createElement('span');
      hint.className = 'field-autogen';
      hint.textContent = 'Auto-generated if empty';
      inputRow.appendChild(hint);
    }

    field.appendChild(inputRow);
    form.appendChild(field);
    fields.push({ name: secret.name, input: input });
  }

  modal.appendChild(form);

  const actions = document.createElement('div');
  actions.className = 'configure-actions';

  const submitBtn = document.createElement('button');
  submitBtn.className = 'btn-ext activate';
  submitBtn.textContent = 'Save';
  submitBtn.addEventListener('click', () => submitConfigureModal(name, fields));
  actions.appendChild(submitBtn);

  const cancelBtn = document.createElement('button');
  cancelBtn.className = 'btn-ext remove';
  cancelBtn.textContent = 'Cancel';
  cancelBtn.addEventListener('click', closeConfigureModal);
  actions.appendChild(cancelBtn);

  modal.appendChild(actions);
  overlay.appendChild(modal);
  document.body.appendChild(overlay);

  if (fields.length > 0) fields[0].input.focus();
}

function submitConfigureModal(name, fields) {
  const secrets = {};
  for (const f of fields) {
    if (f.input.value.trim()) {
      secrets[f.name] = f.input.value.trim();
    }
  }

  apiFetch('/api/extensions/' + encodeURIComponent(name) + '/setup', {
    method: 'POST',
    body: { secrets },
  })
    .then((res) => {
      closeConfigureModal();
      if (res.success) {
        showToast(res.message, 'success');
      } else {
        showToast(res.message || 'Configuration failed', 'error');
      }
      loadExtensions();
    })
    .catch((err) => {
      showToast('Configuration failed: ' + err.message, 'error');
    });
}

function closeConfigureModal() {
  const existing = document.querySelector('.configure-overlay');
  if (existing) existing.remove();
}

// --- Pairing ---

function loadPairingRequests(channel, container) {
  apiFetch('/api/pairing/' + encodeURIComponent(channel))
    .then(data => {
      container.innerHTML = '';
      if (!data.requests || data.requests.length === 0) return;

      const heading = document.createElement('div');
      heading.className = 'pairing-heading';
      heading.textContent = 'Pending pairing requests';
      container.appendChild(heading);

      data.requests.forEach(req => {
        const row = document.createElement('div');
        row.className = 'pairing-row';

        const code = document.createElement('span');
        code.className = 'pairing-code';
        code.textContent = req.code;
        row.appendChild(code);

        const sender = document.createElement('span');
        sender.className = 'pairing-sender';
        sender.textContent = 'from ' + req.sender_id;
        row.appendChild(sender);

        const btn = document.createElement('button');
        btn.className = 'btn-ext activate';
        btn.textContent = 'Approve';
        btn.addEventListener('click', () => approvePairing(channel, req.code, container));
        row.appendChild(btn);

        container.appendChild(row);
      });
    })
    .catch(() => {});
}

function approvePairing(channel, code, container) {
  apiFetch('/api/pairing/' + encodeURIComponent(channel) + '/approve', {
    method: 'POST',
    body: { code },
  }).then(res => {
    if (res.success) {
      showToast('Pairing approved', 'success');
      loadPairingRequests(channel, container);
    } else {
      showToast(res.message || 'Approve failed', 'error');
    }
  }).catch(err => showToast('Error: ' + err.message, 'error'));
}

// --- Jobs ---

let currentJobId = null;
let currentJobSubTab = 'overview';
let jobFilesTreeState = null;

delegate(byId('jobs-tbody'), 'click', 'button[data-job-action]', function(event, button) {
  event.preventDefault();
  const action = button.getAttribute('data-job-action');
  const jobId = button.getAttribute('data-job-id');
  if (!jobId) return;
  if (action === 'cancel') {
    cancelJob(jobId);
    return;
  }
  if (action === 'restart') {
    restartJob(jobId);
  }
});

delegate(byId('jobs-tbody'), 'click', 'tr.job-row[data-job-id]', function(event, row) {
  if (event.target.closest('button[data-job-action]')) return;
  const jobId = row.getAttribute('data-job-id');
  if (jobId) openJobDetail(jobId);
});

delegate(document.querySelector('.jobs-container'), 'click', 'button[data-job-detail-action]', function(event, button) {
  event.preventDefault();
  const action = button.getAttribute('data-job-detail-action');
  if (action === 'back') {
    closeJobDetail();
    return;
  }
  if (action === 'restart') {
    const jobId = button.getAttribute('data-job-id');
    if (jobId) restartJob(jobId);
  }
});

function loadJobs() {
  const requestVersion = beginRequest('jobsList');
  beginRequest('jobDetail');
  currentJobId = null;
  jobFilesTreeState = null;

  // Rebuild DOM if renderJobDetail() destroyed it (it wipes .jobs-container innerHTML).
  const container = document.querySelector('.jobs-container');
  if (!document.getElementById('jobs-summary')) {
    container.innerHTML =
      '<div class="jobs-summary" id="jobs-summary"></div>'
      + '<table class="jobs-table" id="jobs-table"><thead><tr>'
      + '<th>ID</th><th>Title</th><th>Status</th><th>Created</th><th>Actions</th>'
      + '</tr></thead><tbody id="jobs-tbody"></tbody></table>'
      + '<div class="empty-state" id="jobs-empty" style="display:none">No jobs found</div>';
  }

  Promise.all([
    apiFetch('/api/jobs/summary'),
    apiFetch('/api/jobs'),
  ]).then(([summary, jobList]) => {
    if (!isCurrentRequest('jobsList', requestVersion)) return;
    renderJobsSummary(summary);
    renderJobsList(jobList.jobs);
  }).catch(() => {
    if (!isCurrentRequest('jobsList', requestVersion)) return;
  });
}

function renderJobsSummary(s) {
  document.getElementById('jobs-summary').innerHTML = ''
    + summaryCard('Total', s.total, '')
    + summaryCard('In Progress', s.in_progress, 'active')
    + summaryCard('Completed', s.completed, 'completed')
    + summaryCard('Failed', s.failed, 'failed')
    + summaryCard('Stuck', s.stuck, 'stuck');
}

function summaryCard(label, count, cls) {
  return '<div class="summary-card ' + cls + '">'
    + '<div class="count">' + count + '</div>'
    + '<div class="label">' + label + '</div>'
    + '</div>';
}

function renderJobsList(jobs) {
  const tbody = document.getElementById('jobs-tbody');
  const empty = document.getElementById('jobs-empty');

  if (jobs.length === 0) {
    tbody.innerHTML = '';
    empty.style.display = 'block';
    return;
  }

  empty.style.display = 'none';
  tbody.innerHTML = jobs.map((job) => {
    const shortId = job.id.substring(0, 8);
    const stateClass = job.state.replace(' ', '_');
    const escapedId = escapeHtml(job.id);

    let actionBtns = '';
    if (job.state === 'pending' || job.state === 'in_progress') {
      actionBtns = '<button class="btn-cancel" type="button" data-job-action="cancel" data-job-id="' + escapedId + '">Cancel</button>';
    } else if (job.state === 'failed' || job.state === 'interrupted') {
      actionBtns = '<button class="btn-restart" type="button" data-job-action="restart" data-job-id="' + escapedId + '">Restart</button>';
    }

    return '<tr class="job-row" data-job-id="' + escapedId + '">'
      + '<td title="' + escapedId + '">' + shortId + '</td>'
      + '<td>' + escapeHtml(job.title) + '</td>'
      + '<td><span class="badge ' + stateClass + '">' + escapeHtml(job.state) + '</span></td>'
      + '<td>' + formatDate(job.created_at) + '</td>'
      + '<td>' + actionBtns + '</td>'
      + '</tr>';
  }).join('');
}

function cancelJob(jobId) {
  if (!confirm('Cancel this job?')) return;
  apiFetch('/api/jobs/' + jobId + '/cancel', { method: 'POST' })
    .then(() => {
      showToast('Job cancelled', 'success');
      if (currentJobId) openJobDetail(currentJobId);
      else loadJobs();
    })
    .catch((err) => {
      showToast('Failed to cancel job: ' + err.message, 'error');
    });
}

function restartJob(jobId) {
  apiFetch('/api/jobs/' + jobId + '/restart', { method: 'POST' })
    .then((res) => {
      showToast('Job restarted as ' + (res.new_job_id || '').substring(0, 8), 'success');
      loadJobs();
    })
    .catch((err) => {
      showToast('Failed to restart job: ' + err.message, 'error');
    });
}

function openJobDetail(jobId) {
  const requestVersion = beginRequest('jobDetail');
  currentJobId = jobId;
  currentJobSubTab = 'activity';
  apiFetch('/api/jobs/' + jobId).then((job) => {
    if (!isCurrentRequest('jobDetail', requestVersion)) return;
    renderJobDetail(job);
  }).catch((err) => {
    if (!isCurrentRequest('jobDetail', requestVersion)) return;
    addMessage('system', 'Failed to load job: ' + err.message);
    closeJobDetail();
  });
}

function closeJobDetail() {
  currentJobId = null;
  jobFilesTreeState = null;
  loadJobs();
}

function renderJobDetail(job) {
  const container = document.querySelector('.jobs-container');
  const stateClass = job.state.replace(' ', '_');

  container.innerHTML = '';

  // Header
  const header = document.createElement('div');
  header.className = 'job-detail-header';

  let headerHtml = '<button class="btn-back" type="button" data-job-detail-action="back">&larr; Back</button>'
    + '<h2>' + escapeHtml(job.title) + '</h2>'
    + '<span class="badge ' + stateClass + '">' + escapeHtml(job.state) + '</span>';

  if (job.state === 'failed' || job.state === 'interrupted') {
    headerHtml += '<button class="btn-restart" type="button" data-job-detail-action="restart" data-job-id="' + escapeHtml(job.id) + '">Restart</button>';
  }
  if (job.browse_url) {
    headerHtml += '<a class="btn-browse" href="' + escapeHtml(job.browse_url) + '" target="_blank">Browse Files</a>';
  }

  header.innerHTML = headerHtml;
  container.appendChild(header);

  // Sub-tab bar
  const tabs = document.createElement('div');
  tabs.className = 'job-detail-tabs';
  const subtabs = ['overview', 'activity', 'files'];
  for (const st of subtabs) {
    const btn = document.createElement('button');
    btn.textContent = st.charAt(0).toUpperCase() + st.slice(1);
    btn.className = st === currentJobSubTab ? 'active' : '';
    btn.addEventListener('click', () => {
      currentJobSubTab = st;
      renderJobDetail(job);
    });
    tabs.appendChild(btn);
  }
  container.appendChild(tabs);

  // Content
  const content = document.createElement('div');
  content.className = 'job-detail-content';
  container.appendChild(content);

  switch (currentJobSubTab) {
    case 'overview': renderJobOverview(content, job); break;
    case 'files': renderJobFiles(content, job); break;
    case 'activity': renderJobActivity(content, job); break;
  }
}

function metaItem(label, value) {
  return '<div class="meta-item"><div class="meta-label">' + escapeHtml(label)
    + '</div><div class="meta-value">' + escapeHtml(String(value != null ? value : '-'))
    + '</div></div>';
}

function formatDuration(secs) {
  if (secs == null) return '-';
  if (secs < 60) return secs + 's';
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  if (m < 60) return m + 'm ' + s + 's';
  const h = Math.floor(m / 60);
  return h + 'h ' + (m % 60) + 'm';
}

function renderJobOverview(container, job) {
  // Metadata grid
  const grid = document.createElement('div');
  grid.className = 'job-meta-grid';
  grid.innerHTML = metaItem('Job ID', job.id)
    + metaItem('State', job.state)
    + metaItem('Created', formatDate(job.created_at))
    + metaItem('Started', formatDate(job.started_at))
    + metaItem('Completed', formatDate(job.completed_at))
    + metaItem('Duration', formatDuration(job.elapsed_secs))
    + (job.job_mode ? metaItem('Mode', job.job_mode) : '');
  container.appendChild(grid);

  // Description
  if (job.description) {
    const descSection = document.createElement('div');
    descSection.className = 'job-description';
    const descHeader = document.createElement('h3');
    descHeader.textContent = 'Description';
    descSection.appendChild(descHeader);
    const descBody = document.createElement('div');
    descBody.className = 'job-description-body';
    descBody.innerHTML = renderMarkdown(job.description);
    descSection.appendChild(descBody);
    container.appendChild(descSection);
  }

  // State transitions timeline
  if (job.transitions.length > 0) {
    const timelineSection = document.createElement('div');
    timelineSection.className = 'job-timeline-section';
    const tlHeader = document.createElement('h3');
    tlHeader.textContent = 'State Transitions';
    timelineSection.appendChild(tlHeader);

    const timeline = document.createElement('div');
    timeline.className = 'timeline';
    for (const t of job.transitions) {
      const entry = document.createElement('div');
      entry.className = 'timeline-entry';
      const dot = document.createElement('div');
      dot.className = 'timeline-dot';
      entry.appendChild(dot);
      const info = document.createElement('div');
      info.className = 'timeline-info';
      info.innerHTML = '<span class="badge ' + t.from.replace(' ', '_') + '">' + escapeHtml(t.from) + '</span>'
        + ' &rarr; '
        + '<span class="badge ' + t.to.replace(' ', '_') + '">' + escapeHtml(t.to) + '</span>'
        + '<span class="timeline-time">' + formatDate(t.timestamp) + '</span>'
        + (t.reason ? '<div class="timeline-reason">' + escapeHtml(t.reason) + '</div>' : '');
      entry.appendChild(info);
      timeline.appendChild(entry);
    }
    timelineSection.appendChild(timeline);
    container.appendChild(timelineSection);
  }
}

function renderJobFiles(container, job) {
  container.innerHTML = '<div class="job-files">'
    + '<div class="job-files-sidebar"><div class="job-files-tree"></div></div>'
    + '<div class="job-files-viewer"><div class="empty-state">Select a file to view</div></div>'
    + '</div>';

  container._jobId = job ? job.id : null;

  apiFetch('/api/jobs/' + job.id + '/files/list?path=').then((data) => {
    jobFilesTreeState = data.entries.map((e) => ({
      name: e.name,
      path: e.path,
      is_dir: e.is_dir,
      children: e.is_dir ? null : undefined,
      expanded: false,
      loaded: false,
    }));
    renderJobFilesTree();
  }).catch(() => {
    const treeContainer = document.querySelector('.job-files-tree');
    if (treeContainer) {
      treeContainer.innerHTML = '<div class="tree-item" style="color:var(--text-secondary)">No project files</div>';
    }
  });
}

function renderJobFilesTree() {
  const treeContainer = document.querySelector('.job-files-tree');
  if (!treeContainer) return;
  treeContainer.innerHTML = '';
  if (!jobFilesTreeState || jobFilesTreeState.length === 0) {
    treeContainer.innerHTML = '<div class="tree-item" style="color:var(--text-secondary)">No files in workspace</div>';
    return;
  }
  renderJobFileNodes(jobFilesTreeState, treeContainer, 0);
}

function renderJobFileNodes(nodes, container, depth) {
  for (const node of nodes) {
    const row = document.createElement('div');
    row.className = 'tree-row';
    row.style.paddingLeft = (depth * 16 + 8) + 'px';

    if (node.is_dir) {
      const arrow = document.createElement('span');
      arrow.className = 'expand-arrow' + (node.expanded ? ' expanded' : '');
      arrow.textContent = '\u25B6';
      arrow.addEventListener('click', (e) => {
        e.stopPropagation();
        toggleJobFileExpand(node);
      });
      row.appendChild(arrow);

      const label = document.createElement('span');
      label.className = 'tree-label dir';
      label.textContent = node.name;
      label.addEventListener('click', () => toggleJobFileExpand(node));
      row.appendChild(label);
    } else {
      const spacer = document.createElement('span');
      spacer.className = 'expand-arrow-spacer';
      row.appendChild(spacer);

      const label = document.createElement('span');
      label.className = 'tree-label file';
      label.textContent = node.name;
      label.addEventListener('click', () => readJobFile(node.path));
      row.appendChild(label);
    }

    container.appendChild(row);

    if (node.is_dir && node.expanded && node.children) {
      const childContainer = document.createElement('div');
      childContainer.className = 'tree-children';
      renderJobFileNodes(node.children, childContainer, depth + 1);
      container.appendChild(childContainer);
    }
  }
}

function getJobId() {
  const container = document.querySelector('.job-detail-content');
  return (container && container._jobId) || null;
}

function toggleJobFileExpand(node) {
  if (node.expanded) {
    node.expanded = false;
    renderJobFilesTree();
    return;
  }
  if (node.loaded) {
    node.expanded = true;
    renderJobFilesTree();
    return;
  }
  const jobId = getJobId();
  apiFetch('/api/jobs/' + jobId + '/files/list?path=' + encodeURIComponent(node.path)).then((data) => {
    node.children = data.entries.map((e) => ({
      name: e.name,
      path: e.path,
      is_dir: e.is_dir,
      children: e.is_dir ? null : undefined,
      expanded: false,
      loaded: false,
    }));
    node.loaded = true;
    node.expanded = true;
    renderJobFilesTree();
  }).catch(() => {});
}

function readJobFile(path) {
  const viewer = document.querySelector('.job-files-viewer');
  if (!viewer) return;
  const jobId = getJobId();
  apiFetch('/api/jobs/' + jobId + '/files/read?path=' + encodeURIComponent(path)).then((data) => {
    viewer.innerHTML = '<div class="job-files-path">' + escapeHtml(path) + '</div>'
      + '<pre class="job-files-content">' + escapeHtml(data.content) + '</pre>';
  }).catch((err) => {
    viewer.innerHTML = '<div class="empty-state">Error: ' + escapeHtml(err.message) + '</div>';
  });
}

// --- Activity tab (unified for all sandbox jobs) ---

let activityCurrentJobId = null;
// Track how many live SSE events we've already rendered so refreshActivityTab
// only appends new ones (avoids duplicates on each SSE tick).
let activityRenderedLiveIndex = 0;

function renderJobActivity(container, job) {
  activityCurrentJobId = job ? job.id : null;
  activityRenderedLiveIndex = 0;

  container.innerHTML = '<div class="activity-toolbar">'
    + '<select id="activity-type-filter">'
    + '<option value="all">All Events</option>'
    + '<option value="message">Messages</option>'
    + '<option value="tool_use">Tool Calls</option>'
    + '<option value="tool_result">Results</option>'
    + '</select>'
    + '<label class="logs-checkbox"><input type="checkbox" id="activity-autoscroll" checked> Auto-scroll</label>'
    + '</div>'
    + '<div class="activity-terminal" id="activity-terminal"></div>'
    + '<div class="activity-input-bar" id="activity-input-bar">'
    + '<input type="text" id="activity-prompt-input" placeholder="Send follow-up prompt..." />'
    + '<button id="activity-send-btn">Send</button>'
    + '<button id="activity-done-btn" title="Signal done">Done</button>'
    + '</div>';

  document.getElementById('activity-type-filter').addEventListener('change', applyActivityFilter);

  const terminal = document.getElementById('activity-terminal');
  const input = document.getElementById('activity-prompt-input');
  const sendBtn = document.getElementById('activity-send-btn');
  const doneBtn = document.getElementById('activity-done-btn');

  sendBtn.addEventListener('click', () => sendJobPrompt(job.id, false));
  doneBtn.addEventListener('click', () => sendJobPrompt(job.id, true));
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') sendJobPrompt(job.id, false);
  });

  // Load persisted events from DB, then catch up with any live SSE events
  apiFetch('/api/jobs/' + job.id + '/events').then((data) => {
    if (data.events && data.events.length > 0) {
      for (const evt of data.events) {
        appendActivityEvent(terminal, evt.event_type, evt.data);
      }
    }
    appendNewLiveEvents(terminal, job.id);
  }).catch(() => {
    appendNewLiveEvents(terminal, job.id);
  });
}

function appendNewLiveEvents(terminal, jobId) {
  const live = jobEvents.get(jobId) || [];
  for (let i = activityRenderedLiveIndex; i < live.length; i++) {
    const evt = live[i];
    appendActivityEvent(terminal, evt.type.replace('job_', ''), evt.data);
  }
  activityRenderedLiveIndex = live.length;
  const autoScroll = document.getElementById('activity-autoscroll');
  if (!autoScroll || autoScroll.checked) {
    terminal.scrollTop = terminal.scrollHeight;
  }
}

function applyActivityFilter() {
  const filter = document.getElementById('activity-type-filter').value;
  const events = document.querySelectorAll('#activity-terminal .activity-event');
  for (const el of events) {
    if (filter === 'all') {
      el.style.display = '';
    } else {
      el.style.display = el.getAttribute('data-event-type') === filter ? '' : 'none';
    }
  }
}

function appendActivityEvent(terminal, eventType, data) {
  if (!terminal) return;
  const el = document.createElement('div');
  el.className = 'activity-event activity-event-' + eventType;
  el.setAttribute('data-event-type', eventType);

  // Respect current filter
  const filterEl = document.getElementById('activity-type-filter');
  if (filterEl && filterEl.value !== 'all' && filterEl.value !== eventType) {
    el.style.display = 'none';
  }

  switch (eventType) {
    case 'message':
      el.innerHTML = '<span class="activity-role">' + escapeHtml(data.role || 'assistant') + '</span> '
        + '<span class="activity-content">' + escapeHtml(data.content || '') + '</span>';
      break;
    case 'tool_use':
      el.innerHTML = '<details class="activity-tool-block"><summary>'
        + '<span class="activity-tool-icon">&#9881;</span> '
        + escapeHtml(data.tool_name || 'tool')
        + '</summary><pre class="activity-tool-input">'
        + escapeHtml(typeof data.input === 'string' ? data.input : JSON.stringify(data.input, null, 2))
        + '</pre></details>';
      break;
    case 'tool_result': {
      const trSuccess = data.success !== false;
      const trIcon = trSuccess ? '&#10003;' : '&#10007;';
      const trOutput = data.output || data.error || '';
      const trClass = 'activity-tool-block activity-tool-result'
        + (trSuccess ? '' : ' activity-tool-error');
      el.innerHTML = '<details class="' + trClass + '"><summary>'
        + '<span class="activity-tool-icon">' + trIcon + '</span> '
        + escapeHtml(data.tool_name || 'result')
        + '</summary><pre class="activity-tool-output">'
        + escapeHtml(trOutput)
        + '</pre></details>';
      break;
    }
    case 'status':
      el.innerHTML = '<span class="activity-status">' + escapeHtml(data.message || '') + '</span>';
      break;
    case 'result':
      el.className += ' activity-final';
      const success = data.success !== false;
      el.innerHTML = '<span class="activity-result-status" data-success="' + success + '">'
        + escapeHtml(data.message || data.error || data.status || 'done') + '</span>';
      if (data.session_id) {
        el.innerHTML += ' <span class="activity-session-id">session: ' + escapeHtml(data.session_id) + '</span>';
      }
      break;
    default:
      el.innerHTML = '<span class="activity-status">' + escapeHtml(JSON.stringify(data)) + '</span>';
  }

  terminal.appendChild(el);
}

function refreshActivityTab(jobId) {
  if (activityCurrentJobId !== jobId) return;
  if (currentJobSubTab !== 'activity') return;
  const terminal = document.getElementById('activity-terminal');
  if (!terminal) return;
  appendNewLiveEvents(terminal, jobId);
}

function sendJobPrompt(jobId, done) {
  const input = document.getElementById('activity-prompt-input');
  const content = input ? input.value.trim() : '';
  if (!content && !done) return;

  apiFetch('/api/jobs/' + jobId + '/prompt', {
    method: 'POST',
    body: { content: content || '(done)', done: done },
  }).then(() => {
    if (input) input.value = '';
    if (done) {
      const bar = document.getElementById('activity-input-bar');
      if (bar) bar.innerHTML = '<span class="activity-status">Done signal sent</span>';
    }
  }).catch((err) => {
    const terminal = document.getElementById('activity-terminal');
    if (terminal) {
      appendActivityEvent(terminal, 'status', { message: 'Failed to send: ' + err.message });
    }
  });
}

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

// --- Gateway status widget ---

let gatewayStatusInterval = null;

function startGatewayStatusPolling() {
  fetchGatewayStatus();
  gatewayStatusInterval = setInterval(fetchGatewayStatus, 30000);
}

function formatTokenCount(n) {
  if (n == null || n === 0) return '0';
  if (n >= 1000000) return (n / 1000000).toFixed(1) + 'M';
  if (n >= 1000) return (n / 1000).toFixed(1) + 'k';
  return '' + n;
}

function formatCost(costStr) {
  if (!costStr) return '$0.00';
  var n = parseFloat(costStr);
  if (n < 0.01) return '$' + n.toFixed(4);
  return '$' + n.toFixed(2);
}

function shortModelName(model) {
  // Strip provider prefix and shorten common model names
  var m = model.indexOf('/') >= 0 ? model.split('/').pop() : model;
  // Shorten dated suffixes
  m = m.replace(/-20\d{6}$/, '');
  return m;
}

function fetchGatewayStatus() {
  const requestVersion = beginRequest('gatewayStatus');
  apiFetch('/api/gateway/status').then(function(data) {
    if (!isCurrentRequest('gatewayStatus', requestVersion)) return;
    var popover = document.getElementById('gateway-popover');
    var html = '';

    // Connection info
    html += '<div class="gw-section-label">Connections</div>';
    html += '<div class="gw-stat"><span>SSE</span><span>' + (data.sse_connections || 0) + '</span></div>';
    html += '<div class="gw-stat"><span>WebSocket</span><span>' + (data.ws_connections || 0) + '</span></div>';
    html += '<div class="gw-stat"><span>Uptime</span><span>' + formatDuration(data.uptime_secs) + '</span></div>';

    // Cost tracker
    if (data.daily_cost != null) {
      html += '<div class="gw-divider"></div>';
      html += '<div class="gw-section-label">Cost Today</div>';
      html += '<div class="gw-stat"><span>Spent</span><span>' + formatCost(data.daily_cost) + '</span></div>';
      if (data.actions_this_hour != null) {
        html += '<div class="gw-stat"><span>Actions/hr</span><span>' + data.actions_this_hour + '</span></div>';
      }
    }

    // Per-model token usage
    if (data.model_usage && data.model_usage.length > 0) {
      html += '<div class="gw-divider"></div>';
      html += '<div class="gw-section-label">Token Usage</div>';
      data.model_usage.sort(function(a, b) {
        return (b.input_tokens + b.output_tokens) - (a.input_tokens + a.output_tokens);
      });
      for (var i = 0; i < data.model_usage.length; i++) {
        var m = data.model_usage[i];
        var name = escapeHtml(shortModelName(m.model));
        html += '<div class="gw-model-row">'
          + '<span class="gw-model-name">' + name + '</span>'
          + '<span class="gw-model-cost">' + escapeHtml(formatCost(m.cost)) + '</span>'
          + '</div>';
        html += '<div class="gw-token-detail">'
          + '<span>in: ' + formatTokenCount(m.input_tokens) + '</span>'
          + '<span>out: ' + formatTokenCount(m.output_tokens) + '</span>'
          + '</div>';
      }
    }

    popover.innerHTML = html;
  }).catch(function() {
    if (!isCurrentRequest('gatewayStatus', requestVersion)) return;
  });
}

// Show/hide popover on hover
document.getElementById('gateway-status-trigger').addEventListener('mouseenter', () => {
  document.getElementById('gateway-popover').classList.add('visible');
});
document.getElementById('gateway-status-trigger').addEventListener('mouseleave', () => {
  document.getElementById('gateway-popover').classList.remove('visible');
});

// --- TEE attestation ---

let teeInfo = null;
let teeReportCache = null;
let teeReportLoading = false;

function teeApiBase() {
  var parts = window.location.hostname.split('.');
  if (parts.length < 2) return null;
  var domain = parts.slice(1).join('.');
  return window.location.protocol + '//api.' + domain;
}

function teeInstanceName() {
  return window.location.hostname.split('.')[0];
}

function checkTeeStatus() {
  var base = teeApiBase();
  if (!base) return;
  var name = teeInstanceName();
  fetch(base + '/instances/' + encodeURIComponent(name) + '/attestation').then(function(res) {
    if (!res.ok) throw new Error(res.status);
    return res.json();
  }).then(function(data) {
    teeInfo = data;
    document.getElementById('tee-shield').style.display = 'flex';
  }).catch(function() {});
}

function fetchTeeReport() {
  if (teeReportCache) {
    renderTeePopover(teeReportCache);
    return;
  }
  if (teeReportLoading) return;
  teeReportLoading = true;
  var base = teeApiBase();
  if (!base) return;
  var popover = document.getElementById('tee-popover');
  popover.innerHTML = '<div class="tee-popover-loading">Loading attestation report...</div>';
  fetch(base + '/attestation/report').then(function(res) {
    if (!res.ok) throw new Error(res.status);
    return res.json();
  }).then(function(data) {
    teeReportCache = data;
    renderTeePopover(data);
  }).catch(function() {
    popover.innerHTML = '<div class="tee-popover-loading">Could not load attestation report</div>';
  }).finally(function() {
    teeReportLoading = false;
  });
}

function renderTeePopover(report) {
  var popover = document.getElementById('tee-popover');
  var digest = (teeInfo && teeInfo.image_digest) || 'N/A';
  var fingerprint = report.tls_certificate_fingerprint || 'N/A';
  var reportData = report.report_data || '';
  var vmConfig = report.vm_config || 'N/A';
  var truncated = reportData.length > 32 ? reportData.slice(0, 32) + '...' : reportData;
  popover.innerHTML = '<div class="tee-popover-title">'
    + '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/></svg>'
    + 'TEE Attestation</div>'
    + '<div class="tee-field"><div class="tee-field-label">Image Digest</div>'
    + '<div class="tee-field-value">' + escapeHtml(digest) + '</div></div>'
    + '<div class="tee-field"><div class="tee-field-label">TLS Certificate Fingerprint</div>'
    + '<div class="tee-field-value">' + escapeHtml(fingerprint) + '</div></div>'
    + '<div class="tee-field"><div class="tee-field-label">Report Data</div>'
    + '<div class="tee-field-value">' + escapeHtml(truncated) + '</div></div>'
    + '<div class="tee-field"><div class="tee-field-label">VM Config</div>'
    + '<div class="tee-field-value">' + escapeHtml(vmConfig) + '</div></div>'
    + '<div class="tee-popover-actions">'
    + '<button class="tee-btn-copy" type="button" data-tee-action="copy-report">Copy Full Report</button></div>';
}

function copyTeeReport() {
  if (!teeReportCache) return;
  var combined = Object.assign({}, teeReportCache, teeInfo || {});
  navigator.clipboard.writeText(JSON.stringify(combined, null, 2)).then(function() {
    showToast('Attestation report copied', 'success');
  }).catch(function() {
    showToast('Failed to copy report', 'error');
  });
}

delegate(byId('tee-popover'), 'click', 'button[data-tee-action="copy-report"]', function(event) {
  event.preventDefault();
  copyTeeReport();
});

document.getElementById('tee-shield').addEventListener('mouseenter', function() {
  fetchTeeReport();
  document.getElementById('tee-popover').classList.add('visible');
});
document.getElementById('tee-shield').addEventListener('mouseleave', function() {
  document.getElementById('tee-popover').classList.remove('visible');
});

// --- Extension install ---

function installWasmExtension() {
  var name = document.getElementById('wasm-install-name').value.trim();
  if (!name) {
    showToast('Extension name is required', 'error');
    return;
  }
  var url = document.getElementById('wasm-install-url').value.trim();
  if (!url) {
    showToast('URL to .tar.gz bundle is required', 'error');
    return;
  }

  apiFetch('/api/extensions/install', {
    method: 'POST',
    body: { name: name, url: url, kind: 'wasm_tool' },
  }).then(function(res) {
    if (res.success) {
      showToast('Installed ' + name, 'success');
      document.getElementById('wasm-install-name').value = '';
      document.getElementById('wasm-install-url').value = '';
      loadExtensions();
    } else {
      showToast('Install failed: ' + (res.message || 'unknown error'), 'error');
    }
  }).catch(function(err) {
    showToast('Install failed: ' + err.message, 'error');
  });
}

function addMcpServer() {
  var name = document.getElementById('mcp-install-name').value.trim();
  if (!name) {
    showToast('Server name is required', 'error');
    return;
  }
  var url = document.getElementById('mcp-install-url').value.trim();
  if (!url) {
    showToast('MCP server URL is required', 'error');
    return;
  }

  apiFetch('/api/extensions/install', {
    method: 'POST',
    body: { name: name, url: url, kind: 'mcp_server' },
  }).then(function(res) {
    if (res.success) {
      showToast('Added MCP server ' + name, 'success');
      document.getElementById('mcp-install-name').value = '';
      document.getElementById('mcp-install-url').value = '';
      loadExtensions();
    } else {
      showToast('Failed to add MCP server: ' + (res.message || 'unknown error'), 'error');
    }
  }).catch(function(err) {
    showToast('Failed to add MCP server: ' + err.message, 'error');
  });
}

// --- Skills ---

function loadSkills() {
  var requestVersion = beginRequest('skills');
  var skillsList = document.getElementById('skills-list');
  apiFetch('/api/skills').then(function(data) {
    if (!isCurrentRequest('skills', requestVersion)) return;
    if (!data.skills || data.skills.length === 0) {
      skillsList.innerHTML = '<div class="empty-state">No skills installed</div>';
      return;
    }
    skillsList.innerHTML = '';
    for (var i = 0; i < data.skills.length; i++) {
      skillsList.appendChild(renderSkillCard(data.skills[i]));
    }
  }).catch(function(err) {
    if (!isCurrentRequest('skills', requestVersion)) return;
    skillsList.innerHTML = '<div class="empty-state">Failed to load skills: ' + escapeHtml(err.message) + '</div>';
  });
}

function renderSkillCard(skill) {
  var card = document.createElement('div');
  card.className = 'ext-card';

  var header = document.createElement('div');
  header.className = 'ext-header';

  var name = document.createElement('span');
  name.className = 'ext-name';
  name.textContent = skill.name;
  header.appendChild(name);

  var trust = document.createElement('span');
  var trustClass = skill.trust.toLowerCase() === 'trusted' ? 'trust-trusted' : 'trust-installed';
  trust.className = 'skill-trust ' + trustClass;
  trust.textContent = skill.trust;
  header.appendChild(trust);

  var version = document.createElement('span');
  version.className = 'skill-version';
  version.textContent = 'v' + skill.version;
  header.appendChild(version);

  card.appendChild(header);

  var desc = document.createElement('div');
  desc.className = 'ext-desc';
  desc.textContent = skill.description;
  card.appendChild(desc);

  if (skill.keywords && skill.keywords.length > 0) {
    var kw = document.createElement('div');
    kw.className = 'ext-keywords';
    kw.textContent = 'Activates on: ' + skill.keywords.join(', ');
    card.appendChild(kw);
  }

  var actions = document.createElement('div');
  actions.className = 'ext-actions';

  // Only show Remove for registry-installed skills, not user-placed trusted skills
  if (skill.trust.toLowerCase() !== 'trusted') {
    var removeBtn = document.createElement('button');
    removeBtn.className = 'btn-ext remove';
    removeBtn.textContent = 'Remove';
    removeBtn.addEventListener('click', function() { removeSkill(skill.name); });
    actions.appendChild(removeBtn);
  }

  card.appendChild(actions);
  return card;
}

function searchClawHub() {
  var input = document.getElementById('skill-search-input');
  var query = input.value.trim();
  if (!query) return;

  var resultsDiv = document.getElementById('skill-search-results');
  resultsDiv.innerHTML = '<div class="empty-state">Searching...</div>';

  apiFetch('/api/skills/search', {
    method: 'POST',
    body: { query: query },
  }).then(function(data) {
    resultsDiv.innerHTML = '';

    // Show registry error as a warning banner if present
    if (data.catalog_error) {
      var warning = document.createElement('div');
      warning.className = 'empty-state';
      warning.style.color = '#f0ad4e';
      warning.style.borderLeft = '3px solid #f0ad4e';
      warning.style.paddingLeft = '12px';
      warning.style.marginBottom = '16px';
      warning.textContent = 'Could not reach ClawHub registry: ' + data.catalog_error;
      resultsDiv.appendChild(warning);
    }

    // Show catalog results
    if (data.catalog && data.catalog.length > 0) {
      // Build a set of installed skill names for quick lookup
      var installedNames = {};
      if (data.installed) {
        for (var j = 0; j < data.installed.length; j++) {
          installedNames[data.installed[j].name] = true;
        }
      }

      for (var i = 0; i < data.catalog.length; i++) {
        var card = renderCatalogSkillCard(data.catalog[i], installedNames);
        card.style.animationDelay = (i * 0.06) + 's';
        resultsDiv.appendChild(card);
      }
    }

    // Show matching installed skills too
    if (data.installed && data.installed.length > 0) {
      for (var k = 0; k < data.installed.length; k++) {
        var installedCard = renderSkillCard(data.installed[k]);
        installedCard.style.animationDelay = ((data.catalog ? data.catalog.length : 0) + k) * 0.06 + 's';
        installedCard.classList.add('skill-search-result');
        resultsDiv.appendChild(installedCard);
      }
    }

    if (resultsDiv.children.length === 0) {
      resultsDiv.innerHTML = '<div class="empty-state">No skills found for "' + escapeHtml(query) + '"</div>';
    }
  }).catch(function(err) {
    resultsDiv.innerHTML = '<div class="empty-state">Search failed: ' + escapeHtml(err.message) + '</div>';
  });
}

function renderCatalogSkillCard(entry, installedNames) {
  var card = document.createElement('div');
  card.className = 'ext-card ext-available skill-search-result';

  var header = document.createElement('div');
  header.className = 'ext-header';

  var name = document.createElement('a');
  name.className = 'ext-name';
  name.textContent = entry.name || entry.slug;
  name.href = 'https://clawhub.ai/skills/' + encodeURIComponent(entry.slug);
  name.target = '_blank';
  name.rel = 'noopener';
  name.style.textDecoration = 'none';
  name.style.color = 'inherit';
  name.title = 'View on ClawHub';
  header.appendChild(name);

  if (entry.version) {
    var version = document.createElement('span');
    version.className = 'skill-version';
    version.textContent = 'v' + entry.version;
    header.appendChild(version);
  }

  card.appendChild(header);

  if (entry.description) {
    var desc = document.createElement('div');
    desc.className = 'ext-desc';
    desc.textContent = entry.description;
    card.appendChild(desc);
  }

  // Metadata row: owner, stars, downloads, recency
  var meta = document.createElement('div');
  meta.className = 'ext-meta';
  meta.style.fontSize = '11px';
  meta.style.color = '#888';
  meta.style.marginTop = '6px';

  function addMetaSep() {
    if (meta.children.length > 0) {
      meta.appendChild(document.createTextNode(' \u00b7 '));
    }
  }

  if (entry.owner) {
    var ownerSpan = document.createElement('span');
    ownerSpan.textContent = 'by ' + entry.owner;
    meta.appendChild(ownerSpan);
  }

  if (entry.stars != null) {
    addMetaSep();
    var starsSpan = document.createElement('span');
    starsSpan.textContent = entry.stars + ' stars';
    meta.appendChild(starsSpan);
  }

  if (entry.downloads != null) {
    addMetaSep();
    var dlSpan = document.createElement('span');
    dlSpan.textContent = formatCompactNumber(entry.downloads) + ' downloads';
    meta.appendChild(dlSpan);
  }

  if (entry.updatedAt) {
    var ago = formatTimeAgo(entry.updatedAt);
    if (ago) {
      addMetaSep();
      var updatedSpan = document.createElement('span');
      updatedSpan.textContent = 'updated ' + ago;
      meta.appendChild(updatedSpan);
    }
  }

  if (meta.children.length > 0) {
    card.appendChild(meta);
  }

  var actions = document.createElement('div');
  actions.className = 'ext-actions';

  var slug = entry.slug || entry.name;
  var isInstalled = installedNames[entry.name] || installedNames[slug];

  if (isInstalled) {
    var label = document.createElement('span');
    label.className = 'ext-active-label';
    label.textContent = 'Installed';
    actions.appendChild(label);
  } else {
    var installBtn = document.createElement('button');
    installBtn.className = 'btn-ext install';
    installBtn.textContent = 'Install';
    installBtn.addEventListener('click', (function(s, btn) {
      return function() {
        if (!confirm('Install skill "' + s + '" from ClawHub?')) return;
        btn.disabled = true;
        btn.textContent = 'Installing...';
        installSkill(s, null, btn);
      };
    })(slug, installBtn));
    actions.appendChild(installBtn);
  }

  card.appendChild(actions);
  return card;
}

function formatCompactNumber(n) {
  if (n >= 1000000) return (n / 1000000).toFixed(1) + 'M';
  if (n >= 1000) return (n / 1000).toFixed(1) + 'K';
  return '' + n;
}

function formatTimeAgo(epochMs) {
  var now = Date.now();
  var diff = now - epochMs;
  if (diff < 0) return null;
  var minutes = Math.floor(diff / 60000);
  if (minutes < 60) return minutes <= 1 ? 'just now' : minutes + 'm ago';
  var hours = Math.floor(minutes / 60);
  if (hours < 24) return hours + 'h ago';
  var days = Math.floor(hours / 24);
  if (days < 30) return days + 'd ago';
  var months = Math.floor(days / 30);
  if (months < 12) return months + 'mo ago';
  return Math.floor(months / 12) + 'y ago';
}

function installSkill(nameOrSlug, url, btn) {
  var body = { name: nameOrSlug };
  if (url) body.url = url;

  apiFetch('/api/skills/install', {
    method: 'POST',
    headers: { 'X-Confirm-Action': 'true' },
    body: body,
  }).then(function(res) {
    if (res.success) {
      showToast('Installed skill "' + nameOrSlug + '"', 'success');
    } else {
      showToast('Install failed: ' + (res.message || 'unknown error'), 'error');
    }
    loadSkills();
    if (btn) { btn.disabled = false; btn.textContent = 'Install'; }
  }).catch(function(err) {
    showToast('Install failed: ' + err.message, 'error');
    if (btn) { btn.disabled = false; btn.textContent = 'Install'; }
  });
}

function removeSkill(name) {
  if (!confirm('Remove skill "' + name + '"?')) return;
  apiFetch('/api/skills/' + encodeURIComponent(name), {
    method: 'DELETE',
    headers: { 'X-Confirm-Action': 'true' },
  }).then(function(res) {
    if (res.success) {
      showToast('Removed skill "' + name + '"', 'success');
    } else {
      showToast('Remove failed: ' + (res.message || 'unknown error'), 'error');
    }
    loadSkills();
  }).catch(function(err) {
    showToast('Remove failed: ' + err.message, 'error');
  });
}

function installSkillFromForm() {
  var name = document.getElementById('skill-install-name').value.trim();
  if (!name) { showToast('Skill name is required', 'error'); return; }
  var url = document.getElementById('skill-install-url').value.trim() || null;
  if (url && !url.startsWith('https://')) {
    showToast('URL must use HTTPS', 'error');
    return;
  }
  if (!confirm('Install skill "' + name + '"?')) return;
  installSkill(name, url, null);
  document.getElementById('skill-install-name').value = '';
  document.getElementById('skill-install-url').value = '';
}

// Wire up Enter key on search input
document.getElementById('skill-search-input').addEventListener('keydown', function(e) {
  if (e.key === 'Enter') searchClawHub();
});

// --- Keyboard shortcuts ---

document.addEventListener('keydown', (e) => {
  const mod = e.metaKey || e.ctrlKey;
  const tag = (e.target.tagName || '').toLowerCase();
  const inInput = tag === 'input' || tag === 'textarea';

  // Mod+1-8: switch tabs
  if (mod && e.key >= '1' && e.key <= '8') {
    e.preventDefault();
    const tabs = ['chat', 'memory', 'jobs', 'routines', 'extensions', 'skills', 'settings', 'matters'];
    const idx = parseInt(e.key) - 1;
    if (tabs[idx]) switchTab(tabs[idx]);
    return;
  }

  // Mod+K: focus chat input or memory search
  if (mod && e.key === 'k') {
    e.preventDefault();
    if (currentTab === 'memory') {
      document.getElementById('memory-search').focus();
    } else {
      document.getElementById('chat-input').focus();
    }
    return;
  }

  // Mod+N: new thread
  if (mod && e.key === 'n' && currentTab === 'chat') {
    e.preventDefault();
    createNewThread();
    return;
  }

  // Escape: close job detail or blur input
  if (e.key === 'Escape') {
    if (currentJobId) {
      closeJobDetail();
    } else if (inInput) {
      e.target.blur();
    }
    return;
  }
});

// --- Toasts ---

function showToast(message, type) {
  const container = document.getElementById('toasts');
  const toast = document.createElement('div');
  toast.className = 'toast toast-' + (type || 'info');
  toast.textContent = message;
  container.appendChild(toast);
  // Trigger slide-in
  requestAnimationFrame(() => toast.classList.add('visible'));
  setTimeout(() => {
    toast.classList.remove('visible');
    toast.addEventListener('transitionend', () => toast.remove());
  }, 4000);
}

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

function parseCsvList(raw) {
  if (!raw) return [];
  return raw
    .split(',')
    .map(function(v) { return v.trim(); })
    .filter(function(v) { return !!v; });
}

function readMatterCreateFormValues() {
  return {
    matter_id: byId('matter-create-id') ? byId('matter-create-id').value.trim() : '',
    client: byId('matter-create-client') ? byId('matter-create-client').value.trim() : '',
    confidentiality: byId('matter-create-confidentiality') ? byId('matter-create-confidentiality').value.trim() : '',
    retention: byId('matter-create-retention') ? byId('matter-create-retention').value.trim() : '',
    jurisdiction: byId('matter-create-jurisdiction') ? byId('matter-create-jurisdiction').value.trim() : '',
    practice_area: byId('matter-create-practice-area') ? byId('matter-create-practice-area').value.trim() : '',
    opened_at: byId('matter-create-opened-at') ? byId('matter-create-opened-at').value.trim() : '',
    team: parseCsvList(byId('matter-create-team') ? byId('matter-create-team').value : ''),
    adversaries: parseCsvList(byId('matter-create-adversaries') ? byId('matter-create-adversaries').value : ''),
  };
}

function validateMatterCreateForm(formData) {
  if (!formData.matter_id || !formData.client || !formData.confidentiality || !formData.retention) {
    return 'Matter ID, client, confidentiality, and retention are required.';
  }
  if (formData.opened_at && !/^\d{4}-\d{2}-\d{2}$/.test(formData.opened_at)) {
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
    opened_at: formData.opened_at,
    team: formData.team,
    adversaries: formData.adversaries,
  });
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
    controls.style.display = (matterCreateReviewState.status === 'reviewed' && matterCreateReviewState.matched)
      ? 'grid'
      : 'none';
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
  if (matter.opened_at) {
    html += '<div class="matter-card-row"><span class="matter-card-label">Opened</span><span>' + escapeHtml(matter.opened_at) + '</span></div>';
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

function renderMatterDetail() {
  var panel = byId('matter-detail-panel');
  if (!panel) return;
  if (!selectedMatterId) {
    renderMatterDetailPlaceholder('Select a matter to view workflow scorecard, deadlines, documents, and templates.');
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
  html += '<button data-matter-detail-action="build-filing-package">Build Filing Package</button>';
  html += '</div>';
  html += '</div>';

  if (selectedMatter) {
    var metadataRows = [];
    if (selectedMatter.client) metadataRows.push({ label: 'Client', value: selectedMatter.client });
    if (selectedMatter.confidentiality) metadataRows.push({ label: 'Confidentiality', value: selectedMatter.confidentiality });
    if (selectedMatter.retention) metadataRows.push({ label: 'Retention', value: selectedMatter.retention });
    if (selectedMatter.jurisdiction) metadataRows.push({ label: 'Jurisdiction', value: selectedMatter.jurisdiction });
    if (selectedMatter.practice_area) metadataRows.push({ label: 'Practice area', value: selectedMatter.practice_area });
    if (selectedMatter.opened_at) metadataRows.push({ label: 'Opened', value: selectedMatter.opened_at });
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

  panel.innerHTML = html;
}

function openMatterDetail(id) {
  if (!id) return;
  selectedMatterId = id;
  currentMatterDocuments = [];
  currentMatterTemplates = [];
  currentMatterDashboard = null;
  currentMatterDeadlines = [];
  renderMatters();
  renderMatterDetailPlaceholder('Loading detail for ' + id + '…');

  var requestVersion = beginRequest('matterDetail');
  Promise.all([
    apiFetch('/api/matters/' + encodeURIComponent(id) + '/documents?include_templates=false'),
    apiFetch('/api/matters/' + encodeURIComponent(id) + '/dashboard'),
    apiFetch('/api/matters/' + encodeURIComponent(id) + '/deadlines'),
    apiFetch('/api/matters/' + encodeURIComponent(id) + '/templates'),
  ]).then(function (results) {
    if (!isCurrentRequest('matterDetail', requestVersion)) return;
    var docsData = results[0];
    var dashboardData = results[1];
    var deadlinesData = results[2];
    var templatesData = results[3];
    currentMatterDocuments = (docsData && docsData.documents) ? docsData.documents : [];
    currentMatterDashboard = dashboardData || null;
    currentMatterDeadlines = (deadlinesData && deadlinesData.deadlines) ? deadlinesData.deadlines : [];
    currentMatterTemplates = (templatesData && templatesData.templates) ? templatesData.templates : [];
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
      currentMatterDocuments = [];
      currentMatterTemplates = [];
      currentMatterDashboard = null;
      currentMatterDeadlines = [];
    }
    renderMatters();
    updateMatterBadge();
    if (selectedMatterId) {
      openMatterDetail(selectedMatterId);
    } else {
      renderMatterDetailPlaceholder('Select a matter to view workflow scorecard, deadlines, documents, and templates.');
    }
  }).catch(function (err) {
    if (!isCurrentRequest('matters', requestVersion)) return;
    var list = document.getElementById('matters-list');
    if (list) list.innerHTML = '<div class="empty-state" style="color:var(--error)">Failed to load matters: ' + escapeHtml(err.message) + '</div>';
    renderMatterDetailPlaceholder('Failed to load matters.');
  });
}

/** Update the compact matter badge in the tab bar. */
function updateMatterBadge() {
  var badge = document.getElementById('matter-badge');
  var label = document.getElementById('matter-badge-label');
  if (!badge || !label) return;
  if (activeMatterId) {
    label.textContent = activeMatterId;
    badge.style.display = 'flex';
  } else {
    badge.style.display = 'none';
  }
}

/** Render the matters list and active-bar inside the Matters tab panel. */
function renderMatters() {
  setMattersGroupToggleFromState();
  populateMatterConflictSelector();

  var activeName = document.getElementById('matters-active-name');
  var clearBtn = document.getElementById('matters-clear-btn');
  if (activeName) activeName.textContent = activeMatterId || 'None';
  if (clearBtn) clearBtn.style.display = activeMatterId ? 'inline-block' : 'none';

  var list = document.getElementById('matters-list');
  if (!list) return;

  if (mattersCache.length === 0) {
    list.innerHTML = '<div class="empty-state">No matters found yet. Use the Create Matter form above to start one.</div>';
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
  if (formData.opened_at) body.opened_at = formData.opened_at;
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
    resetMatterCreateReview('Run conflict review before creating this matter.', null);
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

// --- Memory upload ---

/**
 * Trigger the hidden file-input click so the browser opens the file picker.
 * The input's `change` event is wired in the DOMContentLoaded block below.
 */
function triggerMemoryUpload() {
  var input = document.getElementById('memory-upload-input');
  if (input) input.click();
}

/**
 * Upload the selected files to POST /api/memory/upload.
 * Each file is written to uploads/<filename> in the workspace.
 * Reloads the memory tree on success.
 *
 * @param {FileList} files - FileList from the <input type="file"> element.
 */
function uploadMemoryFiles(files) {
  if (!files || files.length === 0) return;

  var formData = new FormData();
  for (var i = 0; i < files.length; i++) {
    formData.append('files', files[i], files[i].name);
  }

  // Reset the input so the same file can be re-uploaded after editing.
  var input = document.getElementById('memory-upload-input');
  if (input) input.value = '';

  var names = Array.from(files).map(function(f) { return f.name; }).join(', ');
  showToast('Uploading ' + names + '…', 'info');

  apiFetch('/api/memory/upload', {
    method: 'POST',
    body: formData,
  }).then(function(data) {
    if (!data || !data.files) return;
    var count = data.files.length;
    var paths = data.files.map(function(f) { return f.path; }).join(', ');
    showToast('Uploaded ' + count + ' file' + (count === 1 ? '' : 's') + ': ' + paths, 'success');
    loadMemoryTree();
  }).catch(function(err) {
    showToast('Upload failed: ' + err.message, 'error');
  });
}

// Wire up the file-input change event once the DOM is ready.
(function () {
  var input = document.getElementById('memory-upload-input');
  if (input) {
    input.addEventListener('change', function () {
      uploadMemoryFiles(this.files);
    });
  }
}());

// --- Utilities ---

function escapeHtml(str) {
  const div = document.createElement('div');
  div.textContent = str;
  return div.innerHTML;
}

function formatDate(isoString) {
  if (!isoString) return '-';
  const d = new Date(isoString);
  return d.toLocaleString();
}

// --- Settings ---

function loadSettings() {
  var requestVersion = beginRequest('settings');
  var el = document.getElementById('settings-list');
  el.innerHTML = '<div class="empty-state">Loading\u2026</div>';
  apiFetch('/api/settings').then(function(data) {
    if (!isCurrentRequest('settings', requestVersion)) return;
    if (!data.settings || data.settings.length === 0) {
      el.innerHTML = '<div class="empty-state">No settings configured. Use \u201c+ New\u201d to add one.</div>';
      return;
    }
    var tbl = document.createElement('table');
    tbl.className = 'routines-table';
    tbl.innerHTML = '<thead><tr><th>Key</th><th>Value</th><th>Updated</th><th></th></tr></thead>';
    var tbody = document.createElement('tbody');
    data.settings.forEach(function(s) {
      tbody.appendChild(renderSettingRow(s));
    });
    tbl.appendChild(tbody);
    el.innerHTML = '';
    el.appendChild(tbl);
  }).catch(function(err) {
    if (!isCurrentRequest('settings', requestVersion)) return;
    el.innerHTML = '<div class="empty-state">Failed to load: ' + escapeHtml(err.message) + '</div>';
  });
}

function renderSettingRow(s) {
  var tr = document.createElement('tr');
  tr.style.cursor = 'pointer';
  tr.addEventListener('click', function(e) {
    if (e.target.tagName === 'BUTTON') return;
    openSettingModal(s.key, s.value);
  });

  var keyTd = document.createElement('td');
  keyTd.style.fontFamily = 'var(--font-mono)';
  keyTd.textContent = s.key;

  var valTd = document.createElement('td');
  valTd.style.maxWidth = '340px';
  valTd.style.overflow = 'hidden';
  valTd.style.textOverflow = 'ellipsis';
  valTd.style.whiteSpace = 'nowrap';
  valTd.style.color = 'var(--text-secondary)';
  valTd.style.fontFamily = 'var(--font-mono)';
  var display = typeof s.value === 'string' ? s.value : JSON.stringify(s.value);
  valTd.textContent = display;

  var timeTd = document.createElement('td');
  timeTd.style.color = 'var(--text-secondary)';
  timeTd.textContent = formatDate(s.updated_at);

  var actionTd = document.createElement('td');
  var delBtn = document.createElement('button');
  delBtn.className = 'btn-ext remove';
  delBtn.textContent = 'Delete';
  delBtn.addEventListener('click', function() { deleteSetting(s.key); });
  actionTd.appendChild(delBtn);

  tr.appendChild(keyTd);
  tr.appendChild(valTd);
  tr.appendChild(timeTd);
  tr.appendChild(actionTd);
  return tr;
}

function openSettingModal(key, value) {
  closeSettingModal();
  var isNew = key === null;

  var overlay = document.createElement('div');
  overlay.className = 'configure-overlay';
  overlay.id = 'setting-modal-overlay';
  overlay.addEventListener('click', function(e) {
    if (e.target === overlay) closeSettingModal();
  });

  var modal = document.createElement('div');
  modal.className = 'configure-modal';

  var header = document.createElement('h3');
  header.textContent = isNew ? 'New Setting' : 'Edit Setting';
  modal.appendChild(header);

  var form = document.createElement('div');
  form.className = 'configure-form';

  // Key field
  var keyField = document.createElement('div');
  keyField.className = 'configure-field';
  var keyLabel = document.createElement('label');
  keyLabel.textContent = 'Key';
  keyField.appendChild(keyLabel);
  var keyInput;
  if (isNew) {
    keyInput = document.createElement('input');
    keyInput.type = 'text';
    keyInput.className = 'configure-input';
    keyInput.placeholder = 'e.g. model, system_prompt, max_tokens';
    keyInput.style.fontFamily = 'var(--font-mono)';
  } else {
    keyInput = document.createElement('div');
    keyInput.style.fontFamily = 'var(--font-mono)';
    keyInput.style.color = 'var(--text-secondary)';
    keyInput.style.padding = '8px 0';
    keyInput.style.fontSize = '13px';
    keyInput.textContent = key;
  }
  keyField.appendChild(keyInput);
  form.appendChild(keyField);

  // Value field
  var valField = document.createElement('div');
  valField.className = 'configure-field';
  var valLabel = document.createElement('label');
  valLabel.textContent = 'Value (JSON)';
  valField.appendChild(valLabel);
  var valInput = document.createElement('textarea');
  valInput.className = 'configure-input';
  valInput.rows = 6;
  valInput.style.fontFamily = 'var(--font-mono)';
  valInput.style.fontSize = '12px';
  valInput.style.resize = 'vertical';
  if (!isNew) {
    valInput.value = typeof value === 'string' ? JSON.stringify(value) : JSON.stringify(value, null, 2);
  }
  valField.appendChild(valInput);

  // Inline error
  var errMsg = document.createElement('div');
  errMsg.style.color = 'var(--danger)';
  errMsg.style.fontSize = '12px';
  errMsg.style.marginTop = '4px';
  errMsg.style.display = 'none';
  valField.appendChild(errMsg);
  form.appendChild(valField);
  modal.appendChild(form);

  // Actions
  var actions = document.createElement('div');
  actions.className = 'configure-actions';

  var saveBtn = document.createElement('button');
  saveBtn.className = 'btn-ext activate';
  saveBtn.textContent = 'Save';
  saveBtn.addEventListener('click', function() {
    var resolvedKey = isNew ? keyInput.value.trim() : key;
    if (!resolvedKey) {
      errMsg.textContent = 'Key is required.';
      errMsg.style.display = 'block';
      keyInput.focus();
      return;
    }
    var parsed;
    try {
      parsed = JSON.parse(valInput.value);
    } catch (e) {
      errMsg.textContent = 'Invalid JSON: ' + e.message;
      errMsg.style.display = 'block';
      valInput.focus();
      return;
    }
    errMsg.style.display = 'none';
    saveBtn.disabled = true;
    saveBtn.textContent = 'Saving\u2026';
    apiFetch('/api/settings/' + encodeURIComponent(resolvedKey), {
      method: 'PUT',
      body: { value: parsed },
    }).then(function() {
      closeSettingModal();
      showToast('Setting saved', 'success');
      loadSettings();
    }).catch(function(err) {
      saveBtn.disabled = false;
      saveBtn.textContent = 'Save';
      errMsg.textContent = 'Save failed: ' + err.message;
      errMsg.style.display = 'block';
    });
  });
  actions.appendChild(saveBtn);

  var cancelBtn = document.createElement('button');
  cancelBtn.className = 'btn-ext';
  cancelBtn.textContent = 'Cancel';
  cancelBtn.addEventListener('click', closeSettingModal);
  actions.appendChild(cancelBtn);

  modal.appendChild(actions);
  overlay.appendChild(modal);
  document.body.appendChild(overlay);

  // Focus the right input
  if (isNew) {
    keyInput.focus();
  } else {
    valInput.focus();
    valInput.setSelectionRange(0, 0);
  }
}

function closeSettingModal() {
  var existing = document.getElementById('setting-modal-overlay');
  if (existing) existing.remove();
}

function deleteSetting(key) {
  if (!confirm('Delete setting \u201c' + key + '\u201d?')) return;
  apiFetch('/api/settings/' + encodeURIComponent(key), { method: 'DELETE' })
    .then(function() {
      showToast('Setting deleted', 'success');
      loadSettings();
    })
    .catch(function(err) {
      showToast('Delete failed: ' + err.message, 'error');
    });
}

function exportSettings() {
  apiFetch('/api/settings/export').then(function(data) {
    var blob = new Blob([JSON.stringify(data.settings, null, 2)], { type: 'application/json' });
    var url = URL.createObjectURL(blob);
    var a = document.createElement('a');
    a.href = url;
    a.download = 'settings.json';
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  }).catch(function(err) {
    showToast('Export failed: ' + err.message, 'error');
  });
}

function importSettings(file) {
  var reader = new FileReader();
  reader.onload = function(e) {
    var parsed;
    try {
      parsed = JSON.parse(e.target.result);
    } catch (err) {
      showToast('Invalid JSON file', 'error');
      return;
    }
    // Accept either { settings: {...} } (export format) or a plain object
    var settingsMap = (parsed && typeof parsed.settings === 'object' && !Array.isArray(parsed.settings))
      ? parsed.settings
      : parsed;
    if (settingsMap === null || typeof settingsMap !== 'object' || Array.isArray(settingsMap)) {
      showToast('Unrecognised format — expected a JSON object', 'error');
      return;
    }
    var count = Object.keys(settingsMap).length;
    apiFetch('/api/settings/import', {
      method: 'POST',
      body: { settings: settingsMap },
    }).then(function() {
      showToast('Imported ' + count + ' setting' + (count === 1 ? '' : 's'), 'success');
      loadSettings();
    }).catch(function(err) {
      showToast('Import failed: ' + err.message, 'error');
    });
  };
  reader.readAsText(file);
}

// Wire up Settings action buttons after DOM is ready
document.addEventListener('DOMContentLoaded', function() {
  var addBtn = document.getElementById('settings-add-btn');
  if (addBtn) addBtn.addEventListener('click', function() { openSettingModal(null, null); });

  var exportBtn = document.getElementById('settings-export-btn');
  if (exportBtn) exportBtn.addEventListener('click', exportSettings);

  var importBtn = document.getElementById('settings-import-btn');
  var importFile = document.getElementById('settings-import-file');
  if (importBtn && importFile) {
    importBtn.addEventListener('click', function() { importFile.click(); });
    importFile.addEventListener('change', function() {
      if (importFile.files && importFile.files[0]) {
        importSettings(importFile.files[0]);
        importFile.value = '';
      }
    });
  }
});

// --- Routine creation form ---

function openRoutineCreateModal() {
  closeRoutineCreateModal();

  var overlay = document.createElement('div');
  overlay.className = 'configure-overlay';
  overlay.id = 'routine-create-modal-overlay';
  overlay.addEventListener('click', function(e) {
    if (e.target === overlay) closeRoutineCreateModal();
  });

  var modal = document.createElement('div');
  modal.className = 'configure-modal';
  modal.style.maxWidth = '520px';

  var header = document.createElement('h3');
  header.textContent = 'New Routine';
  modal.appendChild(header);

  var form = document.createElement('div');
  form.className = 'configure-form';

  function makeField(labelText, input, optional) {
    var field = document.createElement('div');
    field.className = 'configure-field';
    var lbl = document.createElement('label');
    lbl.textContent = labelText;
    if (optional) {
      var opt = document.createElement('span');
      opt.className = 'field-optional';
      opt.textContent = ' (optional)';
      lbl.appendChild(opt);
    }
    field.appendChild(lbl);
    field.appendChild(input);
    return field;
  }

  var nameInput = document.createElement('input');
  nameInput.type = 'text';
  nameInput.className = 'configure-input';
  nameInput.placeholder = 'e.g. daily-pr-review';
  form.appendChild(makeField('Name', nameInput));

  var descInput = document.createElement('input');
  descInput.type = 'text';
  descInput.className = 'configure-input';
  descInput.placeholder = 'What this routine does';
  form.appendChild(makeField('Description', descInput, true));

  var triggerSel = document.createElement('select');
  triggerSel.className = 'configure-input';
  ['cron', 'event', 'webhook', 'manual'].forEach(function(v) {
    var o = document.createElement('option');
    o.value = v;
    o.textContent = v;
    triggerSel.appendChild(o);
  });
  form.appendChild(makeField('Trigger type', triggerSel));

  var cronDiv = document.createElement('div');
  var schedInput = document.createElement('input');
  schedInput.type = 'text';
  schedInput.className = 'configure-input';
  schedInput.placeholder = '0 9 * * MON-FRI';
  var schedHint = document.createElement('div');
  schedHint.style.fontSize = '11px';
  schedHint.style.color = 'var(--text-secondary)';
  schedHint.style.marginTop = '4px';
  schedHint.textContent = '6-field cron expression (second minute hour dom month dow)';
  var cronField = makeField('Schedule', schedInput);
  cronField.appendChild(schedHint);
  cronDiv.appendChild(cronField);
  form.appendChild(cronDiv);

  var eventDiv = document.createElement('div');
  var patternInput = document.createElement('input');
  patternInput.type = 'text';
  patternInput.className = 'configure-input';
  patternInput.placeholder = 'deploy.*prod';
  eventDiv.appendChild(makeField('Pattern (regex)', patternInput));
  var channelInput = document.createElement('input');
  channelInput.type = 'text';
  channelInput.className = 'configure-input';
  channelInput.placeholder = 'telegram, slack, ... (leave blank for any)';
  eventDiv.appendChild(makeField('Channel', channelInput, true));
  form.appendChild(eventDiv);

  var actionSel = document.createElement('select');
  actionSel.className = 'configure-input';
  ['lightweight', 'full_job'].forEach(function(v) {
    var o = document.createElement('option');
    o.value = v;
    o.textContent = v;
    actionSel.appendChild(o);
  });
  form.appendChild(makeField('Action type', actionSel));

  var promptLabel = document.createElement('label');
  promptLabel.textContent = 'Prompt';
  var promptInput = document.createElement('textarea');
  promptInput.className = 'configure-input';
  promptInput.rows = 4;
  promptInput.placeholder = 'What should the agent do when this routine fires?';
  promptInput.style.resize = 'vertical';
  var promptField = document.createElement('div');
  promptField.className = 'configure-field';
  promptField.appendChild(promptLabel);
  promptField.appendChild(promptInput);
  form.appendChild(promptField);

  var advToggle = document.createElement('button');
  advToggle.type = 'button';
  advToggle.style.background = 'none';
  advToggle.style.border = 'none';
  advToggle.style.color = 'var(--text-secondary)';
  advToggle.style.cursor = 'pointer';
  advToggle.style.fontSize = '12px';
  advToggle.style.padding = '8px 0 4px';
  advToggle.style.textAlign = 'left';
  advToggle.textContent = '\u25b8 Advanced';
  var advDiv = document.createElement('div');
  advDiv.style.display = 'none';
  var cooldownInput = document.createElement('input');
  cooldownInput.type = 'number';
  cooldownInput.className = 'configure-input';
  cooldownInput.value = '300';
  cooldownInput.min = '0';
  var cooldownHint = document.createElement('div');
  cooldownHint.style.fontSize = '11px';
  cooldownHint.style.color = 'var(--text-secondary)';
  cooldownHint.style.marginTop = '4px';
  cooldownHint.textContent = 'Minimum seconds between fires';
  var cooldownField = makeField('Cooldown (seconds)', cooldownInput);
  cooldownField.appendChild(cooldownHint);
  advDiv.appendChild(cooldownField);
  advToggle.addEventListener('click', function() {
    var open = advDiv.style.display !== 'none';
    advDiv.style.display = open ? 'none' : 'block';
    advToggle.textContent = (open ? '\u25b8' : '\u25be') + ' Advanced';
  });
  form.appendChild(advToggle);
  form.appendChild(advDiv);

  modal.appendChild(form);

  var errMsg = document.createElement('div');
  errMsg.style.color = 'var(--danger)';
  errMsg.style.fontSize = '12px';
  errMsg.style.marginTop = '8px';
  errMsg.style.display = 'none';
  modal.appendChild(errMsg);

  var actions = document.createElement('div');
  actions.className = 'configure-actions';
  var saveBtn = document.createElement('button');
  saveBtn.className = 'btn-ext activate';
  saveBtn.textContent = 'Create';
  var cancelBtn = document.createElement('button');
  cancelBtn.className = 'btn-ext';
  cancelBtn.textContent = 'Cancel';
  cancelBtn.addEventListener('click', closeRoutineCreateModal);
  actions.appendChild(saveBtn);
  actions.appendChild(cancelBtn);
  modal.appendChild(actions);

  overlay.appendChild(modal);
  document.body.appendChild(overlay);

  function updateTriggerFields() {
    var t = triggerSel.value;
    cronDiv.style.display = t === 'cron' ? 'block' : 'none';
    eventDiv.style.display = t === 'event' ? 'block' : 'none';
  }

  function updateActionFields() {
    promptLabel.textContent = actionSel.value === 'full_job' ? 'Description' : 'Prompt';
  }

  triggerSel.addEventListener('change', updateTriggerFields);
  actionSel.addEventListener('change', updateActionFields);
  updateTriggerFields();
  nameInput.focus();

  saveBtn.addEventListener('click', function() {
    errMsg.style.display = 'none';

    var name = nameInput.value.trim();
    if (!name) {
      errMsg.textContent = 'Name is required.';
      errMsg.style.display = 'block';
      nameInput.focus();
      return;
    }

    var triggerType = triggerSel.value;
    if (triggerType === 'cron' && !schedInput.value.trim()) {
      errMsg.textContent = 'Schedule is required for cron trigger.';
      errMsg.style.display = 'block';
      schedInput.focus();
      return;
    }
    if (triggerType === 'event' && !patternInput.value.trim()) {
      errMsg.textContent = 'Pattern is required for event trigger.';
      errMsg.style.display = 'block';
      patternInput.focus();
      return;
    }

    var prompt = promptInput.value.trim();
    if (!prompt) {
      errMsg.textContent =
        (actionSel.value === 'full_job' ? 'Description' : 'Prompt') + ' is required.';
      errMsg.style.display = 'block';
      promptInput.focus();
      return;
    }

    var parsedCooldown = parseInt(cooldownInput.value, 10);
    if (Number.isNaN(parsedCooldown)) parsedCooldown = 300;
    parsedCooldown = Math.max(0, parsedCooldown);

    var body = {
      name: name,
      description: descInput.value.trim() || undefined,
      trigger_type: triggerType,
      schedule: triggerType === 'cron' ? schedInput.value.trim() : undefined,
      event_pattern: triggerType === 'event' ? patternInput.value.trim() : undefined,
      event_channel: (triggerType === 'event' && channelInput.value.trim()) ? channelInput.value.trim() : undefined,
      action_type: actionSel.value,
      prompt: prompt,
      cooldown_secs: parsedCooldown,
    };

    saveBtn.disabled = true;
    saveBtn.textContent = 'Creating...';

    apiFetch('/api/routines', { method: 'POST', body: body })
      .then(function() {
        closeRoutineCreateModal();
        showToast('Routine created', 'success');
        loadRoutines();
      })
      .catch(function(err) {
        saveBtn.disabled = false;
        saveBtn.textContent = 'Create';
        errMsg.textContent = err.message;
        errMsg.style.display = 'block';
      });
  });
}

function closeRoutineCreateModal() {
  var existing = document.getElementById('routine-create-modal-overlay');
  if (existing) existing.remove();
}

(function() {
  var btn = document.getElementById('routine-create-btn');
  if (btn) btn.addEventListener('click', openRoutineCreateModal);
})();
