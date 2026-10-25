/**
 * Records the behavior of src/domain as JSON fixtures for the Rust port in
 * crates/ledger-core. Every fixture runs a real TypeScript function with
 * sequential ids and a frozen clock, then stores the call, its input, and its
 * output or error message. crates/ledger-core/tests/parity.rs replays each
 * one and requires identical JSON.
 *
 * Run with `npm run parity:fixtures`. CI regenerates the fixtures and fails if
 * they differ from the committed copies.
 */
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { test, vi } from "vitest";
import type {
  BusinessData,
  ExpenseDraft,
  JournalEntry,
  SourceDocument,
} from "../src/domain/types";
import * as L from "../src/domain/ledger";
import * as I from "../src/domain/import";
import * as X from "../src/domain/export";
import { seedWorkspace } from "../src/domain/seed";

const NOW = "2026-10-15T12:00:00.000Z";
const OUT = new URL("../fixtures/parity/", import.meta.url);
const MAX = Number.MAX_SAFE_INTEGER;
let nextId = 1;

type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
interface Fixture {
  name: string;
  call: string;
  env: { nextId: number; now: string };
  input: Record<string, Json>;
  output?: Json;
  error?: string;
}

const areas = new Map<string, Fixture[]>();
const stateNames = new Map<string, string>();
const stateFiles = new Map<string, Json>();
const json = <T>(value: T): T => JSON.parse(JSON.stringify(value));

/** Stores `value` once under `states/` so fixtures can refer to it by name. */
function state<T>(name: string, value: T): T {
  const copy = json(value);
  stateNames.set(JSON.stringify(copy), name);
  stateFiles.set(name, copy as Json);
  return copy;
}

/** A large string kept in the fixture as a recipe instead of literally. */
type Part = string | { $repeat: [string, number] };
const concat = (...parts: Part[]) => ({ $concat: parts });
function expand(value: Json): Json {
  if (Array.isArray(value)) return value.map(expand);
  if (value && typeof value === "object") {
    if ("$concat" in value)
      return (value.$concat as Part[])
        .map((p) =>
          typeof p === "string" ? p : p.$repeat[0].repeat(p.$repeat[1]),
        )
        .join("");
    return Object.fromEntries(
      Object.entries(value).map(([k, v]) => [k, expand(v)]),
    );
  }
  return value;
}

/**
 * Runs one TypeScript call on a JSON copy of `input` and records the result.
 * Returns the output as JSON so scenarios can chain calls.
 */
function record(
  area: string,
  name: string,
  call: string,
  input: Record<string, unknown>,
  run: (input: any) => unknown,
): any {
  const fixtures = areas.get(area) ?? [];
  areas.set(area, fixtures);
  if (fixtures.some((f) => f.name === name))
    throw new Error(`Duplicate fixture name: ${area}/${name}`);
  const stored = json(input) as Record<string, Json>;
  const fixture: Fixture = {
    name,
    call,
    env: { nextId, now: NOW },
    input: stored,
  };
  try {
    const result = run(expand(json(stored)));
    fixture.output = result === undefined ? null : json(result);
  } catch (e) {
    // A TypeError is a crash, not specified behavior; never record one.
    if (!(e instanceof Error) || e.constructor !== Error) throw e;
    fixture.error = e.message;
  }
  fixtures.push(fixture);
  return fixture.output;
}

function write() {
  rmSync(OUT, { recursive: true, force: true });
  mkdirSync(new URL("states/", OUT), { recursive: true });
  const pretty = (value: unknown) => JSON.stringify(value, null, 1) + "\n";
  for (const [name, value] of stateFiles)
    writeFileSync(new URL(`states/${name}.json`, OUT), pretty(value));
  for (const [area, fixtures] of areas) {
    const withRefs = fixtures.map((f) => ({
      ...f,
      input: Object.fromEntries(
        Object.entries(f.input).map(([k, v]) => {
          const ref = stateNames.get(JSON.stringify(v));
          return [k, ref ? { $state: ref } : v];
        }),
      ),
    }));
    writeFileSync(new URL(`${area}.json`, OUT), pretty(withRefs));
  }
}

const find = (b: BusinessData, vendor: string, status = "draft") => {
  const d = b.drafts.find((x) => x.vendor === vendor && x.status === status);
  if (!d) throw new Error(`No ${status} draft for ${vendor}`);
  return json(d);
};
const ready = (b: BusinessData, extra: Partial<ExpenseDraft>) => ({
  ...L.blankDraft(b),
  vendor: "Harbor Print",
  description: "Client brochures",
  purpose: "Client deliverable",
  categoryId: b.categories[1].id,
  amountMinor: 4200,
  ...extra,
});
const paymentMap = {
  date: "date",
  description: "description",
  amount: "amount",
  currency: "currency",
  reference: "reference",
};
const historyCSV =
  'Expense Date,Vendor Name,Description,Expense Amount,Currency Code,Expense Category,Reference Number,Business Purpose\n10/23/2026,Paper & Pine,Workshop materials,"1,234.56",USD,Office & supplies,QB-901,Client workshop';
const quickbooks = {
  kind: "expense-history",
  provider: "quickbooks",
  dateFormat: "mdy",
  filename: "expenses.csv",
} as const;

function money() {
  const inputs = [
    "0.29",
    "100.01",
    "-4.05",
    "1.001",
    "NaN",
    "1e4",
    "9007199254740992",
    "",
    "42",
    "42.5",
    "42.50",
    " 42.50 ",
    "\u00a0\ufeff7.25\u3000",
    "\u00857.25",
    "-0",
    "-0.00",
    "0.00",
    "00012.30",
    "1,234.56",
    "+1",
    ".5",
    "5.",
    "--1",
    "1.2.3",
    "１２",
    "90071992547409.91",
    "90071992547409.92",
    "-90071992547409.91",
    "-90071992547409.92",
    "90071992547409",
    "90071992547410",
    "9007199254740991",
  ];
  for (const input of inputs)
    record(
      "money",
      `parseMoney ${JSON.stringify(input)}`,
      "parseMoney",
      { input },
      (i) => L.parseMoney(i.input),
    );
  record(
    "money",
    "parseMoney 400 digits",
    "parseMoney",
    { input: "1".repeat(400) },
    (i) => L.parseMoney(i.input),
  );
  record(
    "money",
    "parseMoney 400 leading zeros",
    "parseMoney",
    { input: "0".repeat(400) + "1.25" },
    (i) => L.parseMoney(i.input),
  );
  for (const n of [0, 1, -1, 5, 99, 100, 10001, -405, 123456, MAX, -MAX])
    record("money", `amountInput ${n}`, "amountInput", { n }, (i) =>
      L.amountInput(i.n),
    );
}

function dates() {
  for (const date of [
    "2026-10-15",
    "2026-02-30",
    "2026-02-29",
    "2024-02-29",
    "2000-02-29",
    "1900-02-29",
    "2026-04-31",
    "2026-12-31",
    "2026-13-01",
    "2026-00-10",
    "2026-01-00",
    "2026-01-32",
    "2026-1-01",
    "2026-01-01T00:00:00Z",
    " 2026-01-01",
    "2026/01/01",
    "abcd-ef-gh",
    "",
    "0000-01-01",
    "0000-02-29",
    "9999-12-31",
    "２０２６-01-01",
  ])
    record(
      "date",
      `validDate ${JSON.stringify(date)}`,
      "validDate",
      { date },
      (i) => L.validDate(i.date),
    );
  for (const [value, format] of [
    ["10/23/2026", "mdy"],
    ["23/10/2026", "dmy"],
    ["1/2/2026", "mdy"],
    ["1/2/2026", "dmy"],
    ["2026-10-23", "iso"],
    ["anything", "iso"],
    ["2026-10-23", "mdy"],
    ["10/23/26", "mdy"],
    ["10-23-2026", "dmy"],
    ["123/1/2026", "mdy"],
    [" 1/2/2026", "mdy"],
    ["1/2/2026/1", "mdy"],
  ])
    record(
      "date",
      `normalizeImportDate ${value} ${format}`,
      "normalizeImportDate",
      { value, format },
      (i) => I.normalizeImportDate(i.value, i.format),
    );
}

function validation(ns: BusinessData, fw: BusinessData) {
  const linear = find(ns, "Linear");
  const notion = find(ns, "Notion");
  const cases: [string, ExpenseDraft][] = [
    ["blank draft", L.blankDraft(ns)],
    ["incomplete seeded draft", find(ns, "West Elm")],
    ["complete draft", find(ns, "Figma")],
    ["possible duplicate", linear],
    [
      "possible duplicate with reason",
      { ...linear, duplicateReason: "Second seat" },
    ],
    [
      "duplicate vendor differs only by case and spacing",
      { ...linear, vendor: "  LINEAR  " },
    ],
    [
      "different vendor is not a duplicate",
      { ...linear, vendor: "Different vendor" },
    ],
    ["blank vendor never duplicates", { ...linear, vendor: "   " }],
    ["posted draft", find(ns, "Adobe", "posted")],
    ["foreign draft", fw.drafts[0]],
    [
      "foreign payment account",
      { ...find(ns, "Figma"), accountId: fw.accounts[0].id },
    ],
    [
      "expense account as payment account",
      { ...find(ns, "Figma"), accountId: ns.accounts[4].id },
    ],
    ["currency mismatch", { ...find(ns, "Figma"), currency: "EUR" }],
    [
      "foreign receipt",
      { ...find(ns, "Figma"), documentIds: [fw.documents[0].id] },
    ],
    [
      "payment amount mismatch",
      { ...notion, amountMinor: notion.amountMinor + 1 },
    ],
    ["payment date mismatch", { ...notion, date: "2026-10-19" }],
    ["unknown payment", { ...notion, paymentId: "missing" }],
    ["empty payment id is ignored", { ...find(ns, "Figma"), paymentId: "" }],
    [
      "invalid date and zero amount",
      { ...find(ns, "Figma"), date: "2026-02-30", amountMinor: 0 },
    ],
    ["negative amount", { ...find(ns, "Figma"), amountMinor: -5 }],
    [
      "whitespace-only text fields",
      { ...find(ns, "Figma"), description: " \t", purpose: "\u3000" },
    ],
    [
      "foreign category",
      { ...find(ns, "Figma"), categoryId: fw.categories[0].id },
    ],
  ];
  // W5 (PARITY_NOTES.md): drafts from another business are not duplicates.
  record(
    "validation",
    "duplicates: drafts from another business",
    "duplicates",
    {
      business: { ...ns, drafts: [...ns.drafts, ...fw.drafts] },
      draft: { ...find(ns, "Linear"), vendor: "Trail Fuel" },
    },
    (i) => L.duplicates(i.business, i.draft),
  );
  for (const [name, draft] of cases)
    for (const call of ["validation", "missingDetails", "duplicates"] as const)
      record(
        "validation",
        `${call}: ${name}`,
        call,
        { business: ns, draft },
        (i) => L[call](i.business, i.draft),
      );

  const balanced = (name: string, b: BusinessData, entry: unknown) =>
    record(
      "validation",
      `assertBalanced: ${name}`,
      "assertBalanced",
      { business: b, entry },
      (i) => L.assertBalanced(i.business, i.entry),
    );
  ns.journals.forEach((j, n) =>
    balanced(`seeded journal ${n} (${j.kind})`, ns, j),
  );
  const j: JournalEntry = json(ns.journals[0]);
  balanced("foreign business journal", ns, fw.journals[0]);
  balanced("unbalanced", ns, {
    ...j,
    lines: [{ ...j.lines[0], debit: j.lines[0].debit + 1 }, j.lines[1]],
  });
  balanced("single line", ns, { ...j, lines: [j.lines[0]] });
  balanced("invalid date", ns, { ...j, date: "2026-02-30" });
  balanced("unknown account", ns, {
    ...j,
    lines: [j.lines[0], { ...j.lines[1], accountId: "missing" }],
  });
  balanced("negative amount", ns, {
    ...j,
    lines: [
      { ...j.lines[0], debit: -5, credit: 0 },
      { ...j.lines[1], debit: 0, credit: -5 },
    ],
  });
  balanced("two-sided line", ns, {
    ...j,
    lines: [
      { ...j.lines[0], credit: 1 },
      { ...j.lines[1], debit: 1 },
    ],
  });
  balanced("category on wrong account", ns, {
    ...j,
    lines: [{ ...j.lines[0], categoryId: ns.categories[3].id }, j.lines[1]],
  });
  balanced("zero amounts", ns, {
    ...j,
    lines: j.lines.map((l) => ({ ...l, debit: 0, credit: 0 })),
  });
  balanced("EUR journal on USD bank", ns, { ...j, currency: "EUR" });
  balanced("EUR journal on EUR bank and expense account", ns, {
    ...j,
    currency: "EUR",
    lines: [j.lines[0], { ...j.lines[1], accountId: ns.accounts[3].id }],
  });
  balanced("total exceeds safe range", ns, {
    ...j,
    lines: [
      { ...j.lines[0], debit: MAX },
      { ...j.lines[0], debit: MAX },
      { ...j.lines[1], credit: MAX },
      { ...j.lines[1], credit: MAX },
    ],
  });
}

function drafts(ns: BusinessData, fw: BusinessData) {
  record(
    "drafts",
    "blankDraft: northstar",
    "blankDraft",
    { business: ns },
    (i) => L.blankDraft(i.business),
  );
  record(
    "drafts",
    "blankDraft: fieldwork",
    "blankDraft",
    { business: fw },
    (i) => L.blankDraft(i.business),
  );
  record(
    "drafts",
    "blankDraft: no bank account in business currency",
    "blankDraft",
    { business: { ...ns, profile: { ...ns.profile, currency: "GBP" } } },
    (i) => L.blankDraft(i.business),
  );
  const save = (name: string, b: BusinessData, draft: unknown) =>
    record(
      "drafts",
      `saveDraft: ${name}`,
      "saveDraft",
      { business: b, draft },
      (i) => L.saveDraft(i.business, i.draft),
    );
  save("new draft", ns, ready(ns, {}));
  save("new draft with blank vendor", ns, ready(ns, { vendor: "  " }));
  save(
    "new draft with existing vendor in other case",
    ns,
    ready(ns, { vendor: "  figma " }),
  );
  save("update existing draft", ns, {
    ...find(ns, "Figma"),
    vendor: "Figma Inc",
    amountMinor: 9700,
  });
  save("posted draft cannot be edited", ns, find(ns, "Adobe", "posted"));
  save("posted draft relabelled as draft", ns, {
    ...find(ns, "Adobe", "posted"),
    status: "draft",
  });
  save("foreign draft", ns, fw.drafts[0]);
  save("foreign receipt", ns, {
    ...find(ns, "Figma"),
    documentIds: [fw.documents[0].id],
  });
  save("foreign import source", ns, {
    ...ready(ns, {}),
    importSource: {
      id: "src",
      businessId: fw.profile.id,
      provider: "csv",
      reference: "",
      fingerprint: "fp",
      date: "2026-10-01",
      amountMinor: 4200,
      currency: "USD",
      vendor: "",
      category: "",
      filename: "",
    },
  });
  // Pinned (PARITY_NOTES.md): evidence may be attached after the fact.
  save("import source added to existing draft", ns, {
    ...find(ns, "Figma"),
    importSource: {
      id: "src",
      businessId: ns.profile.id,
      provider: "csv",
      reference: "LATE-1",
      fingerprint: "fp-late",
      date: "2026-10-22",
      amountMinor: 9600,
      currency: "USD",
      vendor: "Figma",
      category: "",
      filename: "late.csv",
    },
  });
  record(
    "drafts",
    "event: settings updated",
    "event",
    {
      business: ns,
      type: "settings.updated",
      entityId: ns.profile.id,
      detail: "Changed",
      sourceIds: ["a", "b"],
    },
    (i) => {
      L.event(i.business, i.type, i.entityId, i.detail, i.sourceIds);
      return i.business;
    },
  );
}

function approvals(ns: BusinessData, fw: BusinessData) {
  const approve = (name: string, b: BusinessData, expenseId: string) =>
    record("approve", name, "approve", { business: b, expenseId }, (i) =>
      L.approve(i.business, i.expenseId),
    );
  approve("success", ns, find(ns, "Figma").id);
  approve("payment-linked success", ns, find(ns, "Notion").id);
  approve("already posted", ns, find(ns, "Adobe", "posted").id);
  approve("not found", ns, "missing");
  approve("foreign-business draft", ns, fw.drafts[0].id);
  approve("incomplete draft", ns, find(ns, "West Elm").id);
  const linear = find(ns, "Linear");
  approve("duplicate without reason", ns, linear.id);
  approve(
    "duplicate with reason",
    {
      ...ns,
      drafts: ns.drafts.map((d) =>
        d.id === linear.id ? { ...d, duplicateReason: "Second seat" } : d,
      ),
    },
    linear.id,
  );
  approve(
    "duplicate posting prevented by existing approval",
    {
      ...ns,
      approvals: [
        ...ns.approvals,
        { ...ns.approvals[0], id: "extra", expenseId: find(ns, "Figma").id },
      ],
    },
    find(ns, "Figma").id,
  );
  const eur = ready(ns, {
    currency: "EUR",
    accountId: ns.accounts[3].id,
    vendor: "Euro vendor",
  });
  const withEur = L.saveDraft(ns, eur);
  approve("EUR expense on EUR account", withEur, eur.id);
  const mismatch = ready(ns, { currency: "EUR", vendor: "Euro vendor" });
  approve("currency mismatch", L.saveDraft(ns, mismatch), mismatch.id);
  const notion = find(ns, "Notion");
  approve(
    "payment mismatch",
    {
      ...ns,
      drafts: ns.drafts.map((d) =>
        d.id === notion.id ? { ...d, amountMinor: d.amountMinor + 1 } : d,
      ),
    },
    notion.id,
  );
  const posted = L.approve(ns, notion.id);
  const second = {
    ...notion,
    id: L.id(),
    vendor: "Notion Labs",
    documentIds: [],
  };
  approve("payment already linked", L.saveDraft(posted, second), second.id);
  approve(
    "category found by id in another business first",
    {
      ...ns,
      categories: [
        { ...fw.categories[0], id: ns.categories[0].id },
        ...ns.categories,
      ],
    },
    find(ns, "Figma").id,
  );
}

function corrections(ns: BusinessData) {
  const correct = (
    name: string,
    b: BusinessData,
    originalId: string,
    replacement: unknown,
    reason: string,
  ) =>
    record(
      "correct",
      name,
      "correct",
      { business: b, originalId, replacement, reason },
      (i) => L.correct(i.business, i.originalId, i.replacement, i.reason),
    );
  const adobe = find(ns, "Adobe", "posted");
  const replacement = {
    ...adobe,
    id: L.id(),
    status: "draft",
    amountMinor: 7000,
    journalId: undefined,
  };
  const next = correct(
    "success",
    ns,
    adobe.id,
    replacement,
    "Updated invoice amount",
  );
  correct("missing reason", ns, adobe.id, replacement, "   ");
  correct(
    "original is a draft",
    ns,
    find(ns, "Figma").id,
    replacement,
    "Reason",
  );
  correct(
    "replacement reuses original id",
    ns,
    adobe.id,
    { ...replacement, id: adobe.id },
    "Reason",
  );
  correct(
    "replacement id already exists",
    ns,
    adobe.id,
    { ...replacement, id: find(ns, "Figma").id },
    "Reason",
  );
  correct(
    "replacement already posted",
    ns,
    adobe.id,
    { ...replacement, status: "posted" },
    "Reason",
  );
  correct(
    "replacement in another business",
    ns,
    adobe.id,
    { ...replacement, businessId: "fieldwork" },
    "Reason",
  );
  correct(
    "invalid replacement amount",
    ns,
    adobe.id,
    { ...replacement, amountMinor: 0 },
    "Invalid",
  );
  correct(
    "replacement keeps the old journal id",
    ns,
    adobe.id,
    { ...adobe, id: L.id(), status: "draft" },
    "Same",
  );
  correct(
    "correcting the original a second time",
    next,
    adobe.id,
    { ...replacement, id: L.id() },
    "Again",
  );
  correct(
    "correcting a replacement a second time",
    next,
    replacement.id,
    { ...replacement, id: L.id(), amountMinor: 6500 },
    "Second revision",
  );
  const notion = find(ns, "Notion");
  const postedNotion = L.approve(ns, notion.id);
  const posted = find(postedNotion, "Notion", "posted");
  correct(
    "changed payment link",
    postedNotion,
    notion.id,
    { ...posted, id: L.id(), status: "draft", paymentId: undefined },
    "Unlink",
  );
  correct(
    "payment-linked amount change",
    postedNotion,
    notion.id,
    {
      ...posted,
      id: L.id(),
      status: "draft",
      amountMinor: posted.amountMinor + 1,
    },
    "Changed receipt total",
  );
  correct(
    "payment-linked description change",
    postedNotion,
    notion.id,
    { ...posted, id: L.id(), status: "draft", description: "Team workspace" },
    "Clarify description",
  );
}

function totals(ns: BusinessData, fw: BusinessData) {
  const eur = ready(ns, {
    currency: "EUR",
    accountId: ns.accounts[3].id,
    vendor: "Euro vendor",
    amountMinor: 10001,
  });
  const withEur = state(
    "northstar-with-eur",
    L.approve(L.saveDraft(ns, eur), eur.id),
  );
  for (const [label, b] of [
    ["northstar", ns],
    ["fieldwork", fw],
    ["northstar with EUR", withEur],
  ] as const)
    for (const period of [
      "2026-10",
      "2026",
      "2026-09",
      "2026-10-08",
      "1900-01",
      "",
    ]) {
      record(
        "totals",
        `totals: ${label} ${JSON.stringify(period)}`,
        "totals",
        { business: b, period },
        (i) => L.totals(i.business, i.period),
      );
      for (const currency of ["USD", "EUR", "GBP"])
        record(
          "totals",
          `categoryTotals: ${label} ${JSON.stringify(period)} ${currency}`,
          "categoryTotals",
          { business: b, period, currency },
          (i) => L.categoryTotals(i.business, i.period, i.currency),
        );
    }
  record(
    "totals",
    "categoryTotals: unknown category name and zero net",
    "categoryTotals",
    {
      business: { ...ns, categories: ns.categories.slice(1) },
      journalContexts: ["northstar"],
      period: "2026-10",
      currency: "USD",
    },
    (i) => L.categoryTotals(i.business, i.period, i.currency),
  );
  // W4 (PARITY_NOTES.md): queries skip journals from another business.
  const mixed = { ...ns, journals: [...ns.journals, ...fw.journals] };
  record(
    "totals",
    "totals: mixed businesses",
    "totals",
    {
      business: mixed,
      journalContexts: ["northstar", "fieldwork"],
      period: "2026-10",
    },
    (i) => L.totals(i.business, i.period),
  );
  record(
    "totals",
    "categoryTotals: mixed businesses",
    "categoryTotals",
    {
      business: mixed,
      journalContexts: ["northstar", "fieldwork"],
      period: "2026-10",
      currency: "USD",
    },
    (i) => L.categoryTotals(i.business, i.period, i.currency),
  );
  return withEur;
}

function csv() {
  const parse = (name: string, text: unknown) =>
    record("csv", `parseCSV: ${name}`, "parseCSV", { text }, (i) =>
      I.parseCSV(i.text),
    );
  parse(
    "quoted fields with commas, CRLF",
    'date,description,amount,currency,reference\r\n2026-10-20,"Tools, Inc.",-42.50,USD,R1\r\n2026-10-21,"Tools, Inc.",-42.50,USD,R2\r\n',
  );
  parse("embedded newline", 'a,b\n"line one\nline two",x\n');
  parse("embedded CRLF and comma", 'a,b\r\n"one,\r\ntwo",x');
  parse("doubled quotes", 'a,b\n"say ""hi""",""""');
  parse("CR line endings", "a,b\r1,2\r3,4");
  parse("byte order mark", "\ufeffa,b\n1,2");
  parse("blank and empty-cell rows skipped", "a,b\n\n , \n1,2\n,\n");
  parse("cells trimmed, quoted whitespace too", 'a,b\n  x  ," y "');
  parse("whitespace after closing quote", 'a,b\n"x"  ,z');
  parse("ragged rows", "a,b,c\n1\n1,2,3,4");
  parse("unterminated quote", 'a,b\n"unterminated');
  parse("quote inside unquoted cell", 'a,b\nx"y",z');
  parse("space before quote", 'a,b\n "x",z');
  parse("space after comma before quote", 'a,b\nx, "y"');
  parse("JS whitespace before quote", 'a,b\n\t\u00a0\ufeff"x, y",z');
  parse("text and space before quote", 'a,b\nx,y "z"');
  parse("whitespace between two quoted values", 'a,b\n"x" "y",z');
  parse("text after closing quote", 'a,b\n"x"y,z');
  parse("header only", "a,b\n");
  parse("empty", "");
  parse("duplicate headers", "a,a\n1,2");
  parse("empty header", "a,,c\n1,2,3");
  parse("2,000 transactions", concat("amount\n", { $repeat: ["1\n", 2000] }));
  parse("2,001 transactions", concat("amount\n", { $repeat: ["1\n", 2001] }));
  parse("oversized file", concat("a,b\n", { $repeat: ["x", 2_000_000] }));
  // 1,000,006 code points but 2,000,006 UTF-16 code units.
  parse(
    "oversized by UTF-16 length",
    concat("a,b\n1,", { $repeat: ["😀", 1_000_000] }),
  );
  // 3 MB of UTF-8 but 1,500,008 UTF-16 code units, trimmed to a blank row.
  parse(
    "within limit by UTF-16 length",
    concat("a,b\n1,2\n", { $repeat: ["\u00a0", 1_500_000] }),
  );
  parse("emoji cells", "a,b\n😀,ok");
  for (const headers of [
    ["date", "description", "amount", "currency", "reference"],
    [
      "Expense Date",
      "Vendor Name",
      "Description",
      "Expense Amount",
      "Currency Code",
      "Expense Category",
      "Reference Number",
      "Business Purpose",
    ],
    ["Date", "Name", "Memo/Description", "Amount", "Transaction Type", "Num"],
    ["Transaction Date", "Merchant", "Total", "Paid Through", "Expense ID"],
    [
      "Date",
      "Vendor",
      "Description",
      "Amount",
      "Currency",
      "Payment account",
      "Reference",
    ],
    [
      "DATE",
      "Date",
      "Payee",
      "Total Amount",
      "Category",
      "Type",
      "Bank Account",
      "ID",
    ],
    ["x", "y"],
  ])
    record(
      "csv",
      `guessMapping: ${headers.join("|")}`,
      "guessMapping",
      { headers },
      (i) => I.guessMapping(i.headers),
    );
}

function payments(ns: BusinessData, fw: BusinessData) {
  const account = ns.accounts[0].id;
  const preview = (
    name: string,
    b: BusinessData,
    rows: unknown,
    extra: Record<string, unknown> = {},
  ) =>
    record(
      "import",
      `previewImport: ${name}`,
      "previewImport",
      {
        business: b,
        rows,
        map: paymentMap,
        accountId: account,
        convention: "negative",
        options: {},
        ...extra,
      },
      (i) =>
        I.previewImport(
          i.business,
          i.rows,
          i.map,
          i.accountId,
          i.convention,
          i.options,
        ),
    );
  const commit = (
    name: string,
    b: BusinessData,
    rows: unknown,
    accountId = account,
  ) =>
    record(
      "import",
      `commitImport: ${name}`,
      "commitImport",
      { business: b, rows, accountId },
      (i) => I.commitImport(i.business, i.rows, i.accountId),
    );
  const header = ["date", "description", "amount", "currency", "reference"];
  const tools = I.parseCSV(
    'date,description,amount,currency,reference\r\n2026-10-20,"Tools, Inc.",-42.50,USD,R1\r\n2026-10-21,"Tools, Inc.",-42.50,USD,R2\r\n',
  );
  const rows = preview("equal amounts with distinct references", ns, tools);
  const next = commit("two payments", ns, rows);
  commit("re-import creates nothing", next, rows);
  preview("re-import is all duplicates", next, tools);
  const one = (
    name: string,
    row: string[],
    extra: Record<string, unknown> = {},
  ) => preview(name, ns, [header, row], extra);
  one("impossible date", ["2026-02-30", "A", "-1", "USD", "X"]);
  one("income row", ["2026-10-01", "A", "10", "USD", "Y"]);
  one("currency mismatch", ["2026-10-01", "A", "-10", "EUR", "Z"]);
  one("lower-case currency", ["2026-10-01", "A", "-10", "usd", "Z"]);
  one("zero amount", ["2026-10-01", "A", "0.00", "USD", "Z"]);
  one("negative zero", ["2026-10-01", "A", "-0", "USD", "Z"]);
  one("thousands groups", ["2026-10-01", "A", "-1,234.56", "USD", "Z"]);
  one("many thousands groups", [
    "2026-10-01",
    "A",
    "-12,345,678.90",
    "USD",
    "Z",
  ]);
  one("malformed thousands groups", [
    "2026-10-01",
    "A",
    "-1,23.00",
    "USD",
    "Z",
  ]);
  one("four-digit leading group", [
    "2026-10-01",
    "A",
    "-1234,567.00",
    "USD",
    "Z",
  ]);
  one("three decimals", ["2026-10-01", "A", "-1.234", "USD", "Z"]);
  one("missing description", ["2026-10-01", "", "-1", "USD", "Z"]);
  one("column count mismatch", ["2026-10-01", "A", "-1", "USD", "Z", "extra"]);
  one("short row", ["2026-10-01", "A"]);
  one("no reference uses details", [
    "2026-10-01",
    "  Tools \t Inc  ",
    "-1",
    "USD",
    "",
  ]);
  one("positive convention outflow", ["2026-10-01", "A", "42.50", "USD", "P"], {
    convention: "positive",
  });
  one("positive convention refund", ["2026-10-01", "A", "-42.50", "USD", "P"], {
    convention: "positive",
  });
  one("unmapped amount", ["2026-10-01", "A", "-1", "USD", "Z"], {
    map: { ...paymentMap, amount: "" },
  });
  one(
    "currency not mapped uses account currency",
    ["2026-10-01", "A", "-1", "EUR", "Z"],
    {
      map: { ...paymentMap, currency: "" },
    },
  );
  one("expense account selected", ["2026-10-01", "A", "-1", "USD", "Z"], {
    accountId: ns.accounts[4].id,
  });
  one(
    "other business account selected",
    ["2026-10-01", "A", "-1", "USD", "Z"],
    { accountId: fw.accounts[0].id },
  );
  one("unknown account selected", ["2026-10-01", "A", "-1", "USD", "Z"], {
    accountId: "missing",
  });
  for (const [format, date, ok] of [
    ["mdy", "10/20/2026", true],
    ["dmy", "20/10/2026", true],
    ["iso", "2026-10-20", true],
    ["mdy", "20/10/2026", false],
    ["dmy", "10/20/2026", false],
    ["iso", "10/20/2026", false],
    ["mdy", "2/29/2028", true],
  ] as const)
    one(
      `date format ${format} ${date}${ok ? "" : " rejected"}`,
      [date, "A", "-1", "USD", "D"],
      {
        options: { dateFormat: format },
      },
    );
  const typed = [
    "date",
    "description",
    "amount",
    "currency",
    "reference",
    "type",
  ];
  preview(
    "transaction types",
    ns,
    [
      typed,
      ["2026-10-01", "A", "-1", "USD", "T1", "Expense"],
      ["2026-10-01", "B", "-1", "USD", "T2", "Credit Card Charge"],
      ["2026-10-01", "C", "-1", "USD", "T3", "Deposit"],
      ["2026-10-01", "D", "-1", "USD", "T4", ""],
    ],
    { map: { ...paymentMap, transactionType: "type" } },
  );
  preview(
    "multiple source accounts",
    ns,
    [
      [...header, "account"],
      ["2026-10-01", "A", "-1", "USD", "S1", "Checking"],
      ["2026-10-02", "B", "-1", "USD", "S2", " checking "],
      ["2026-10-03", "C", "-1", "USD", "S3", "Card"],
    ],
    { map: { ...paymentMap, sourceAccount: "account" } },
  );
  preview(
    "one source account in different case",
    ns,
    [
      [...header, "account"],
      ["2026-10-01", "A", "-1", "USD", "S1", "Checking"],
      ["2026-10-02", "B", "-1", "USD", "S2", " CHECKING "],
    ],
    { map: { ...paymentMap, sourceAccount: "account" } },
  );
  preview("combined amount exceeds the safe range", ns, [
    header,
    ["2026-10-01", "A", "-50000000000000.00", "USD", "C1"],
    ["2026-10-02", "B", "-50000000000000.00", "USD", "C2"],
    ["2026-10-03", "C", "-1.00", "USD", "C3"],
  ]);
  const conflictCSV = I.parseCSV(
    "date,description,amount,currency,reference\n2026-10-20,Tools,-42.50,USD,REF-1\n2026-10-21,Tools,-55.00,USD,REF-1",
  );
  const inFile = preview("conflicting references in one file", ns, conflictCSV);
  commit("conflict blocks the whole import", ns, inFile);
  preview("repeated reference in one file", ns, [
    header,
    ["2026-10-01", "A", "-10", "USD", "SAME"],
    ["2026-10-01", "A", "-10", "USD", "SAME"],
  ]);
  const abc = [header, ["2026-10-01", "Vendor", "-10", "USD", "ABC"]];
  const abcRows = I.previewImport(ns, abc, paymentMap, account, "negative");
  const withAbc = commit("single payment", ns, abcRows);
  const changed = [header, ["2026-10-01", "Vendor", "-20", "USD", "ABC"]];
  preview("conflicting reference against existing payment", withAbc, changed);
  commit(
    "conflict with existing payment at commit time",
    I.commitImport(
      ns,
      I.previewImport(ns, changed, paymentMap, account, "negative"),
      account,
    ),
    abcRows,
  );
  commit("expense account rejected", ns, abcRows, ns.accounts[4].id);
  commit("other business account rejected", ns, abcRows, fw.accounts[0].id);
  commit("crafted invalid row", ns, [{ ...abcRows[0], date: "2026-02-30" }]);
  commit("crafted currency mismatch", ns, [{ ...abcRows[0], currency: "EUR" }]);
  commit("empty error string is not an error", ns, [
    { ...abcRows[0], error: "" },
  ]);
  // Audit regression: previews cannot be moved across businesses/accounts.
  commit(
    "payment rows previewed for another business",
    ns,
    I.previewImport(fw, abc, paymentMap, fw.accounts[0].id, "negative"),
  );
}

function history(ns: BusinessData, fw: BusinessData) {
  const account = ns.accounts[0].id;
  const preview = (
    name: string,
    b: BusinessData,
    rows: string[][],
    options: unknown,
    extra: Record<string, unknown> = {},
  ) =>
    record(
      "history",
      `previewImport: ${name}`,
      "previewImport",
      {
        business: b,
        rows,
        map: I.guessMapping(rows[0]),
        accountId: account,
        convention: "positive",
        options,
        ...extra,
      },
      (i) =>
        I.previewImport(
          i.business,
          i.rows,
          i.map,
          i.accountId,
          i.convention,
          i.options,
        ),
    );
  const commit = (
    name: string,
    b: BusinessData,
    rows: unknown,
    accountId = account,
  ) =>
    record(
      "history",
      `commitImport: ${name}`,
      "commitImport",
      { business: b, rows, accountId },
      (i) => I.commitImport(i.business, i.rows, i.accountId),
    );
  const rows = I.parseCSV(historyCSV);
  const migration = preview(
    "QuickBooks export with regional dates",
    ns,
    rows,
    quickbooks,
  );
  const imported = state(
    "northstar-history-imported",
    commit("QuickBooks rows become drafts", ns, migration),
  );
  commit(
    "source rejected in another business",
    fw,
    migration,
    fw.accounts[0].id,
  );
  const d: ExpenseDraft = imported.drafts.at(-1);
  record(
    "history",
    "saveDraft: import source cannot change",
    "saveDraft",
    {
      business: imported,
      draft: {
        ...d,
        importSource: { ...d.importSource, reference: "different" },
      },
    },
    (i) => L.saveDraft(i.business, i.draft),
  );
  record(
    "history",
    "saveDraft: import source removed",
    "saveDraft",
    {
      business: imported,
      draft: { ...d, importSource: undefined },
    },
    (i) => L.saveDraft(i.business, i.draft),
  );
  record(
    "history",
    "saveDraft: draft moved to another business",
    "saveDraft",
    {
      business: fw,
      draft: {
        ...d,
        businessId: fw.profile.id,
        accountId: fw.accounts[0].id,
        categoryId: fw.categories[0].id,
      },
    },
    (i) => L.saveDraft(i.business, i.draft),
  );
  const edited = record(
    "history",
    "saveDraft: edit imported draft",
    "saveDraft",
    {
      business: imported,
      draft: { ...d, vendor: "Edited vendor" },
    },
    (i) => L.saveDraft(i.business, i.draft),
  );
  const posted = record(
    "history",
    "approve: imported draft",
    "approve",
    { business: edited, expenseId: d.id },
    (i) => L.approve(i.business, i.expenseId),
  );
  const original = posted.drafts.find((x: ExpenseDraft) => x.id === d.id);
  const corrected = record(
    "history",
    "correct: imported expense",
    "correct",
    {
      business: posted,
      originalId: d.id,
      replacement: {
        ...original,
        id: L.id(),
        status: "draft",
        amountMinor: 123000,
      },
      reason: "Corrected expense total",
    },
    (i) => L.correct(i.business, i.originalId, i.replacement, i.reason),
  );
  record(
    "history",
    "correct: changed import source",
    "correct",
    {
      business: posted,
      originalId: d.id,
      replacement: {
        ...original,
        id: L.id(),
        status: "draft",
        importSource: { ...original.importSource, filename: "other.csv" },
      },
      reason: "Different file",
    },
    (i) => L.correct(i.business, i.originalId, i.replacement, i.reason),
  );
  preview(
    "re-import after correction is a duplicate",
    corrected,
    rows,
    quickbooks,
  );
  commit("re-import after correction creates nothing", corrected, migration);
  const duplicate = {
    ...d,
    id: L.id(),
    vendor: "Unrelated vendor",
    status: "draft",
  };
  record(
    "history",
    "approve: import source already posted",
    "approve",
    {
      business: L.saveDraft(corrected, duplicate),
      expenseId: duplicate.id,
    },
    (i) => L.approve(i.business, i.expenseId),
  );

  const legacy = I.parseCSV(
    historyCSV.replace("Office & supplies", "Legacy category"),
  );
  const unresolved = commit(
    "unknown category stays unresolved",
    ns,
    preview("unknown category", ns, legacy, {
      kind: "expense-history",
      dateFormat: "mdy",
    }),
  );
  record(
    "history",
    "approve: unresolved category",
    "approve",
    {
      business: unresolved,
      expenseId: unresolved.drafts.at(-1).id,
    },
    (i) => L.approve(i.business, i.expenseId),
  );
  preview(
    "Zoho bill row rejected",
    ns,
    I.parseCSV(
      "Date,Name,Memo/Description,Amount,Transaction Type,Num\n23/10/2026,Vendor,Old bill,12.00,Bill,Z1",
    ),
    { kind: "expense-history", dateFormat: "dmy", provider: "zoho" },
  );
  preview(
    "conflicting repeated reference",
    ns,
    I.parseCSV(
      historyCSV +
        "\n10/23/2026,Paper & Pine,Workshop materials,500.00,USD,Office & supplies,QB-901,Client workshop",
    ),
    { kind: "expense-history", dateFormat: "mdy" },
  );
  const mixed = preview(
    "mixed-account file",
    ns,
    I.parseCSV(
      "Date,Vendor,Description,Amount,Currency,Payment account,Reference\n2026-10-01,A,Materials,12.00,USD,Checking,A1\n2026-10-02,B,Service,15.00,USD,Credit card,B1",
    ),
    { kind: "expense-history" },
  );
  commit("mixed-account file blocked", ns, mixed);
  const sparse = preview(
    "missing vendor, category and purpose columns",
    ns,
    I.parseCSV(
      "Date,Description,Amount\n2026-10-02,Bulk paper,15.00\n2026-10-03,Toner,  \n2026-10-04,,9.99",
    ),
    { kind: "expense-history" },
  );
  preview(
    "blank vendor cell with mapped vendor column",
    ns,
    I.parseCSV(
      "Date,Vendor,Description,Amount,Category\n2026-10-02,,Bulk paper,15.00,",
    ),
    { kind: "expense-history" },
  );
  commit(
    "rows without references",
    ns,
    sparse.filter((r: I.ImportRow) => !r.error),
  );
  const exported = X.exportRecords(ns, "expenses", {
    from: "",
    to: "",
    currency: "",
  });
  const back = I.parseCSV(exported.csv);
  commit(
    "our export imports back as drafts",
    ns,
    preview("our expense export maps back", ns, back, {
      kind: "expense-history",
    }),
  );
}

function exports(ns: BusinessData, fw: BusinessData, withEur: BusinessData) {
  const all = { from: "", to: "", currency: "" };
  const run = (
    name: string,
    b: BusinessData,
    format: string,
    filter: unknown,
    journalContexts?: string[],
  ) =>
    record(
      "export",
      `exportRecords: ${name}`,
      "exportRecords",
      { business: b, format, filter, journalContexts },
      (i) => X.exportRecords(i.business, i.format, i.filter),
    );
  for (const format of ["expenses", "journal"]) {
    run(`${format}, northstar`, ns, format, all);
    run(`${format}, fieldwork`, fw, format, all);
    run(`${format}, EUR only`, withEur, format, { ...all, currency: "EUR" });
    run(`${format}, GBP only`, withEur, format, { ...all, currency: "GBP" });
    run(`${format}, date range`, ns, format, {
      ...all,
      from: "2026-10-05",
      to: "2026-10-16",
    });
    run(`${format}, from only`, ns, format, { ...all, from: "2026-10-12" });
    run(`${format}, to only`, ns, format, { ...all, to: "2026-10-12" });
    run(`${format}, empty range`, ns, format, {
      ...all,
      from: "1900-01-01",
      to: "1900-02-01",
    });
    run(
      `${format}, mixed businesses`,
      {
        ...withEur,
        drafts: [...withEur.drafts, ...fw.drafts],
        journals: [...withEur.journals, ...fw.journals],
      },
      format,
      all,
      ["northstar-with-eur", "fieldwork"],
    );
  }
  run("invalid from date", ns, "expenses", { ...all, from: "2026-02-30" });
  run("invalid to date", ns, "journal", { ...all, to: "10/01/2026" });
  run("reversed range", ns, "expenses", {
    ...all,
    from: "2026-12-31",
    to: "2026-01-01",
  });
  run("same-day range", ns, "expenses", {
    ...all,
    from: "2026-10-12",
    to: "2026-10-12",
  });
  const target = find(ns, "Adobe", "posted").id;
  const hostile = [
    '=HYPERLINK("https://invalid.example", "open")',
    "+1",
    "-2",
    "@SUM(A1)",
    "  =x",
    "\ufeff=x",
    "\tTabbed",
    "\rReturn",
    "\nNewline",
    'Quoted "receipt", with\nnewline',
    "Plain",
  ];
  hostile.forEach((vendor, n) =>
    run(
      `formula-injection escaping ${n}`,
      {
        ...ns,
        drafts: ns.drafts.map((d) =>
          d.id === target
            ? { ...d, vendor, description: vendor, purpose: vendor }
            : d,
        ),
      },
      "expenses",
      all,
    ),
  );
  for (const name of ["Café Ünïcode & Co.", "!!!", "  --Northstar--  "])
    run(
      `filename for ${JSON.stringify(name)}`,
      { ...ns, profile: { ...ns.profile, name } },
      "journal",
      { ...all, currency: "USD" },
    );
  for (const [name, rows] of [
    ["empty", []],
    ["header only", [["a", "b"]]],
    [
      "escaping",
      [
        [
          "=1",
          "+2",
          "-3",
          "@4",
          " =5",
          "\t6",
          "\r7",
          "\n8",
          "\ufeff=9",
          'say "hi"',
          "x,y",
          "",
          "1-2",
        ],
      ],
    ],
  ] as const)
    record("export", `encodeCSV: ${name}`, "encodeCSV", { rows }, (i) =>
      X.encodeCSV(i.rows),
    );
}

function integrity(ns: BusinessData, fw: BusinessData) {
  for (const kind of ["payment", "expense-history"] as const) {
    for (const reference of ["AUDIT-ONE", ""]) {
      const rows = I.parseCSV(
        `date,description,amount,currency,reference\n2026-10-24,Audit Vendor,-42.50,USD,${reference}`,
      );
      const account = ns.accounts[0].id;
      const preview = I.previewImport(
        ns,
        rows,
        paymentMap,
        account,
        "negative",
        { kind },
      );
      const imported = I.commitImport(ns, preview, account);
      const commit = (
        name: string,
        business: BusinessData,
        accountId: string,
      ) =>
        record(
          "integrity",
          `${kind}/${reference || "fallback"}: ${name}`,
          "commitImport",
          { business, rows: preview, accountId },
          (i) => I.commitImport(i.business, i.rows, i.accountId),
        );
      commit("different account", ns, ns.accounts[1].id);
      commit("different business", fw, fw.accounts[0].id);
      commit(
        "already imported row still checks destination",
        imported,
        ns.accounts[1].id,
      );
      commit("same destination re-import", imported, account);
      // In scope, but the fingerprint no longer matches the row.
      const forged = preview.map((r) => ({
        ...r,
        fingerprint: '["northstar","elsewhere"]',
        ...(r.importSource && {
          importSource: {
            ...r.importSource,
            fingerprint: '["northstar","elsewhere"]',
          },
        }),
      }));
      record(
        "integrity",
        `${kind}/${reference || "fallback"}: forged fingerprint in scope`,
        "commitImport",
        { business: ns, rows: forged, accountId: account },
        (i) => I.commitImport(i.business, i.rows, i.accountId),
      );
      const draft = {
        ...imported.drafts.at(-1)!,
        categoryId: ns.categories[0].id,
        purpose: "Client work",
      };
      const posted = L.approve(L.saveDraft(imported, draft), draft.id);
      const original = posted.drafts.find((d) => d.id === draft.id)!;
      const replacement = {
        ...original,
        id: L.id(),
        status: "draft" as const,
        correctionOf: original.id,
      };
      record(
        "integrity",
        `${kind}/${reference || "fallback"}: forged correction`,
        "approve",
        {
          business: L.saveDraft(posted, replacement),
          expenseId: replacement.id,
        },
        (i) => L.approve(i.business, i.expenseId),
      );
      record(
        "integrity",
        `${kind}/${reference || "fallback"}: valid correction`,
        "correct",
        {
          business: posted,
          originalId: original.id,
          replacement,
          reason: "Clarify purchase",
        },
        (i) => L.correct(i.business, i.originalId, i.replacement, i.reason),
      );
    }
  }
}

test("generate parity fixtures", () => {
  vi.spyOn(crypto, "randomUUID").mockImplementation(
    () =>
      `id-${String(nextId++).padStart(4, "0")}` as ReturnType<
        typeof crypto.randomUUID
      >,
  );
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date(NOW));
  try {
    const workspace = seedWorkspace();
    for (const b of workspace.businesses)
      for (const d of b.documents) delete (d as Partial<SourceDocument>).blob;
    stateFiles.set(
      "seed",
      json({ env: { nextId, now: NOW }, workspace }) as Json,
    );
    const ns = state("northstar", workspace.businesses[0]);
    const fw = state("fieldwork", workspace.businesses[1]);
    money();
    dates();
    validation(ns, fw);
    drafts(ns, fw);
    approvals(ns, fw);
    corrections(ns);
    const withEur = totals(ns, fw);
    csv();
    payments(ns, fw);
    history(ns, fw);
    exports(ns, fw, withEur);
    integrity(ns, fw);
    write();
  } finally {
    vi.useRealTimers();
    vi.restoreAllMocks();
  }
});
