# Procore feature research for a clean-room parity checklist
Public sources only, as of 2026-10-12. The report runs to roughly 5,500 words: it goes slightly over the 5,000 target because of the number of tools. I wrote no files.

## 0. Method and caveats
- **Sources.** Procore's public support site (support.procore.com and its v2 mirror), procore.com marketing, press and What's New pages, public press wires, and a few third-party pages. I did not log in, start a trial, use a sandbox, open the app, use the developer portal, or download any Procore software, templates or sample files.
- **Wording.** All behaviour is described in my own words. Status and setting names are recorded only to capture how each workflow works. We should pick our own labels, default lists, email wording, templates and colours.
- **Weights.** "W" is my own 1–10 estimate of everyday importance for a typical general contractor (GC). It is not a Procore figure.
- **Regional terms.** Procore renames things by region (it calls these "dictionaries"):
  - Punch list items appear as deficiency, defect or snag items ([en-gb tutorial](https://en-gb.support.procore.com/products/online/user-guide/project-level/punch-list/tutorials/retrieve-deleted-punch-list-items-from-the-recycle-bin)).
  - Change orders appear as variations, and the prime contract as a head or main contract ([Related Items FAQ](https://support.procore.com/faq/what-are-related-items-in-procore)).
  - Invoices appear as payment applications or progress claims ([sliding-scale tutorial](https://support.procore.com/products/online/user-guide/project-level/commitments/tutorials/enable-sliding-scale-retention-rules-on-subcontractor-invoices)).
  - We should build a terminology layer into our i18n from the start.
- **Pages I could not read fully:**
  - The old Project Scheduling URL returned 404, so I used the [v2 page](https://v2.support.procore.com/en-ca/product-manuals/scheduling-project/) instead.
  - The March and April 2026 weekly release-note pages returned 404, and the [index](https://support.procore.com/product-releases/weekly-release-notes) lists no months between Nov 2025 and Apr 2026.
  - The long landing pages for Budget, Commitments, Invoicing, Daily Log, Company Directory and Analytics were only read up to about 100,000 characters.
  - I could not read the AIA G702 instructions PDF as text, and I did not check omniclass.org/license. Those points rest on search snippets.
- **Conflicts between Procore's own pages:**
  - The Meetings page says deleted meetings can be restored; the Recycle Bin FAQ says they cannot.
  - The budget "Approved COs" definition is odd (see 4.2).
  - The Project Overview page labels itself beta, but a 7/2026 release note says it is generally available (GA).

## 1. Tools, grouped the way Procore groups them publicly
Procore's marketing site uses these groups:
- Preconstruction: Bid Management, BIM, Estimating, Prequalification ([preconstruction](https://www.procore.com/preconstruction)).
- [Project Management](https://www.procore.com/project-management).
- [Quality & Safety](https://www.procore.com/quality-safety).
- [Project Financials](https://www.procore.com/project-financials), plus [Invoice Management](https://www.procore.com/invoice-management).
- [Resource Management](https://www.procore.com/resource-management).
- A platform and AI layer: Insights, AI agents, 360 Reporting, Analytics 2.0, 500+ integrations and mobile apps ([platform](https://www.procore.com/platform)).

Directory, Documents, Tasks, Schedule, Daily Log, Photos and Canned Reports appear in several groups.

The support site lists 52 [project-level](https://support.procore.com/products/online/user-guide/project-level) and 22 [company-level](https://support.procore.com/products/online/user-guide/company-level) tools as flat lists. Which settings each tool has at company level versus project level is tabulated in [this FAQ](https://support.procore.com/faq/what-settings-are-available-for-project-tools-in-procore). Pricing is per product, with unlimited users and storage ([pricing](https://www.procore.com/pricing)).

### 1.1 Project Management

**RFIs — W9 (the design question-and-answer loop on every job).** Formal questions from the field to the design team, with tracked answers.
- **Statuses:** Draft, Open and Closed, and a closed RFI can be reopened. Closing directly from Draft gives a separate "closed-draft" state (3/2025). Deleted RFIs keep their status inside the recycle bin ([RFIs](https://support.procore.com/products/online/user-guide/project-level/rfi)).
- **Numbering:**
  - A number is assigned only when the RFI becomes Open.
  - Numbers are sequential, never duplicated, and not reused after a deletion.
  - Each project can set a prefix and a starting number ([numbering FAQ](https://support.procore.com/faq/how-does-procore-assign-numbers-to-rfis)).
  - An optional prefix by project stage cannot be switched off once enabled ([settings](https://support.procore.com/products/online/user-guide/project-level/rfi/tutorials/configure-advanced-settings-rfis)).
- **Roles:**
  - An RFI manager. The project can set a default manager, and Standard users can be allowed to pick an eligible one.
  - Assignees. Their responses can be required by default.
  - A distribution list, with a project-wide default distribution.
  - Users can shift the ball in court and forward an RFI for review.
  - One response can be marked "official". An option hides the other responses from Standard and Read Only users ([settings](https://support.procore.com/products/online/user-guide/project-level/rfi/tutorials/configure-advanced-settings-rfis), [RFIs](https://support.procore.com/products/online/user-guide/project-level/rfi)).
- **Due dates:** creation date plus a configurable number of days to answer, counted on the project's working days. Overdue RFIs trigger daily reminders, which stop at 45 days overdue ([settings](https://support.procore.com/products/online/user-guide/project-level/rfi/tutorials/configure-advanced-settings-rfis)).
- **Configure settings:**
  - Private RFIs can be enabled, and new RFIs can be private by default.
  - Custom fields (each limited to 255 characters).
  - An email matrix of events by role, with a reset-to-defaults option (same source).
- **Reply by email:** replying to the notification email posts the reply and its attachments as a response ([respond](https://support.procore.com/products/online/user-guide/project-level/rfi/tutorials/respond-to-an-rfi)).
- **Other:**
  - Revisions, change history and recycle bin.
  - CSV/PDF exports that keep the current filters, sort and columns (8/2025).
  - Saved views, bulk actions and custom reports.
  - From an RFI you can create a Change Event, a potential change order (PCO), an Instruction or a Correspondence item. RFIs can be pinned or linked on drawings.
  - A Draft RFI AI agent entered beta in 9/2025 ([RFIs](https://support.procore.com/products/online/user-guide/project-level/rfi)).

**Submittals — W9 (they gate procurement and installation).** Shop drawings, product data and samples routed through a review workflow, organised by spec division.
- **Statuses:** Draft, Open and Closed by default. Companies can add custom statuses, each mapped to open or closed. Workflow due dates stay frozen while a submittal is in Draft ([statuses FAQ](https://support.procore.com/faq/what-are-the-default-submittal-statuses-in-procore)).
- **Workflow:**
  - An ordered list of steps made up of submitters and approvers. Putting several people on one step makes that step parallel.
  - Ball in court moves to the next step once the required responders on the current step have answered ([workflow FAQ](https://support.procore.com/faq/how-do-i-add-people-to-the-submittal-workflow)).
  - Approvers can add reviewers. Workflows can be saved as templates.
  - An optional "reject workflow" sends rejections back to the submittal manager ([settings](https://support.procore.com/products/online/user-guide/project-level/submittals/tutorials/configure-settings-submittals-tool)).
- **Default settings** ([settings](https://support.procore.com/products/online/user-guide/project-level/submittals/tutorials/configure-settings-submittals-tool)):
  - A default submittal manager and a default distribution list.
  - Due in 14 working days.
  - A private-by-default option, which is off by default.
  - Overdue reminders, which stop at 45 days.
  - Dynamic approver due dates.
  - Optional schedule calculations that plan dates backwards from a required on-site date.
  - Attachments can optionally be downloaded without logging in, through links that expire after 14 days.
  - The "action required" email cannot be turned off.
- **Other** ([Submittals](https://support.procore.com/products/online/user-guide/project-level/submittals)):
  - Numbering, with an option to number by spec section.
  - Revisions, packages, custom types and custom responses.
  - Personal markup stamps for PDFs, QR codes and an approver response-time report.
  - Recycle bin, and a change history that records who was emailed.
  - Submittals can be imported.
  - An AI "Submittal Builder" drafts submittals from spec sections (3/2026).

**Drawings — W9 (field teams work from the current set).**
- **Upload:** you upload a set, OCR pre-fills each sheet's number and title, a reviewer checks and confirms the sheets, and the set is then published ([Drawings](https://support.procore.com/products/online/user-guide/project-level/drawings), [review and confirm](https://support.procore.com/products/online/user-guide/project-level/drawings/tutorials/review-and-confirm-drawings)).
- **Revisions:** a sheet whose number already exists becomes that sheet's next revision. Revisions are ordered by drawing date. Users see a "current" list plus an all-sets-and-revisions report in which obsolete sheets are shown grey ([view](https://support.procore.com/products/online/user-guide/project-level/drawings/tutorials/view-drawings), [all sets](https://support.procore.com/products/online/user-guide/project-level/drawings/tutorials/view-all-drawing-sets-and-revisions)).
- **Other** ([Drawings](https://support.procore.com/products/online/user-guide/project-level/drawings)):
  - Sets, disciplines (with a default list) and drawing areas.
  - Markups are either personal or published.
  - Item links on sheets, and automatic cross-links between sheets.
  - Revision comparison and sketches.
  - Download, print and email; export of the drawing log; subscriptions to log updates; QR codes.
  - Table modernization in 2026.
- **Cross-company sync:** drawings can be synced to another company's project through Procore Connect ([Connection Manager](https://support.procore.com/products/online/user-guide/project-level/connection-manager)).

**Specifications — W6** ([Specifications](https://support.procore.com/products/online/user-guide/project-level/specifications)).
- An uploaded spec book is split into divisions and sections using OCR. A reviewer checks it before it is published, and review and publish are separate permissions.
- Sets and areas; revisions are saved automatically.
- Comparison, either overlaid or side by side (8/2026).
- Public and private markups (8/2026), and pins linking items to sections (9/2026).
- Search, download, email and export.
- Items in its recycle bin cannot be restored ([Recycle Bin FAQ](https://support.procore.com/faq/what-is-procores-recycle-bin)).

**Documents — W8** ([Documents](https://support.procore.com/products/online/user-guide/project-level/documents)).
- **Features:**
  - A folder tree with permissions on folders and files, and private items.
  - Versions, with download of earlier versions.
  - Check-out and check-in locking, and tracking of who downloaded each file.
  - An offline upload queue on mobile.
  - Public and private markup layers, and version comparison (7/2026).
  - Items such as RFIs can be pinned to spots on files (9/2026).
  - A recycle bin with restore.
  - Project templates copy the folder structure but not the files inside the folders ([template FAQ](https://en-gb.support.procore.com/faq/what-gets-copied-over-to-a-new-project-when-applying-a-project-template)).
- **Document Management (a separate tool, available only in the UK, Ireland, Australia and New Zealand):**
  - Metadata and naming standards, with machine-learning metadata detection.
  - Upload requirements.
  - Revisions, statuses and a "stage" field.
  - Review workflows, packages and placeholders.
  - Positioned as supporting ISO 19650-style information management.
  - SharePoint links ([Document Management](https://support.procore.com/products/online/user-guide/project-level/document-management)).

**Daily Log — W9** ([Daily Log](https://support.procore.com/products/online/user-guide/project-level/daily-log)).
- **Log types:** weather, manpower, notes, timecards, equipment, visitors, phone calls, inspections, deliveries, safety violations, accidents, quantities, productivity, dumpsters, waste, scheduled work, delays and plan revisions.
- **Views:** a single day, a date range, or a calendar.
- **Daily workflow:**
  - Copying the previous day needs a granular permission.
  - A day can be marked complete and reopened, and entries cannot be deleted while it is complete.
  - Entries from collaborators stay pending until an admin approves or rejects them. Standard users edit only their own entries.
- **Recent additions:**
  - Segments allow several logs per day, by location, phase, crew or shift (8/2026).
  - Log types can be renamed.
  - Fieldsets and custom fields per log type.
  - Email, and export to PDF.

**Meetings — W7** ([Meetings](https://support.procore.com/products/online/user-guide/project-level/meetings)).
- Meetings have categories and items, and switch between agenda mode and minutes mode.
- Attendance is recorded.
- Open items carry over into follow-up meetings, and individual items can be excluded from carrying over.
- Company-level templates and PDF layouts.
- Agendas and minutes are distributed by email, and attendees can approve or reject the minutes.
- Tasks can be created from meeting items (4/2026).
- Change history and recycle bin.

**Photos — W8** ([Photos](https://support.procore.com/products/online/user-guide/project-level/photos)).
- **Organising:** albums; upload from web or mobile, or by inbound email; trade and location tags; a map view; private photos and albums; bulk edit.
- **On mobile:** markup, comments and @mentions; capture and viewing of 360° photos.
- **Other:** export to PDF, recycle bin, and AI summaries of selected photos.

**Schedule and Scheduling — W7.**
- **Schedule (legacy):** imports MS Project, Primavera P6 and MPX files. Gantt and calendar views, lookaheads, percent-complete updates, change requests and weekly schedule emails ([Schedule](https://support.procore.com/products/online/user-guide/project-level/schedule)).
- **Procore Scheduling (GA 2/17/2026):** schedules can be built from scratch, with lookahead plans, real-time multi-user editing, and assignment of responsibility ([Scheduling](https://v2.support.procore.com/en-ca/product-manuals/scheduling-project/)).
- A competing vendor claims Procore Scheduling has no computed float or critical path and cannot export native schedule formats. This is unverified ([Planera](https://www.planera.io/post/procore-alternatives)).

**Transmittals — W4** ([Transmittals](https://support.procore.com/products/online/user-guide/project-level/transmittals)).
- A letter-style record of what was sent to whom, with To and CC fields and attached or related items.
- Rules for who can see private transmittals; Standard users can edit only their own.
- Email forwarding, export of the letter as a PDF, log export, and recycle bin.

**Correspondence — W6** ([Correspondence](https://support.procore.com/products/online/user-guide/project-level/correspondence)).
- **Types:** companies define their own correspondence types. Each type has its own fieldset, custom fields and custom statuses. Statuses map to draft, open or closed, plus closed-successful and closed-unsuccessful for reporting.
- **Behaviour:**
  - A closed item cannot receive responses.
  - Distribution lists and an email matrix.
  - Private items, and a stricter "super private" type.
  - Due dates, and a filter for overdue items.
- **Other:** workflows, DocuSign, archiving, optional locking of items once they leave Draft, and change history.

**Emails — W4.** A project mailbox with an inbound address that anyone, including non-users, can write to. Private or public emails, tags, archive, spam marking, recycle bin and PDF export. A Change Event can be created from an email ([Emails](https://support.procore.com/products/online/user-guide/project-level/emails)).

**Tasks — W5.**
- Statuses: Initiated, In Progress, Ready for Review, Closed and Void.
- Multiple assignees, a default distribution list, private tasks and company-level categories.
- Related items and email notifications; granular permissions were added in 1/2026 ([Tasks](https://support.procore.com/products/online/user-guide/project-level/tasks)).
- Tasks are now the single home for meeting action items (8/2026) ([Meetings](https://support.procore.com/products/online/user-guide/project-level/meetings)).

**Forms — W5.** Fillable-PDF templates set up at company level, filled in on web or mobile. Private forms, email, export and recycle bin ([Forms](https://support.procore.com/products/online/user-guide/project-level/forms)).

**Conversations — W5** ([Conversations](https://support.procore.com/products/online/user-guide/company-level/conversations)).
- Direct messages, and groups of up to 2,000 people that can sync with the project directory.
- Conversations attached to items in Budget, Change Events, Commitments, Observations, Punch List, RFIs and Submittals.
- No permission set of its own; access follows the underlying items.

**Project Overview, Procore Home and My Open Items — W8.**
- The project landing page shows open items, quick-create, weather, timeline and team. It became GA in 7/2026, replacing Project Home and Portfolio ([Project Overview](https://support.procore.com/products/online/user-guide/project-level/project-overview)).
- My Open Items, a cross-project to-do list, gained grouping and saved views on 10/13/2026 ([What's New](https://www.procore.com/whats-new)).

**Connection Manager (Procore Connect) — W4.** Links an upstream project to a downstream one, including across company accounts. What syncs as read-only copies: drawings (GA 10/2024), closed RFIs (GA 1/2026) and submittals (beta 4/2026). Connections need a request and an approval, and can be quickly undone ([Connection Manager](https://support.procore.com/products/online/user-guide/project-level/connection-manager)).

**Models and Coordination Issues — W4** ([Coordination Issues](https://support.procore.com/products/online/user-guide/project-level/coordination-issues)).
- A 3D model viewer, plus a log of BIM coordination issues.
- Issues come from the viewer, from plugins, or from BCF import, with up to four viewpoints each.
- Assignees and watchers; issues can be escalated to an RFI or an Observation; recycle bin.
- Private issues were added in 6/2026 ([June 2026](https://support.procore.com/product-releases/weekly-release-notes/june-2026)), and custom types and priorities in 7/2026 ([July 2026](https://support.procore.com/product-releases/weekly-release-notes/july-2026)).

**Instructions — W3 (Australia, New Zealand and Canada only).** Architect, client and site instructions. Related items, and a change event can be created from an instruction ([Instructions](https://support.procore.com/products/online/user-guide/project-level/instructions)).

### 1.2 Quality & Safety

**Punch List — W8 (critical at closeout; heavy mobile use).**
- **Workflow:**
  - Item states: Draft, Work Required, Ready for Review, Ready to Close and Closed, with Not Accepted and In Dispute branches.
  - Each assignee responds separately, and the item stays in Work Required while any assignee still has work outstanding.
  - Items become overdue while work is outstanding or not accepted ([workflow FAQ](https://support.procore.com/faq/what-is-the-punch-list-work-flow)).
- **Roles:**
  - Creator, assignees and final approver.
  - Punch item manager: Admins, or Standard users granted the role.
  - If no final approver is set, the creator acts as final approver.
  - Who may close an item depends on their role and the item's state (same sources).
- **Configure settings** ([settings](https://support.procore.com/products/online/user-guide/project-level/punch-list/tutorials/configure-advanced-settings-punch-list)):
  - Defaults for punch item manager, final approver and distribution.
  - Private by default.
  - Due-date days, counted on working days.
  - Overdue emails to assignees, and an email matrix.
  - Punch types.
  - Templates carrying a default trade, assignee and approver.
- **Other:** drawing pins, multi-tier locations, import, bulk actions, PDF/CSV export, a recycle bin, and per-item change history ([Punch List](https://support.procore.com/products/online/user-guide/project-level/punch-list)).

**Inspections — W8** ([Inspections](https://support.procore.com/products/online/user-guide/project-level/inspections)).
- Checklist templates at company and project level, with conditional logic.
- Depending on the response, a line item can require a photo or an observation.
- Signatures on the whole inspection or on individual items.
- Inspection schedules, which can include equipment.
- Deficient items become observations, and closed inspections can be reinspected.
- Alphanumeric numbering and bulk create.
- Rules for who can see private inspections.
- Email, PDF, recycle bin and change history.

**Observations — W7.**
- **Types:** quality, safety, commissioning, warranty and work-to-complete.
- **Workflow:**
  - New observations start as Initiated; the assignee moves them to Ready for Review; the creator either sends them back as Not Accepted or Closes them.
  - Overdue emails go out only while an observation is Initiated or Not Accepted.
  - Assignee-only users can move an observation only to Ready for Review ([create](https://support.procore.com/products/online/user-guide/project-level/observations/tutorials/create-an-observation), [update status](https://support.procore.com/products/online/user-guide/project-level/observations/tutorials/update-the-status-of-an-observation)).
- **Other:** templates; a quality/safety split and granular permissions (11/2025); private observations; recycle bin; PDF/CSV export ([Observations](https://support.procore.com/products/online/user-guide/project-level/observations)).

**Incidents — W6.**
- **Records:** each incident can hold injury/illness, near-miss, property-damage and environmental records, plus witness statements and actions.
- **Settings:** configurable severity levels and alert recipients; a root-cause-analysis form (7/2026); private incidents; CSV/PDF export ([Incidents](https://support.procore.com/products/online/user-guide/project-level/incidents)). Incident numbers became editable in 6/2026 ([June 2026](https://support.procore.com/product-releases/weekly-release-notes/june-2026)).
- **OSHA:** the data model follows the content of OSHA forms 300, 300A and 301 ([Procore library](https://www.procore.com/library/construction-incident-report)), and records carry recordable and notifiable flags ([FAQ](https://support.procore.com/faq/how-can-i-use-procores-incidents-tool-to-record-coronavirus-covid-19-infection-information)). I found no evidence that it exports the OSHA forms themselves.

**Action Plans — W4** ([Action Plans](https://support.procore.com/products/online/user-guide/project-level/action-plans)).
- Milestone plans such as inspection and test plans (ITPs), QA plans, pour plans, start-up plans and safety plans, built from company or project templates.
- Plans have sections and items.
- Roles: creator, assignee, approver, receiver and plan manager.
- Verification methods and required records.
- Signatures, with optional blocking until signed.
- PDF export and recycle bin.

### 1.3 Project Financials (details in section 4)
- Budget — W9.
- Change Events — W8.
- Change Orders — W8.
- Commitments — W9.
- Prime Contracts (also labelled Funding or Client Contracts) — W8.
- Invoicing — W8.
- Direct Costs — W7.
- **T&M Tickets — W5.** Out-of-scope work tickets signed in the field, with labour, equipment and material entries. Signing works offline. Revise & Resubmit and Reject statuses were added in 6/2026. A ticket can be added to, or create, a change event ([T&M](https://support.procore.com/products/online/user-guide/project-level/tm-tickets)).
- **Workflows — W6.** See section 3.
- **Procore Pay — W4 (US only).** See 4.8.
- **ERP Integrations — W6.**
  - Connectors: Viewpoint Spectrum and Vista, QuickBooks Desktop and Online, Sage 300 CRE, Sage 100 Contractor and Sage Intacct, CMiC, NetSuite, Acumatica, Yardi, MRI, Workday and Xero ([ERP](https://support.procore.com/products/online/user-guide/company-level/erp-integrations)).
  - At least one user must hold an "Accounting Approver" role ([permissions FAQ](https://support.procore.com/faq/what-are-permissions-in-procore-and-how-do-they-work?mt-view=f1)).

### 1.4 Preconstruction
- **Bidding — W6** ([Bidding](https://support.procore.com/products/online/user-guide/project-level/bidding)).
  - Bid packages with documents, and optional NDAs (7/2025).
  - Bid forms, built manually or by CSV, with sections.
  - Bidders invited from the directory or from Procore's construction network.
  - A blind-bidding manager can show or hide submitted bids.
  - Bid leveling (GA 9/2025), with each vendor's history shown.
  - Awarding a bid turns it into a subcontract or purchase order.
  - Bid form and bidder list templates (4/2026); public project bidding entered beta (2/2026).
- **Bid Board — W4.** The subcontractor side: a pipeline of bid invitations, takeoff and estimating, proposal, and creating a project once awarded ([Bid Board](https://support.procore.com/products/online/user-guide/company-level/bid-board)).
- **Planroom — W3.** A portal where invited bidders view packages, state whether they will bid, download documents, submit or update bids, and ask pre-bid questions ([Planroom](https://support.procore.com/products/online/user-guide/company-level/planroom)).
- **Estimating — W5** ([Estimating](https://support.procore.com/products/online/user-guide/project-level/estimating)).
  - Takeoff, including auto-count, area and Revit-model takeoff.
  - Cost catalog items and assemblies, plus adjustments.
  - Proposals.
  - An estimate can be sent to the budget, or used to create a prime contract or POs.
- **Prequalifications — W4.** A configurable questionnaire; companies are invited, submissions reviewed and commented on, change requests raised and statuses set. Subcontractors respond through a portal ([Prequalifications](https://support.procore.com/products/online/user-guide/company-level/prequalifications)).

### 1.5 Resource Management / Workforce
- **Timesheets — W7** ([Timesheets](https://support.procore.com/products/online/user-guide/project-level/timesheets)).
  - Crew time entry, with start/stop times or total hours, and break tracking.
  - Overtime rules (2/2025).
  - Multi-tier review and approval, and worker signatures on mobile.
  - A time-clock kiosk (beta).
  - Field production and labour budget-to-actual reports.
- **Crews, Timecard and My Time — W4.**
  - Crews are groups of workers.
  - Workers can be "contacts", meaning people without a login ([contacts FAQ](https://support.procore.com/faq/what-is-a-contact-in-procore-and-which-project-tools-support-the-concept)).
  - Timecard tracks time for the project management team ([PM page](https://www.procore.com/project-management)).
- **Resource Planning — W5** ([Resource Planning](https://support.procore.com/products/online/user-guide/company-level/resource-planning)).
  - People, projects, assignments, requests and time off.
  - Assignment boards and a Gantt view; forecasting dashboards.
  - Assignment alerts and messages by SMS or email.
  - Tags for certifications and skills.
- **Equipment — W5** ([Equipment](https://support.procore.com/products/online/user-guide/company-level/equipment)).
  - A register of owned and rented equipment, with custom statuses.
  - Assignment to projects, and maintenance logs.
  - Telematics for Caterpillar, John Deere and Samsara (GA 1/2026).
  - QR codes.
  - Used inside Daily Log, Timesheets, Inspections and T&M Tickets.
- **Materials — W3.** Material requirements, receipts, inventory, issuing and transfers, and defects. GA 6/2026 ([Materials](https://support.procore.com/products/online/user-guide/project-level/materials), [June 2026](https://support.procore.com/product-releases/weekly-release-notes/june-2026)).
- **Assets — W3.** An asset register running from specification through operations. Beta in 5/2026 ([May 2026](https://support.procore.com/product-releases/weekly-release-notes/may-2026)).

### 1.6 Company-level and platform
- **Company Directory, Project Directory and Permissions — W10 and W7.** See section 2.
- **Company Admin — W6** ([Company Admin](https://support.procore.com/products/online/user-guide/company-level/admin)).
  - Single sign-on (SSO), webhooks and app management.
  - The cost-code structure (work breakdown structure, WBS).
  - Project defaults: stages, types and regions.
  - Settings for each tool, plus fieldsets and custom fields.
  - Settings only a superuser can change, such as the dictionary and enabling project templates.
  - Sandboxes (6/2026) and multi-currency (11/2025).
- **360 Reporting — W6.** Custom reports across tools and projects, single-tool reports and dashboards; reports can be shared and snapshots distributed. Two permission layers apply: permission to report, and permission to the source data ([360 Reporting](https://support.procore.com/products/online/user-guide/company-level/reports)).
- **Analytics 2.0 — W4.** Power BI reports; data access through Databricks, Delta Sharing or a command-line tool; hourly refresh; row-level security ([Analytics](https://support.procore.com/products/online/user-guide/company-level/analytics)).
- **Procore AI — W4.** "Assist" answers questions about project data, builds reports and answers help questions; agents are covered in section 5 ([Assist](https://support.procore.com/products/online/user-guide/project-level/assist)).
- **Mobile — W10.** iOS and Android apps. Items already cached can be used offline, and changes sync later ([offline FAQ](https://v2.support.procore.com/faq-can-i-use-procores-mobile-application-offline)).
- **Integrations.** More than 500 partner integrations, and an App Marketplace ([platform](https://www.procore.com/platform)).

## 2. Permission model and Directory

**Scopes.** Company-level tools have one instance per account. Project-level tools hold separate data for each project. Some tools, such as Directory, exist at both levels ([permissions FAQ](https://support.procore.com/faq/what-are-permissions-in-procore-and-how-do-they-work?mt-view=f1)).

**General levels, one per tool per user** (same source):
- **None:** the tool is hidden from the user.
- **Read Only:** can view.
- **Standard:** can view and work in the tool, but not configure it.
- **Admin:** full control, including configuration.

There are separate permission matrices for web and mobile.

**Granular permissions.** These add specific abilities on top of Read Only or Standard. They never apply to None or Admin (same source). Examples:
- Directory: create and edit users ([permissions FAQ](https://support.procore.com/faq/what-are-permissions-in-procore-and-how-do-they-work?mt-view=f1)).
- RFIs: act as RFI manager, and mark official responses ([RFIs](https://support.procore.com/products/online/user-guide/project-level/rfi)).
- Daily Log: copy a log, and "collaborator entry only" ([Daily Log](https://support.procore.com/products/online/user-guide/project-level/daily-log)).
- Documents: access private files ([Documents](https://support.procore.com/products/online/user-guide/project-level/documents)).
- Viewing change history ([change event history](https://v2.support.procore.com/product-manuals/change-events-project/tutorials/View_the_Change_History_of_a_Change_Event/)).
- Procore keeps adding granular permissions tool by tool: Specifications (7/2025), Observations (11/2025), Tasks (1/2026) and Equipment (3/2026) ([Specifications](https://support.procore.com/products/online/user-guide/project-level/specifications), [Observations](https://support.procore.com/products/online/user-guide/project-level/observations), [Tasks](https://support.procore.com/products/online/user-guide/project-level/tasks), [Equipment](https://support.procore.com/products/online/user-guide/company-level/equipment)).

**Role-based privileges.** A third layer that sits outside permission templates and is assigned on the user's directory record, for example Accounting Approver for ERP ([permissions FAQ](https://support.procore.com/faq/what-are-permissions-in-procore-and-how-do-they-work?mt-view=f1)). Similar tool-specific roles:
- Payments Admin, Disburser and Beneficiary Approver in Procore Pay ([Pay](https://support.procore.com/products/online/user-guide/company-level/payments)).
- Blind Bidding Manager ([Bidding](https://support.procore.com/products/online/user-guide/project-level/bidding)).
- Designated Reviewer for change orders ([Change Orders](https://en-ca.support.procore.com/products/online/user-guide/project-level/change-orders)).

**Item-level rules.** Standard users are typically limited to:
- Editing or deleting only what they created ([Transmittals](https://support.procore.com/products/online/user-guide/project-level/transmittals), [T&M](https://support.procore.com/products/online/user-guide/project-level/tm-tickets), [Change Events](https://support.procore.com/products/online/user-guide/project-level/change-events)).
- Acting only when they are the item's manager, an assignee or on its distribution list ([RFIs](https://support.procore.com/products/online/user-guide/project-level/rfi)).

**Permission templates** ([template FAQ](https://support.procore.com/faq/what-is-a-permissions-template)):
- Company templates govern company tools, and project templates govern project tools.
- Project templates are either global, meaning assignable on any project, or project-specific. A project-specific template can be created from a project's directory but can then be edited only in the company Permissions tool.
- **Defaults:** Procore seeds a default set according to the company type (GC, owner, subcontractor or architect), with each template marked internal or external. The defaults can be edited but not deleted ([defaults FAQ](https://support.procore.com/faq/what-are-the-default-project-permissions-templates-in-procore)).
- **Assignable flag:** an "assignable" flag controls which templates non-admins can give to other users ([assignable FAQ](https://en-gb.support.procore.com/faq/what-is-an-assignable-permissions-template), [Permissions tool](https://support.procore.com/products/online/user-guide/company-level/permissions)).
- **History and redesign:** templates have their own change history. A redesigned permissions interface entered open beta on 10/13/2026 ([What's New](https://www.procore.com/whats-new)).

**Super-admin.** Admin on the Company Directory gives admin rights in every company and project tool. Users who have only granular rights cannot change company admins ([Permissions tool](https://support.procore.com/products/online/user-guide/company-level/permissions)).

**Directory.**
- **Company Directory** ([Company Directory](https://support.procore.com/products/online/user-guide/company-level/directory)):
  - Holds users, contacts and companies.
  - Users can be invited or re-invited, deactivated and reactivated in bulk, and unlocked.
  - Companies can be merged, and carry insurance records and bidder information.
  - Companies from Procore's construction network can be added.
  - Imports, and CSV/PDF export with saved views.
  - Each user has a change log, which includes project changes.
  - Users can be assigned to existing or future projects, in bulk since 3/2026.
- **Project Directory:** the people and companies on a project, each with a project template and project roles. Distribution groups can be added to it ([tutorial](https://support.procore.com/products/online/user-guide/project-level/directory/tutorials/add-a-distribution-group-to-the-project-directory)).
- **Contacts:** people without a login, such as craft workers or visitors. They are created from Incidents or Crews and can only be deactivated, never deleted ([contacts FAQ](https://support.procore.com/faq/what-is-a-contact-in-procore-and-which-project-tools-support-the-concept)).
- **Distribution groups vs distribution lists:**
  - A distribution group is a named set of users that can fill a role (such as assignee) or be added to a distribution list.
  - A distribution list is the per-item, cc-style set of people who get notified ([FAQ](https://support.procore.com/faq/what-is-the-difference-between-a-distribution-group-and-a-distribution-list)).
  - The Canadian version of the page says groups have no security effect.

## 3. Cross-cutting behaviours

**Change History.** A per-item audit trail that cannot be edited, recording date, user, the change, and the old and new values ([commitment](https://support.procore.com/products/online/user-guide/project-level/commitments/tutorials/view-the-change-history-of-a-commitment), [change event](https://v2.support.procore.com/product-manuals/change-events-project/tutorials/View_the_Change_History_of_a_Change_Event/)). It exists in RFIs, Punch List, Inspections, Meetings, Prime Contracts and permission templates, among others.

**Recycle Bin** ([FAQ](https://support.procore.com/faq/what-is-procores-recycle-bin)).
- One per tool. Deleted items are never permanently removed, and only that tool's admins can see them.
- Restore works in most tools, but not in Commitments, project Equipment, Meetings or Specifications. The Meetings page itself contradicts this.

**Related Items.** A generic link between any two objects across tools ([FAQ](https://support.procore.com/faq/what-are-related-items-in-procore)). Also links inside drawing markups, and pins on documents and specs (9/2026) ([Sept 2026](https://support.procore.com/product-releases/weekly-release-notes/september-2026)).

**Private items.**
- The pattern is a tool setting to enable private items, plus an optional "private by default".
- Private items are visible to the creator, assignees, manager, distribution list and admins, plus anyone with a granular permission to see private items from their own company ([RFI settings](https://support.procore.com/products/online/user-guide/project-level/rfi/tutorials/configure-advanced-settings-rfis), [Punch settings](https://support.procore.com/products/online/user-guide/project-level/punch-list/tutorials/configure-advanced-settings-punch-list), [Transmittals](https://support.procore.com/products/online/user-guide/project-level/transmittals)).
- Contracts use explicit access lists instead ([Prime Contracts](https://support.procore.com/products/online/user-guide/project-level/prime-contracts)).

**Email notifications.**
- Each tool has a matrix of events by item roles that can be reset to defaults, a default distribution list, and overdue reminders.
- Some notifications cannot be turned off ([Submittal settings](https://support.procore.com/products/online/user-guide/project-level/submittals/tutorials/configure-settings-submittals-tool)).
- The person who performed an action is not emailed about it ([Correspondence FAQ](https://v2.support.procore.com/faq-who-receives-correspondence-item-emails-and-push-notifications)).
- Schedule notifications are set per user rather than per tool ([Schedule](https://support.procore.com/products/online/user-guide/project-level/schedule)).

**Email-in and reply-by-email.**
- Replying to an RFI email posts a response ([respond](https://support.procore.com/products/online/user-guide/project-level/rfi/tutorials/respond-to-an-rfi)).
- The Emails tool has an inbound address ([Emails](https://support.procore.com/products/online/user-guide/project-level/emails)), and Photos accepts inbound email ([Photos](https://support.procore.com/products/online/user-guide/project-level/photos)).
- I found no public confirmation of reply-by-email for submittals.

**Configurable fieldsets and custom fields.**
- Each field can be set as required, optional or hidden. This is set at company level, with one fieldset per tool or per item type ([fieldsets FAQ](https://v2.support.procore.com/faq-what-are-configurable-fieldsets-and-which-procore-tools-support-them)). Supported in about 24 tools.
- **Field types** ([types FAQ](https://v2.support.procore.com/en-au/faq-what-field-types-are-available-for-custom-fields-in-procore-tools)):
  - Checkbox, number, short text, rich text and read-only text.
  - Company, date and date-time, file uploads, and location.
  - Single-select and multi-select.
  - Project user (single or multiple) and tool user.
- **Limits:** between 10 and 30 custom fields per tool ([custom fields FAQ](https://support.procore.com/faq/what-are-custom-fields-and-which-procore-tools-support-them)).
- Conditional custom fields arrived in 2/2026 ([Company Admin](https://support.procore.com/products/online/user-guide/company-level/admin)).
- Not every custom field shows on mobile.

**Configurable workflows (the Workflows tool)** ([Workflows](https://support.procore.com/products/online/user-guide/company-level/workflows)).
- **Templates:** diagram templates and linear templates (linear in beta, 2026).
- **Steps:**
  - Template roles are mapped to project roles.
  - A response step can have several assignees, with a rule for how they reach a decision.
  - Each step has a days-to-complete setting.
  - A workflow can be returned to an earlier step or restarted.
- **Routing:** conditional routing, for example on budget-code overruns, stored materials or custom fields.
- **People:** assignees can be replaced permanently or temporarily, or in bulk across projects (6/2026). There is a commenter role (8/2026).
- **Other actions:** webhook actions (5/2026); a step override with a required reason (9/2026) ([What's New](https://www.procore.com/whats-new)); history export.
- **Objects covered:** commitments, prime contracts and owner invoices, change orders, correspondence, observations and Document Management (same source).

**Lists and exports.** Search and filter, saved views, column choice, bulk actions, list exports to CSV/PDF that honour the current filters, single-item PDFs, and custom reports per tool ([RFIs](https://support.procore.com/products/online/user-guide/project-level/rfi)).

**Locations** ([FAQ](https://support.procore.com/faq/what-are-multi-tiered-locations)).
- Each project has a location tree with as many tiers as needed, shared by many tools.
- Users can optionally create locations from inside other tools; renaming a location updates it everywhere; imports allow up to six tiers.
- The Project Map draws locations and shows pins coloured by status (9/2026) ([Sept 2026](https://support.procore.com/product-releases/weekly-release-notes/september-2026)).

**Project templates** ([FAQ](https://en-gb.support.procore.com/faq/what-gets-copied-over-to-a-new-project-when-applying-a-project-template)).
- Copied: tool settings, which tools are active, the folder structure, users who hold global templates, and commitments (as drafts).
- Not copied: logs, drawings, and files inside folders.

**Mobile and offline.** Cached items stay available offline and edits sync later ([FAQ](https://v2.support.procore.com/faq-can-i-use-procores-mobile-application-offline)). Workflows work offline since 9/2026 ([Sept 2026](https://support.procore.com/product-releases/weekly-release-notes/september-2026)).

**Localization and hosting.**
- Italian and Traditional Chinese entered open beta in 7/2026 ([July 2026](https://support.procore.com/product-releases/weekly-release-notes/july-2026)).
- Multi-currency ([currency](https://support.procore.com/products/online/user-guide/project-level/admin/tutorials/configure-project-currency-settings)).
- Data hosting zones: a UK zone already exists, and an EU zone was added 10/5/2026 ([What's New](https://www.procore.com/whats-new)).

## 4. Financial specifics

### 4.1 Cost codes and WBS
- A budget code is made up of a cost code (tiered) plus a cost type (flat), an optional sub-job (enabled per project), and up to ten custom segments in any order ([WBS](https://v2.support.procore.com/product-manuals/admin-company/tutorials/about-work-breakdown-structure)).
- The default cost-code list follows the CSI MasterFormat division structure ([budget code FAQ](https://support.procore.com/faq/what-is-a-budget-code-in-procores-wbs)). See the IP note in section 6.
- The default cost types are a short list of generic job-cost categories (labour, materials, equipment, subcontract, and so on). They cannot be deleted but can be added to ([cost types FAQ](https://v2.support.procore.com/en-ca/faq-what-are-procores-default-cost-types)). We should define our own defaults.
- Budget code attributes allow grouping (GA 8/2025) ([Budget](https://support.procore.com/products/online/user-guide/project-level/budget)).
- Multi-currency works at company, project and individual-object level, with an exchange rate that is either synced to the company rate or fixed ([currency](https://support.procore.com/products/online/user-guide/project-level/admin/tutorials/configure-project-currency-settings)).

### 4.2 Budget
- **Basics** ([Budget](https://support.procore.com/products/online/user-guide/project-level/budget)):
  - Line items per budget code, CSV import, and locking and unlocking.
  - "Budget changes" are the current mechanism (they gained a type field in 2/2026); "budget modifications" are legacy.
  - Budget views are defined at company level.
  - Columns are standard, source (pulling from other objects by status, with advanced filters) or calculated.
- **Standard view logic** (paraphrased from [standard view](https://support.procore.com/products/online/user-guide/project-level/budget/tutorials/about-the-procore-standard-budget-view) and [read a budget](https://support.procore.com/products/online/user-guide/project-level/budget/tutorials/read-a-budget)):
  - Revised budget = budget modifications + approved change orders.
  - Projected budget = revised budget + pending budget changes (pending prime-contract COs).
  - Committed costs = approved or complete subcontracts + approved POs + approved commitment COs.
  - Direct costs = direct costs that are pending, in revise-and-resubmit, or approved.
  - Job-to-date costs = direct costs + subcontractor invoices.
  - Pending cost changes = subcontracts out for signature + POs in progress + pending COs.
  - Projected costs = committed costs + direct costs + pending cost changes.
  - Forecast to complete = projected budget minus costs to date.
  - Estimated cost at completion = projected costs + forecast to complete.
  - Projected over/under = projected budget minus estimated cost at completion.
  - A row turns red when any calculated value in it is negative.
- **Caveat:** Procore's page says "Approved COs" counts commitment change orders by default, with prime-contract COs as an opt-in. Yet the pending column counts prime-contract COs. We should define our own semantics explicitly.
- **Other** ([Aug 2026](https://support.procore.com/product-releases/weekly-release-notes/august-2026)):
  - Snapshots and their comparison.
  - Project status snapshots with custom statuses.
  - Forecasting views and forecasting curves.
  - "Columns Studio", which lets one column definition be reused across views (8/2026).
  - Budget reports and ERP export.

### 4.3 Change Events and Change Orders
- **Change events** ([CE](https://support.procore.com/products/online/user-guide/project-level/change-events)):
  - Statuses: Open, Pending, Closed and Void ([statuses](https://support.procore.com/faq/what-are-the-default-statuses-for-change-events-in-procore)).
  - Scope: in scope, out of scope, or TBD ([scope](https://v2.support.procore.com/en-au/faq-what-are-the-default-scope-options-for-change-events-in-procore)).
  - A change reason.
  - Line items per budget code, with revenue and cost rough order of magnitude (ROM) values and latest price and cost.
  - Requests for quote (RFQs) sent to vendors, who quote as collaborators. ROM and RFQ values can feed budget views.
  - A change event can create budget changes, prime-contract PCOs and change orders, commitment PCOs and change orders, or new contracts.
  - Change events can originate from RFIs, T&M tickets, instructions and emails.
- **Tiers** ([tiers FAQ](https://v2.support.procore.com/en-au/faq-what-are-the-different-change-order-tier-settings-in-project-financials)):
  - Prime contracts default to 2-tier: PCOs grouped into a prime contract change order.
  - Commitments default to 1-tier.
  - 3-tier adds a change order request in between.
  - The tier must be chosen before the first change order and is then locked. When Change Events is on, the first tier must start from a change event.
- **Change order statuses** ([CO statuses](https://support.procore.com/faq/what-are-the-default-statuses-for-change-orders-in-procore)):
  - Draft; several pending sub-states (in review, revised, pricing, not pricing, proceeding, not proceeding); approved; rejected; no charge; void (void exists only on PCOs).
  - Pending states feed the pending column and approved feeds the approved column; the others do not affect the budget.
  - A "pending – billable" status (10/2025) lets unapproved single-tier COs be billed ([Change Orders](https://en-ca.support.procore.com/products/online/user-guide/project-level/change-orders)).
- **Approvals:** Standard users can approve a commitment CO only as its designated reviewer. A DocuSign signature-status column is available (same source).
- **Markup:** horizontal markup is applied per line item, and vertical markup on the subtotal ([markup](https://v2.support.procore.com/product-manuals/prime-contracts-project/tutorials/add-financial-markup-to-prime-contract-change-orders)). Compounded markup, import from another prime contract, and copying from templates were added in 2026 ([Prime Contracts](https://support.procore.com/products/online/user-guide/project-level/prime-contracts)).

### 4.4 Commitments (subcontracts and purchase orders)
- **Subcontract statuses:** draft, out for bid, out for signature, approved, complete, terminated and void ([statuses](https://support.procore.com/faq/what-are-the-default-commitment-statuses-in-procore)).
- **PO statuses:** draft, processing, submitted, partially received, received, approved and closed.
- **Budget effect:** "in progress" states (out for signature, processing, submitted, received) feed pending cost changes; approved and complete feed committed costs.
- **Schedule of values (SOV)** ([Commitments](https://support.procore.com/products/online/user-guide/project-level/commitments)):
  - Line items per budget code, entered manually or from CSV (including tax codes).
  - An "always editable SOV" setting.
  - A detailed subcontractor SOV, which the subcontractor creates by invitation.
- **Other** ([Commitments](https://support.procore.com/products/online/user-guide/project-level/commitments)):
  - Retainage can be enabled per commitment.
  - Default privacy plus per-commitment access lists.
  - DocuSign.
  - Invoice contacts, and "invite to bill".
  - Payment schedules and payments issued.
  - ERP acceptance.
  - Up to 30 custom fields ([custom fields FAQ](https://support.procore.com/faq/what-are-custom-fields-and-which-procore-tools-support-them)).
- Winning bids can be awarded straight into commitments ([Bidding](https://support.procore.com/products/online/user-guide/project-level/bidding)).

### 4.5 Prime Contracts
- **Statuses:** draft, out for bid, out for signature, approved, complete and terminated. A contract must be approved or complete before it can have prime contract change orders or owner invoices ([edit prime](https://v2.support.procore.com/en-ca/product-manuals/prime-contracts-project/tutorials/edit-prime-contracts)).
- **Features** ([Prime Contracts](https://support.procore.com/products/online/user-guide/project-level/prime-contracts)):
  - The SOV comes from CSV, manual entry or the budget.
  - PCOs, change order requests and prime contract change orders, with markup.
  - Payments received.
  - A private access list.
- **Exports:** PDF for Standard and Read Only users; DOCX or PDF for Admins.

### 4.6 Invoicing and payment applications
- **Two directions:** owner invoices go upstream against the prime contract; subcontractor invoices come in against commitments. Permissions come from the Commitments and Prime Contracts tools, and specialty contractors see the tool as "Progress Billings" ([Invoicing](https://support.procore.com/products/online/user-guide/project-level/invoicing)).
- **Billing periods and prefill:**
  - Billing periods are created manually or automatically.
  - Owner invoices can be prefilled from approved subcontractor invoices, change orders and approved direct costs. Users can select, split or mark costs non-billable (5/2026) ([May 2026](https://support.procore.com/product-releases/weekly-release-notes/may-2026), [direct cost](https://support.procore.com/products/online/user-guide/project-level/direct-costs/tutorials/create-a-direct-cost)).
- **Statuses** ([invoice statuses](https://support.procore.com/faq/what-are-the-default-statuses-for-procore-invoices)):
  - Owner invoices: draft, under review, revise and resubmit, approved.
  - Subcontractor invoices add "approved as noted" and "pending owner approval".
  - Approved subcontractor invoices count toward job-to-date costs.
  - "Invite to bill" has its own tracking states: invited, accepted, declined, no contacts, and SOV not approved.
- **Progress billing:** each line shows work completed, stored materials and percent complete. Stored materials can roll forward automatically (GA 2/2025) ([Invoicing](https://support.procore.com/products/online/user-guide/project-level/invoicing)).
- **Retainage:**
  - Set per line, and released as a percentage of the total held ([retainage](https://v2.support.procore.com/product-manuals/invoicing-project/tutorials/set-or-release-retainage-on-a-subcontractor-invoice)).
  - Sliding-scale retention allows up to five rules, keyed to the original value, the revised value or the amount held. The rules reduce new withholding but do not automatically release retention held in earlier periods ([sliding scale](https://support.procore.com/products/online/user-guide/project-level/commitments/tutorials/enable-sliding-scale-retention-rules-on-subcontractor-invoices)).
- **Approvals and output:** approval workflows can route on dollar thresholds, stored materials or custom fields. Backup documents can be compiled automatically, and a custom PDF builder controls what upstream invoices show ([Invoice Management](https://www.procore.com/invoice-management)).

### 4.7 Direct Costs
- **What it tracks:** costs outside contracts, of three types: invoice, expense and payroll.
- **Statuses:** draft, pending, revise and resubmit, approved.
- **Budget effect:** any status except draft flows into the budget's direct-costs column. Approved lines with a matching budget code flow onto the owner invoice SOV ([create](https://support.procore.com/products/online/user-guide/project-level/direct-costs/tutorials/create-a-direct-cost)).
- **Other:** import, export, email and self-serve workflows ([Direct Costs](https://support.procore.com/products/online/user-guide/project-level/direct-costs)).

### 4.8 Procore Pay (US only)
- Payor and payee roles; disbursements through partner banks.
- **Lien waivers:** templates, generation, signing and eNotarization.
- **Payment controls:** compliance requirements that gate payment; manual holds.
- **Other:** joint checks; Early Pay (beta, 4/2026); bank-account verification (8/2026) ([Pay](https://support.procore.com/products/online/user-guide/company-level/payments), [May 2026](https://support.procore.com/product-releases/weekly-release-notes/may-2026)).

## 5. New in 2025–2026 worth tracking

**AI**
- Assist became GA in North America on 3/13/2025 and was renamed from Copilot in 6/2025 ([Assist](https://support.procore.com/products/online/user-guide/project-level/assist)).
- Groundbreak 2025 (10/2025): Agent Builder in open beta, plus photo intelligence and Spanish and Polish for Assist ([press](https://www.businesswire.com/news/home/20251015796723/en/Procore-Advances-the-Future-of-Construction-with-New-AI-Innovations-at-Groundbreak-2025)).
- Procore acquired Datagrid on 1/20/2026 ([press](https://www.businesswire.com/news/home/20260120797748/en/Procore-Acquires-Datagrid-to-Accelerate-AI-Strategy-and-Deliver-Enhanced-Data-Integration-for-Customers)).
- 5/21/2026: embedded agents for deep search, submittal review, RFIs, daily logs and contract review. Agents can take actions and respond to triggers, and a person approves before anything changes ([press](https://www.procore.com/press/new-procore-ai-experience-embeds-datagrid-into-procore)).
- 7/2026: Starter, Pro and Enterprise "Digital Coworker" tiers, with credit budgets and scheduled or event-triggered agents ([July 2026](https://support.procore.com/product-releases/weekly-release-notes/july-2026)).
- 8/19/2026: a pack for specialty contractors that adds schedule-analysis and site-safety agents ([What's New item](https://www.procore.com/whats-new/procore-ai-digital-coworker-starter-pack-now-available-for-specialty-contractors)).

**Financials**
- Multi-currency (11/2025).
- Billable but unapproved change orders, "pending – billable" (10/2025).
- Project status snapshots (8/2025) and Columns Studio (8/2026).
- Configurable fieldsets on owner invoices (5/2026).
- Bid leveling GA (9/2025), and a bidding–estimating integration in beta (1/2026) ([Budget](https://support.procore.com/products/online/user-guide/project-level/budget), [Invoicing](https://support.procore.com/products/online/user-guide/project-level/invoicing), [Bidding](https://support.procore.com/products/online/user-guide/project-level/bidding)).

**Project management and field**
- Procore Scheduling GA (2/2026).
- Procore Connect for RFIs GA (1/2026), and for submittals in beta (4/2026).
- Daily Log segments (8/2026).
- In the shared document viewer: public and private markups, comparison and item pins (7–9/2026).
- Project Archive GA (7/2026).
- Project Overview GA (7/2026) (sources: tool pages linked above).

**Quality and safety, workforce**
- Inspections: reinspection (8/2025) and bulk create (10/2025).
- Incidents: root-cause-analysis form (7/2026).
- Equipment telematics GA (1/2026), Materials GA (6/2026) and Assets beta (5/2026).

**Platform**
- Bulk replacement of workflow assignees (6/2026), linear workflow templates, and the step override (9/2026).
- Redesigned permissions, My Open Items grouping, and the EU hosting zone (10/2026) ([What's New](https://www.procore.com/whats-new)).
- Public webhooks (5/2026) and on-demand sandboxes (6/2026).

**Owners**
- 6/17/2026: Owners Hub, Portfolio Monitoring, Concept Projects and Asset Management, with GA planned for summer 2026. Capital Planning and Funding Source Management have GA planned for late 2026 ([press](https://www.procore.com/press/procore-introduces-new-suite-of-portfolio-management-and-capital-planning-capabilities-for-owners-powered-by-procore-ai)).

**Watch next:** Groundbreak 2026, October 20–22 ([preview](https://www.constructionowners.com/news/procore-groundbreak-2026-the-need-to-know-guide-for-construction-owners)).

## 6. Generic vocabulary and third-party IP

**Generic industry terms.** This is common construction vocabulary, not tied to any one source, and safe to use:
- RFI; submittal (shop drawing, product data, sample); transmittal.
- Drawing set, sheet, revision and discipline; specification section.
- Daily report; punch, snag or deficiency list; observation; inspection checklist; inspection and test plan (ITP).
- Incident, near miss, recordable injury.
- Meeting minutes and action items.
- Schedule, lookahead, critical path, baseline.
- Change event, potential change order, change order request, change directive, change order.
- ROM, RFQ, T&M ticket.
- Schedule of values; prime or owner contract; subcontract; purchase order; commitment.
- Original and revised budget, committed cost, job-to-date cost, cost to complete, estimate at completion, over/under.
- Cost code, cost type, WBS.
- Retainage or retention; progress billing or pay application; stored materials.
- Conditional and unconditional lien waivers, partial and final.
- Certificate of insurance; prequalification; bid package; bid leveling.
- Closeout, O&M manuals, as-builts, warranty.

Avoid Procore's product coinages such as "Procore Connect", "360 Reporting" and "Digital Coworker".

**Third-party material to avoid copying**
- **AIA G702/G703.** These are copyrighted documents. Copying blank forms, or putting AIA text into a non-AIA document, needs the AIA's written permission ([AIA FAQ](https://help.aiacontracts.com/hc/en-us/articles/1500009295161-faqs-copying-aia-contract-documents)). According to a search snippet of the G702 instructions, buyers may make only a limited number of copies of a completed form for one project ([instructions](https://documentsondemand.aia.org/GeneratedFiles/G702-1992_Instr.pdf)).
  - Recommendation: design our own payment-application summary and continuation sheet, with original layout and wording, and do not use AIA form names as labels without legal review.
- **CSI MasterFormat.** It is copyrighted, and use is governed by an end-user licence. Any commercial derivative product that uses its numbers or titles must contact CSI ([CSI FAQ](https://www.csiresources.org/mf-faqs)). Procore's default cost codes follow it.
  - Recommendation: ship no MasterFormat numbers or titles; let customers import their own lists.
  - OmniClass is a CSI trademark with its own licence terms, which I did not verify ([CSI](https://www.csiresources.org/omniclass)).
  - Uniclass 2015 is described as free to use, but I did not verify the exact licence ([Uniclass](https://en.wikipedia.org/wiki/Uniclass)).
- **ISO 19650.** ISO forbids copying its content without permission ([ISO](https://www.iso.org/copyright.html)). Implement the concepts in our own words.
- **ConsensusDocs.** Copyrighted; any use beyond internal review needs a paid subscription ([ConsensusDocs](https://www.consensusdocs.org/help-center/)).
- **OSHA 300, 300A and 301.** OSHA offers the forms for free and accepts equivalent forms in other formats if they contain the required information ([OSHA](https://www.osha.gov/recordkeeping/forms)). An OSHA-equivalent export from our incidents tool is therefore permissible.
