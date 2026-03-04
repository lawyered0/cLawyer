// --- Threads ---

function loadThreads() {
  apiFetch('/api/chat/threads').then((data) => {
    assistantThreadId = data && data.assistant_thread ? data.assistant_thread.id : null;
    const list = document.getElementById('thread-list');
    list.innerHTML = '';
    const entries = [];
    if (data.assistant_thread) entries.push(data.assistant_thread);
    const threads = data.threads || [];
    for (const thread of threads) {
      entries.push(thread);
    }

    for (const thread of entries) {
      const item = document.createElement('div');
      item.className = 'thread-item' + (thread.id === currentThreadId ? ' active' : '');
      if (thread.id === assistantThreadId) {
        item.classList.add('thread-item-assistant');
      }
      const label = document.createElement('span');
      label.className = 'thread-label';
      const labelText = thread.id === assistantThreadId
        ? 'Assistant'
        : (thread.title || thread.id.substring(0, 8));
      label.textContent = labelText;
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
      switchThread(assistantThreadId);
    }

    // Enable chat input once a thread is available
    if (currentThreadId) {
      enableChatInput();
    }
  }).catch(() => {});
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

