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

