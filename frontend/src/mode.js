export function presentationMode() {
 try { return localStorage.getItem('eplyx-detail') === 'technical' ? 'technical' : 'overview'; } catch { return 'overview'; }
}
export function setPresentationMode(mode) {
 const selected=mode==='technical'?'technical':'overview';
 document.documentElement.dataset.mode=selected;
 try { localStorage.setItem('eplyx-detail',selected); } catch { /* Presentation still works without storage. */ }
 document.dispatchEvent(new CustomEvent('eplyx-mode',{detail:selected}));
}
export function initializeMode() { document.documentElement.dataset.mode=presentationMode(); }
