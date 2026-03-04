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
