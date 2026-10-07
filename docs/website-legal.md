# Website privacy and publication record

Prepared 1 October 2026 for the current free information website and demonstration reports. This is a working operator record, not a certificate that the site is compliant in every country. It excludes separately operated hosted accounts, evidence uploads, CLI services and future commercial features.

## Supplied and verified facts

- Public name: **Eplyx**. Operator based in the Netherlands; visitors worldwide.
- Contact and privacy requests: **eplyxcontact@gmail.com**.
- Hosting update, 8 October 2026: the operator selected **Cloudflare** for `eplyx.dev`. The deployment uses its global static asset network; the earlier Netherlands-only hosting statement no longer applies. Subprocessors, access locations and contractual arrangements remain unverified.
- Current public offering: free information and demos; no analytics, advertising or payments.
- Frontend review: assets and fonts self-hosted, no tracking SDK, no cookie set by public information pages. The decorative intro uses page memory only.
- The operator requested that an address be omitted for now. No address, registry number or legal identity has been invented. This does not decide whether a disclosure is legally required.
- Mail is handled through Gmail. A Netherlands hosting location does not establish that email or provider access stays in the Netherlands/EEA.

The site now links to `/legal`, `/privacy`, `/cookies`, `/terms`, `/contact` and `/licenses` from its footer. Notices are in English to match the site. They visibly remain drafts while required operator details are incomplete.

## Complete before treating the notices as final

1. Identify the natural person or legal entity responsible for Eplyx. A brand alone does not necessarily identify a controller. Determine whether this is a personal project or registered business, and review applicable name, address, registration and other mandatory disclosures. Do not mark these resolved merely because an address was omitted on request.
2. Identify the actual hosting provider and its contracted region, subprocessors, support access, log fields, backup retention and deletion settings. Retain the applicable service terms and any required processing agreement. Check the deployed domain/proxy for added cookies or analytics.
3. Check the Gmail account arrangement and the applicable provider terms, roles and transfer safeguards. Decide whether it is suitable for this processing; a generic privacy-policy link is not a processing agreement.
4. Document each actual purpose, lawful basis, necessity, recipients and retention period. Review the proposed legitimate interests, impact on visitors and safeguards; do not use consent as a blanket substitute. Update the public privacy notice with the resulting facts.
5. Establish and apply a mailbox retention/deletion schedule, including attachments, trash and backups. Set hosting log retention with the provider. Do not publish a retention promise that cannot be implemented.
6. Verify the demonstration corpus: origin of public addresses and transaction data, whether individuals can be identified, necessity, publication basis and how correction/removal requests are handled. Public blockchain data is not automatically outside data protection law.
7. Review rights to the logo, reference artwork and generated imagery. Preserve Three.js MIT and font OFL notices. The notices page does not assign a licence to the whole repository.
8. Assign responsibility for the request and incident procedures below, secure the mailbox and hosting account, and document the actual controls and provider evidence.
9. Review the laws applicable to the actual worldwide offering. The Netherlands/EU baseline does not settle every country's obligations. Reassess when accounts, paid services, targeted markets or new data processing are introduced.

Public facts and unresolved review items live in `frontend/src/legal-config.js`; notice wording lives in `frontend/src/legal.js`. Keep both consistent. Do not store secrets or a private home address in frontend configuration. If a required public disclosure is identified, provide the appropriate lawful disclosure rather than simply switching a boolean.

`npm run check:legal` lists missing items without blocking development. `npm run check:legal:release` exits unsuccessfully while items are open. These commands are reminders; they cannot verify contracts, legal applicability, deployment facts or actual operational compliance. Clearing the configuration flags alone does not finalise the notice wording or certify the site.

## Processing inventory to maintain

| Activity | People/data | Purpose and proposed basis | Recipients and locations | Retention/action |
| --- | --- | --- | --- | --- |
| Site delivery/security | Visitors; IP address, HTTP request headers, any provider logs | Deliver/protect the site; proposed legitimate interests, assessment pending | Cloudflare global network; access/subprocessors and contractual arrangements unverified | Confirm actual logging, configure necessary retention and deletion |
| Email correspondence | Senders; address, name, message, attachments | Respond to enquiries; proposed legitimate interests, assessment pending; legal obligation where a rights request requires handling | Eplyx operator and Google Gmail; arrangements/transfers pending | Define and apply purpose-based deletion, with documented exceptions |
| Demo publication | People identifiable from retained blockchain references, if any | Show evidence examples; provenance, necessity and lawful publication basis pending | Public website recipients worldwide | Review corpus, minimise data, define review/removal process |

Keep this inventory current even if a specific formal register exemption applies. It helps explain and evidence the actual processing. Do not put requesters' personal details in this public repository.

Optional developer tools use `eplyx-operator-token` in session storage, `eplyx-last-project` in local storage and `eplyx-detail` in the separate dashboard. The separate hosted service uses the `eplyx_session` login cookie and request-retry session keys. Their triggers and lifetimes are disclosed on `/cookies`. Before offering accounts/uploads as part of the public release, complete a separate service inventory covering credentials, project/source evidence, backups, processors, access controls, deletion, notices and applicable terms. Apply secure cookies and HTTPS in production. Static-site documentation does not make those services ready.

## Privacy request procedure

Monitor the published mailbox. Record receipt date, request type, responsible person, deadline and outcome in a restricted record. Collect only the identity verification needed for the request; do not routinely ask for a full identity document.

Identify relevant records with providers, assess which rights apply, and fulfil or explain the outcome. GDPR requests normally require a response within one month; a justified extension must be communicated within that month. Handle applicable provider/recipient notifications and retain only the evidence needed to demonstrate handling. Define access and deletion for the request log. This procedure still needs an assigned operator and implementation.

## Incident procedure

Secure affected systems, limit further disclosure, preserve necessary evidence and record when the incident became known, affected data/people, likely consequences and corrective actions. Assess whether notification is required. A reportable personal data breach must be notified to the competent authority without undue delay and, where feasible, within 72 hours of awareness; inform affected people without undue delay when the applicable high-risk threshold is met. Record the decision and reasoning even when no notification is required. Use the current AP guidance and a restricted incident register; do not commit incident personal data here.

## Consent and deployment review

No consent popup has been added because the current frontend includes no consent-requiring tracking. Informing visitors is still necessary. Check the actual deployment, not just source code: cookie headers, network requests, proxy settings and browser storage in a fresh session.

If analytics, marketing, tracking embeds or another consent-requiring purpose is added, keep it off until active opt-in. Give refusal the same ease as acceptance, allow withdrawal, and store only the necessary consent record. Scrolling/browsing is not consent. Update the inventory and notices before activation. A decorative consent banner that does not block integrations is insufficient.

Deployment also needs HTTPS, account protection, least-privilege access and an appropriate security review. These are operator tasks; no live hosting configuration or provider account has been changed in this work.

## Official references

- [Dutch cookie rules](https://business.gov.nl/regulations/cookies/) and [AP guidance on cookie banners](https://autoriteitpersoonsgegevens.nl/themas/internet-slimme-apparaten/cookies/heldere-en-misleidende-cookiebanners).
- [Business identity and correspondence disclosures](https://business.gov.nl/regulations/rules-business-correspondence/).
- [AP right to information](https://www.autoriteitpersoonsgegevens.nl/themas/basis-avg/privacyrechten-avg/recht-op-informatie) and [government overview of GDPR privacy rights](https://www.rijksoverheid.nl/vraag-en-antwoord/privacy-en-persoonsgegevens/hoe-versterkt-de-algemene-verordening-gegevensbescherming-avg-mijn-privacyrechten).
- [GDPR implementation checklist, including processor agreements](https://business.gov.nl/running-your-business/legal-matters/how-to-comply-with-gdpr-rules-checklist/) and [processing records](https://business.gov.nl/regulations/drawing-up-a-processing-register/).
- [AP breach response guidance](https://autoriteitpersoonsgegevens.nl/themas/beveiliging/datalekken/datalek-dit-moet-u-doen).
- [Google privacy policy](https://policies.google.com/privacy); this does not establish the account's contractual arrangements.

Recheck current official guidance and the actual offering when updating the release record.
