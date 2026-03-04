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

