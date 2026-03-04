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
    container.innerHTML = '<div class="tree-item">No files in workspace</div>';
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
      tree.innerHTML = '<div class="tree-item">No results</div>';
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
