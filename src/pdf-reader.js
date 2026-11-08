const reader = document.querySelector('.pdf-reader');
if (reader) {
  const upload = reader.dataset.localFiles ? document.getElementById(reader.dataset.localFiles) : null;
  let localFiles = [], objectUrls = [];
  const status = reader.querySelector('[data-reader-status]');
  const files = reader.querySelector('#reader-document');
  const canvas = reader.querySelector('canvas');
  const viewport = reader.querySelector('.reader-canvas');
  const previous = reader.querySelector('[data-reader-prev]');
  const next = reader.querySelector('[data-reader-next]');
  const counter = reader.querySelector('[data-reader-page]');
  const text = reader.querySelector('[data-reader-text]');
  const zoom = reader.querySelector('#reader-zoom');
  const download = reader.querySelector('[data-reader-download]');
  const retry = reader.querySelector('[data-reader-retry]');
  let pdf, loading, render, pageNumber = 1, generation = 0;

  function controls(busy) {
    previous.disabled = busy || pageNumber <= 1;
    next.disabled = busy || !pdf || pageNumber >= pdf.numPages;
    zoom.disabled = busy;
    viewport.setAttribute('aria-busy', String(busy));
  }
  function fail() {
    retry.hidden = false;
    status.textContent = 'PDF’en kunne ikke vises her. Prøv at vælge dokumentet igen, eller åbn PDF-linket nedenfor.';
    canvas.hidden = true;
    text.textContent = '';
    controls(true);
  }
  async function draw() {
    if (!pdf) return;
    const current = ++generation;
    render?.cancel();
    controls(true);
    canvas.hidden = true;
    text.textContent = '';
    status.textContent = 'Indlæser side …';
    try {
      const page = await pdf.getPage(pageNumber);
      if (current !== generation) return;
      const natural = page.getViewport({ scale: 1 });
      const width = Math.max(100, viewport.clientWidth - 24);
      const scale = Math.min(width / natural.width, 2) * Number(zoom.value);
      const view = page.getViewport({ scale });
      // Bound pixel memory even for unusually tall pages and high-density displays.
      const density = Math.min(devicePixelRatio || 1, 2, Math.sqrt(8_000_000 / (view.width * view.height)));
      canvas.width = Math.max(1, Math.floor(view.width * density));
      canvas.height = Math.max(1, Math.floor(view.height * density));
      canvas.style.width = `${view.width}px`;
      canvas.style.height = `${view.height}px`;
      canvas.setAttribute('aria-label', `${files.selectedOptions[0].textContent}, side ${pageNumber} af ${pdf.numPages}`);
      render = page.render({ canvasContext: canvas.getContext('2d'), viewport: view, transform: [density, 0, 0, density, 0, 0] });
      await render.promise;
      if (current !== generation) return;
      canvas.hidden = false;
      counter.textContent = `Side ${pageNumber} af ${pdf.numPages}`;
      status.textContent = `${files.selectedOptions[0].textContent} · ${pdf.numPages} ${pdf.numPages === 1 ? 'side' : 'sider'}`;
      controls(false);
      const content = await page.getTextContent();
      if (current !== generation) return;
      text.textContent = content.items.map(item => item.str + (item.hasEOL ? '\n' : ' ')).join('') || 'Denne side har ingen udtrækkelig tekst. Se den viste PDF-side.';
    } catch (error) {
      if (current === generation && error.name !== 'RenderingCancelledException') fail();
    }
  }
  async function open() {
    const current = ++generation;
    render?.cancel();
    controls(true);
    canvas.hidden = true;
    counter.textContent = 'Side –';
    text.textContent = '';
    retry.hidden = true;
    status.textContent = 'Indlæser PDF …';
    download.href = upload ? objectUrls[Number(files.value)] : files.value;
    pdf = null;
    const old = loading;
    loading = null;
    try {
      await old?.destroy();
      const { getDocument, GlobalWorkerOptions } = await import('/assets/pdfjs/pdf.mjs');
      if (current !== generation) return;
      GlobalWorkerOptions.workerSrc = '/assets/pdfjs/pdf.worker.mjs';
      const source = upload
        ? { data: new Uint8Array(await localFiles[Number(files.value)].arrayBuffer()) }
        : { url: files.value, withCredentials: true };
      if (current !== generation) return;
      const task = getDocument({ ...source,
        isEvalSupported: false, enableXfa: false,
        cMapUrl: '/assets/pdfjs/cmaps/', cMapPacked: true,
        standardFontDataUrl: '/assets/pdfjs/standard_fonts/', wasmUrl: '/assets/pdfjs/wasm/' });
      loading = task;
      const loaded = await task.promise;
      if (current !== generation) { await task.destroy(); return; }
      pdf = loaded;
      pageNumber = 1;
      await draw();
    } catch { if (current === generation) fail(); }
  }
  retry.addEventListener('click', open);
  files.addEventListener('change', open);
  previous.addEventListener('click', () => { if (pageNumber > 1) { pageNumber--; draw(); } });
  next.addEventListener('click', () => { if (pdf && pageNumber < pdf.numPages) { pageNumber++; draw(); } });
  zoom.addEventListener('change', draw);
  let resizeTimer;
  let readerWidth = viewport.clientWidth;
  new ResizeObserver(() => {
    if (viewport.clientWidth === readerWidth) return;
    readerWidth = viewport.clientWidth;
    clearTimeout(resizeTimer); resizeTimer = setTimeout(draw, 150);
  }).observe(viewport);
  function releaseUrls() {
    objectUrls.forEach(url => URL.revokeObjectURL(url));
    objectUrls = [];
  }
  function selectedFiles() {
    // Changing the file input invalidates any previous render before its bytes finish loading.
    ++generation;
    render?.cancel();
    pdf = null;
    const old = loading;
    loading = null;
    old?.destroy().catch(() => {});
    releaseUrls();
    localFiles = Array.from(upload.files);
    files.replaceChildren();
    canvas.hidden = true;
    text.textContent = '';
    reader.hidden = !localFiles.length || localFiles.length > 10 || localFiles.reduce((size, file) => size + file.size, 0) > 20 * 1024 * 1024;
    if (reader.hidden) return;
    localFiles.forEach((file, index) => {
      objectUrls.push(URL.createObjectURL(new Blob([file], { type: 'application/pdf' })));
      const option = document.createElement('option');
      option.value = String(index);
      option.textContent = `${index + 1}. ${file.name}`;
      files.append(option);
    });
    open();
  }
  if (upload) {
    upload.addEventListener('change', selectedFiles);
    window.addEventListener('pagehide', event => { if (!event.persisted) releaseUrls(); });
    selectedFiles();
  } else {
    open();
  }
}
