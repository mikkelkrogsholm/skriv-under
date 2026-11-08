document.addEventListener('click', async (event) => {
  const button = event.target.closest('[data-copy-target]');
  if (!button) return;
  const input = document.getElementById(button.dataset.copyTarget);
  const note = button.closest('.share-link').querySelector('[data-copy-note]');
  try { await navigator.clipboard.writeText(input.value); note.textContent = 'Link kopieret. Del det kun med den rette modtager.'; }
  catch { input.focus(); input.select(); note.textContent = 'Link markeret. Kopiér det med Ctrl+C eller Cmd+C.'; }
});
(() => {
  const panel = document.querySelector('[data-process-id]');
  if (!panel || panel.dataset.status !== 'pending') return;
  const timer = setInterval(async () => {
    if (document.hidden) return;
    try {
      const response = await fetch(`/documents/${panel.dataset.processId}/status`, { credentials: 'same-origin', cache: 'no-store' });
      if (response.status === 403) {
        clearInterval(timer);
        panel.querySelector('[data-live-note]').textContent = 'Dit login er udløbet. Log ind igen for at følge processen.';
        return;
      }
      if (!response.ok) return;
      const state = await response.json();
      if (String(state.revision) !== panel.dataset.revision || state.status !== panel.dataset.status) window.location.reload();
      else if (state.status !== 'pending') clearInterval(timer);
    } catch { /* Keep the current view when temporarily offline. */ }
  }, 10000);
})();

(() => {
  const input = document.getElementById('package-pdfs');
  const list = document.querySelector('[data-upload-list]');
  if (!input || !list) return;
  input.addEventListener('change', () => {
    list.replaceChildren();
    let bytes = 0;
    for (const file of input.files) {
      bytes += file.size;
      const item = document.createElement('li');
      item.textContent = `${file.name} · ${(file.size / 1024 / 1024).toLocaleString('da-DK', { maximumFractionDigits: 1 })} MiB`;
      list.append(item);
    }
    input.setCustomValidity(input.files.length > 10 ? 'Vælg højst 10 PDF-filer.' : bytes > 20 * 1024 * 1024 ? 'Filerne må tilsammen højst fylde 20 MiB.' : '');
    input.reportValidity();
  });
})();
