import type {
  BusinessData,
  Currency,
  ExpenseDraft,
  SourceDocument,
  Workspace,
} from "./types";
import {
  approve,
  assertBalanced,
  blankDraft,
  correct,
  currentPeriod,
  id,
  now,
  saveDraft,
} from "./ledger";
import { commitImport, previewImport } from "./import";
import { PRODUCT_NAME } from "../config";
const escapeXML = (value: string) =>
  value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
function receipt(
  b: BusinessData,
  vendor: string,
  amount: string,
  date: string,
): SourceDocument {
  const xml = `<svg xmlns="http://www.w3.org/2000/svg" width="600" height="740" viewBox="0 0 600 740"><rect width="600" height="740" fill="#fff"/><g font-family="Arial,sans-serif" fill="#283a37"><text x="55" y="85" font-size="30" font-weight="700">${escapeXML(vendor)}</text><text x="55" y="118" font-size="13" fill="#7d8985">DEMONSTRATION RECEIPT · NOT A REAL INVOICE</text><path d="M55 155H545" stroke="#dce1de"/><text x="55" y="205" font-size="14" fill="#71817a">BILLED TO</text><text x="55" y="235" font-size="19">${escapeXML(b.profile.name)}</text><text x="55" y="275" font-size="15">Date: ${date}</text><text x="55" y="305" font-size="15">Reference: DEMO-${vendor
    .toUpperCase()
    .replace(/[^A-Z]/g, "")
    .slice(
      0,
      6,
    )}-01</text><path d="M55 355H545" stroke="#dce1de"/><text x="55" y="395" font-size="16">Business services</text><text x="545" y="395" font-size="16" text-anchor="end">${amount}</text><path d="M55 435H545" stroke="#dce1de"/><text x="55" y="485" font-size="20" font-weight="700">Total</text><text x="545" y="485" font-size="26" font-weight="700" text-anchor="end">${amount}</text><rect x="55" y="540" width="490" height="65" rx="5" fill="#f0f5f1"/><text x="78" y="579" font-size="15" fill="#356454">Receipt evidence · payment recorded separately</text><text x="55" y="675" font-size="13" fill="#87928d">Fictional document for the ${escapeXML(PRODUCT_NAME)} demo.</text></g></svg>`;
  const blob = new Blob([xml], { type: "image/svg+xml" });
  return {
    id: id(),
    businessId: b.profile.id,
    name: `${vendor.toLowerCase().replace(/\s/g, "-")}-receipt.svg`,
    mime: blob.type,
    size: blob.size,
    blob,
    kind: "receipt",
    createdAt: now(),
    demo: true,
  };
}
function business(
  businessId: string,
  name: string,
  industry: string,
  initials: string,
  color: string,
  landscape: boolean,
): BusinessData {
  const currency: Currency = "USD";
  const categories = [
    "Software & subscriptions",
    "Office & supplies",
    "Travel & meals",
    "Professional services",
    landscape ? "Equipment & materials" : "Marketing & design",
  ];
  let b: BusinessData = {
    profile: {
      id: businessId,
      businessId,
      name,
      industry,
      initials,
      color,
      currency,
    },
    accounts: [
      {
        id: `${businessId}-checking`,
        businessId,
        name: "Operating account",
        kind: "bank",
        currency,
      },
      {
        id: `${businessId}-card`,
        businessId,
        name: "Business card",
        kind: "card",
        currency,
      },
      {
        id: `${businessId}-reserve`,
        businessId,
        name: "Reserve account",
        kind: "bank",
        currency,
      },
      {
        id: `${businessId}-eur`,
        businessId,
        name: "Euro account",
        kind: "bank",
        currency: "EUR",
      },
    ],
    categories: [],
    vendors: [],
    documents: [],
    payments: [],
    drafts: [],
    journals: [],
    approvals: [],
    rules: [],
    audits: [],
    events: [],
  };
  categories.forEach((name, i) => {
    const accountId = `${businessId}-expense-${i}`,
      categoryId = `${businessId}-category-${i}`;
    b.accounts.push({
      id: accountId,
      businessId,
      name,
      kind: "expense",
      currency,
    });
    b.categories.push({ id: categoryId, businessId, name, accountId });
  });
  const software = landscape ? "Jobber" : "Figma";
  b.rules.push({
    id: id(),
    businessId,
    vendorIncludes: software,
    categoryId: b.categories[0].id,
    purpose: landscape
      ? "Scheduling and managing field service work"
      : "Design collaboration for client projects",
    enabled: true,
  });
  const period = currentPeriod();
  const add = (
    vendor: string,
    amount: number,
    day: number,
    category: number,
    description: string,
    posted = false,
    hasReceipt = true,
  ) => {
    const date = `${period}-${String(day).padStart(2, "0")}`;
    const doc = hasReceipt
      ? receipt(b, vendor, `$${(amount / 100).toFixed(2)}`, date)
      : undefined;
    if (doc) b.documents.push(doc);
    const draft: ExpenseDraft = {
      ...blankDraft(b),
      vendor,
      amountMinor: amount,
      date,
      categoryId: b.categories[category].id,
      description,
      purpose: description,
      documentIds: doc ? [doc.id] : [],
      accountId: b.accounts[1].id,
    };
    b = saveDraft(b, draft);
    if (posted) b = approve(b, draft.id);
    return draft;
  };
  add(
    software,
    landscape ? 14900 : 9600,
    22,
    0,
    landscape
      ? "Monthly field service subscription"
      : "Professional plan · monthly subscription",
  );
  const incomplete = add(
    landscape ? "Greenway Supply" : "West Elm",
    24850,
    21,
    1,
    "Purchase awaiting details",
    false,
    false,
  );
  b.drafts.find((d) => d.id === incomplete.id)!.categoryId = "";
  b.drafts.find((d) => d.id === incomplete.id)!.purpose = "";
  const mapping = {
    date: "date",
    description: "description",
    amount: "amount",
    currency: "currency",
    reference: "reference",
  };
  const vendor = landscape ? "Oak Valley Nursery" : "Notion";
  b = commitImport(
    b,
    previewImport(
      b,
      [
        ["date", "description", "amount", "currency", "reference"],
        [`${period}-20`, vendor, "-180.00", "USD", `${businessId}-PAY-020`],
      ],
      mapping,
      b.accounts[0].id,
      "negative",
    ),
    b.accounts[0].id,
  );
  const matched = b.drafts.at(-1)!;
  matched.categoryId = b.categories[landscape ? 4 : 0].id;
  matched.purpose = landscape
    ? "Plants for a client landscape project"
    : "Team documentation and project workspace";
  const matchDoc = receipt(b, vendor, "$180.00", matched.date);
  b.documents.push(matchDoc);
  matched.documentIds.push(matchDoc.id);
  add(
    landscape ? "Trail Fuel" : "Linear",
    3200,
    18,
    landscape ? 4 : 0,
    landscape ? "Fuel for field equipment" : "Issue tracking subscription",
    true,
  );
  add(
    landscape ? "Trail Fuel" : "Linear",
    3200,
    18,
    landscape ? 4 : 0,
    "Possible repeat charge · review before posting",
  );
  add(
    landscape ? "Cedar Equipment" : "Studio Supply Co.",
    landscape ? 145000 : 87500,
    16,
    landscape ? 4 : 1,
    landscape
      ? "Commercial mower maintenance"
      : "Workspace equipment and supplies",
    true,
  );
  add(
    landscape ? "Clearwater Insurance" : "Meridian Legal",
    240000,
    12,
    3,
    landscape ? "Business liability insurance" : "Client contract review",
    true,
  );
  const corrected = add(
    landscape ? "Field Crew Co." : "Atlas Creative",
    125000,
    8,
    4,
    "Project services · initial record",
    true,
  );
  const correctedReceipt = receipt(
    b,
    corrected.vendor,
    "$1150.00",
    corrected.date,
  );
  b.documents.push(correctedReceipt);
  b = correct(
    b,
    corrected.id,
    {
      ...corrected,
      id: id(),
      amountMinor: 115000,
      documentIds: [correctedReceipt.id],
      description: "Project services · corrected invoice amount",
      status: "draft",
    },
    "Supplier issued a corrected invoice: reduced by $100.00.",
  );
  add(
    landscape ? "Sprout Software" : "Adobe",
    8999,
    5,
    0,
    "Monthly creative tools subscription",
    true,
  );
  add(
    landscape ? "Route Coffee" : "Common Ground",
    6800,
    3,
    2,
    "Client project planning meeting",
    true,
  );
  const transfer = {
    id: id(),
    businessId,
    date: `${period}-10`,
    currency,
    kind: "transfer" as const,
    memo: "Operating account → Reserve account",
    createdAt: now(),
    lines: [
      { accountId: b.accounts[2].id, debit: 200000, credit: 0 },
      { accountId: b.accounts[0].id, debit: 0, credit: 200000 },
    ],
  };
  assertBalanced(b, transfer);
  b.journals.push(transfer);
  return b;
}
export function seedWorkspace(): Workspace {
  return {
    schemaVersion: 1,
    revision: 0,
    businesses: [
      business(
        "northstar",
        "Northstar Studio",
        "Design & creative services",
        "NS",
        "#c8da78",
        false,
      ),
      business(
        "fieldwork",
        "Fieldwork Landscapes",
        "Landscape & garden care",
        "FL",
        "#b5c9eb",
        true,
      ),
    ],
  };
}
