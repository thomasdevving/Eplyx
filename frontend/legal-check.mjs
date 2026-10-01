import { LEGAL_CONFIG, legalOpenItems } from './src/legal-config.js';

// A publication reminder, not an automated legal assessment. Review the
// notices and deployment evidence as well as completing these supplied facts.
const open = legalOpenItems();
if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(LEGAL_CONFIG.email)) {
  console.error('A valid public contact email is required.');
  process.exitCode = 1;
} else if (open.length) {
  console.log(`Legal notices remain drafts (${open.length} open operator items):`);
  for (const item of open) console.log(`- ${item}`);
  console.log('See docs/website-legal.md. Do not treat a passing build as legal clearance.');
  if (process.argv.includes('--production')) process.exitCode = 1;
} else {
  console.log('Recorded operator review items are complete. Review current notices and deployment before publication.');
}
