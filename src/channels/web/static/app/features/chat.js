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

