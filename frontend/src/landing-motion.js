// A single finite sequence connects the declared steps. Text never waits for
// an observer and the reduced-motion version shows the complete connection.
export function attachLandingMotion() {
  const list = document.querySelector('.workflow-steps');
  if (!list) return () => {};
  const preference = matchMedia('(prefers-reduced-motion: reduce)');
  let observer;

  const update = () => {
    observer?.disconnect();
    list.classList.toggle('workflow-steps--animated', !preference.matches && 'IntersectionObserver' in window);
    if (preference.matches || !('IntersectionObserver' in window)) return;
    observer = new IntersectionObserver(entries => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        entry.target.classList.add('is-connected');
        observer.unobserve(entry.target);
      }
    }, { threshold: .55, rootMargin: '0px 0px -10% 0px' });
    list.querySelectorAll('li:not(.is-connected)').forEach(step => observer.observe(step));
  };
  const visibility = () => list.classList.toggle('workflow-steps--paused', document.hidden);
  preference.addEventListener('change', update);
  document.addEventListener('visibilitychange', visibility);
  update();
  visibility();
  return () => {
    observer?.disconnect();
    preference.removeEventListener('change', update);
    document.removeEventListener('visibilitychange', visibility);
  };
}
