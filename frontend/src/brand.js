export function Logo({ wordmark = true, className = '' } = {}) {
  return `
    <span class="brand ${className}" aria-label="Eplyx">
      <svg class="brand__mark" viewBox="0 0 440 440" aria-hidden="true">
        <use href="/public/logo.svg#crescent"></use>
        <use href="/public/logo.svg#wave"></use>
      </svg>
      ${wordmark ? '<span class="brand__word">Eplyx</span>' : ''}
    </span>`;
}

export function Mark({ className = '' } = {}) {
  return `<svg class="${className}" viewBox="0 0 440 440" aria-hidden="true"><use href="/public/logo.svg#crescent"></use><use href="/public/logo.svg#wave"></use></svg>`;
}
