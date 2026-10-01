function savedPresentationMode() {
 try { return localStorage.getItem('eplyx-detail') === 'technical' ? 'technical' : 'overview'; } catch { return 'overview'; }
}
export function presentationMode() {
 const active = globalThis.document?.documentElement?.dataset.mode;
 if (active === 'overview' || active === 'technical') return active;
 return savedPresentationMode();
}
export function setPresentationMode(mode) {
 const selected=mode==='technical'?'technical':'overview';
 document.documentElement.dataset.mode=selected;
 try { localStorage.setItem('eplyx-detail',selected); } catch { /* Presentation still works without storage. */ }
 document.dispatchEvent(new CustomEvent('eplyx-mode',{detail:selected}));
}
// The initial HTML declares Overview. On reload the saved choice takes priority.
export function initializeMode() { document.documentElement.dataset.mode=savedPresentationMode(); }
