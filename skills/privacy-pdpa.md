---
name: privacy-pdpa
description: Personal data under Thailand's PDPA — what to collect (and not), the lawful basis and consent, privacy notices, sensitive data, retention and deletion, people's rights requests, breaches, data sent abroad, analytics cookies. Load whenever a form or feature collects names, phone numbers, emails, ID card numbers, photos, addresses or health data ("ข้อมูลส่วนบุคคล", "PDPA", "ยินยอม"), for registration/booking systems, and before launch. Technical protection lives in backend-security. Not legal advice.
area: both
---
# Personal data & PDPA (Thailand)

Practical rules for building features; for anything large or unusual tell the user to check with a lawyer or their organization's DPO. Say this once, plainly — don't hide behind it.

## 1. Collect less
- For each field ask: what is it for, and does the feature work without it? Drop the rest. Event registration rarely needs a national ID number, full birth date or home address.
- If an ID number is truly required: say why on the form, store it encrypted or masked (last 4 digits), restrict who can see it, delete it when no longer needed.
- **Sensitive data** (health, religion, ethnicity, biometrics such as face photos used for identity, criminal records, sexual orientation, union membership…) needs **explicit consent** or another narrow legal basis — avoid collecting it unless essential (e.g. food allergies for an event: optional, separate, explicit).

## 2. Lawful basis and consent
- Many things don't need consent: performing the booking the person asked for (contract), legal obligations (tax records). Consent is for extras — marketing, sharing with sponsors, photos used in promotion.
- Consent must be separate per purpose, not pre-ticked, not bundled with the terms, as easy to withdraw as to give; record who agreed to what version and when.
- Minors: younger people may need a parent's or guardian's consent — ask the age range of users if the product could involve students under 20.

## 3. Tell people (privacy notice)
At the point of collection, short and in plain Thai: what is collected, why, the basis, how long it is kept, who it's shared with (payment gateway, LINE, hosting abroad), their rights, and a contact. Link the full policy; design it into the form (`interaction-patterns` microcopy), not hidden in a footer.

## 4. Keep, protect, delete
- A retention period per kind of data (e.g. registrations deleted or anonymized 1 year after the event; accounting records kept as tax law requires) and a job that does it.
- Access by role, encryption at rest for sensitive fields, audit log of who viewed/exported personal data, no personal data in logs or analytics (`backend-security`, `reliability`).
- Staging and test data: anonymized copies, never a raw production dump on a laptop.
- Exports (CSV of attendees) are personal data too: only for roles that need them, logged, with a warning.

## 5. People's rights
Provide a way (even an email address and a process) to ask for access, correction, deletion, data portability, objection and withdrawal of consent; answer within the legal time limits (typically 30 days). Build "delete/anonymize this person" as a real function — it's needed anyway for retention.

## 6. Others who handle the data
Hosting, email, LINE, payment, analytics are processors or other controllers: note them in the notice; data stored or sent abroad (most cloud services) must go to places with adequate protection or under suitable safeguards — list the services and regions for the user.

## 7. When something leaks
A breach plan before launch: who decides, how to contain it, notifying the regulator (PDPC) within 72 hours when there's a risk to people, and telling affected people when the risk is high. Logs and backups make the investigation possible.

## 8. Cookies and analytics
Strictly necessary cookies need no consent; analytics and marketing cookies/pixels do — a banner with "reject" as easy as "accept", and the tags only after consent.

## 9. Before handing over
List the personal data collected with its purpose and basis, what was removed or made optional, where consent is asked and recorded, the retention and deletion job, who can access/export it, the processors involved, and the parts the user should confirm with a lawyer/DPO.
