// Public information only. Do not put credentials or private addresses here.
// The release check deliberately distinguishes supplied facts from open items.
export const LEGAL_CONFIG = Object.freeze({
  brand: 'Eplyx',
  email: 'eplyxcontact@gmail.com',
  country: 'Netherlands',
  hostingCountry: null,
  scope: 'Free information website and demonstration reports',
  updated: '2026-10-08',
  legalName: null,
  operatorStatus: null,
  addressDisclosureReviewed: false,
  registrationDisclosureReviewed: false,
  hostingProvider: 'Cloudflare',
  hostingLogRetention: null,
  emailRetention: null,
  internationalTransfers: null,
  privacyBasisReviewed: false,
  assetRightsReviewed: false,
  demoCorpusReviewed: false,
  operationalReviewComplete: false,
});

export const legalOpenItems = (config = LEGAL_CONFIG) => [
  ['legalName', 'Full legal operator identity'],
  ['operatorStatus', 'Individual or registered business status'],
  ['addressDisclosureReviewed', 'Applicable address disclosure'],
  ['registrationDisclosureReviewed', 'Applicable business registration disclosures'],
  ['hostingProvider', 'Hosting provider'],
  ['hostingLogRetention', 'Hosting access-log retention'],
  ['emailRetention', 'Email retention and deletion procedure'],
  ['internationalTransfers', 'Provider access, processing locations and transfer safeguards'],
  ['privacyBasisReviewed', 'Lawful bases and legitimate-interest assessment'],
  ['assetRightsReviewed', 'Asset ownership and third-party licences'],
  ['demoCorpusReviewed', 'Demonstration data provenance and publication basis'],
  ['operationalReviewComplete', 'Privacy requests and incident response procedures'],
].filter(([key]) => !config[key]).map(([, label]) => label);
