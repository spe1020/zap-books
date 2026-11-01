//! Invariants that must hold for any input, checked with proptest.

mod common;

use std::collections::BTreeMap;

use common::{fresh, minor, seed};
use ledger_core::*;
use proptest::prelude::*;

/// A reviewable draft for Northstar with the given details.
#[derive(Debug, Clone)]
struct Spec {
    amount: i64,
    currency: Currency,
    day: u32,
    category: usize,
    vendor: String,
}

fn spec(max_amount: i64) -> impl Strategy<Value = Spec> {
    (
        1..=max_amount,
        prop_oneof![
            Just(Currency::Usd),
            Just(Currency::Eur),
            Just(Currency::Gbp)
        ],
        1u32..=28,
        0usize..5,
        "[A-Za-z][A-Za-z ]{0,11}",
    )
        .prop_map(|(amount, currency, day, category, vendor)| Spec {
            amount,
            currency,
            day,
            category,
            vendor,
        })
}

/// Northstar plus a GBP bank account, so every currency can be posted.
fn business() -> (BusinessData, SequentialEnv) {
    let (b, env) = fresh();
    (with_gbp(b), env)
}

fn with_gbp(mut b: BusinessData) -> BusinessData {
    b.accounts.push(Account {
        id: "northstar-gbp".into(),
        business_id: b.profile.id.clone(),
        name: "Sterling account".into(),
        kind: AccountKind::Bank,
        currency: Currency::Gbp,
    });
    b
}

fn draft(b: &BusinessData, env: &mut SequentialEnv, s: &Spec) -> ExpenseDraft {
    let mut d = blank_draft(b, env);
    d.vendor = s.vendor.clone();
    d.description = "Generated expense".into();
    d.purpose = "Generated purpose".into();
    d.date = format!("2026-10-{:02}", s.day);
    d.amount_minor = minor(s.amount);
    d.currency = s.currency;
    d.category_id = b.categories[s.category].id.clone();
    d.account_id = b
        .accounts
        .iter()
        .find(|a| a.kind != AccountKind::Expense && a.currency == s.currency)
        .unwrap()
        .id
        .clone();
    // Random vendors can collide with seeded drafts; a reason keeps the
    // duplicate check from deciding the outcome.
    d.duplicate_reason = "Separate generated expense".into();
    d
}

fn post(b: &BusinessData, env: &mut SequentialEnv, s: &Spec) -> (BusinessData, ExpenseDraft) {
    let d = draft(b, env, s);
    let saved = save_draft(b, &d, env).unwrap();
    (approve(&saved, &d.id, env).unwrap(), d)
}

fn journal_for<'a>(b: &'a BusinessData, expense_id: &str) -> &'a JournalEntry {
    b.journals
        .iter()
        .find(|j| j.expense_id.as_deref() == Some(expense_id) && j.kind == JournalKind::Expense)
        .unwrap()
}

/// Net debit minus credit per account across `entries`, in i128 so the check
/// itself cannot overflow.
fn net_by_account<'a>(
    entries: impl IntoIterator<Item = &'a JournalEntry>,
) -> BTreeMap<String, i128> {
    let mut net = BTreeMap::new();
    for j in entries {
        for l in &j.lines {
            *net.entry(l.account_id.clone()).or_insert(0) +=
                i128::from(l.debit.get()) - i128::from(l.credit.get());
        }
    }
    net
}

/// Every record that carries a business id.
fn business_ids(b: &BusinessData) -> Vec<&str> {
    let mut ids = vec![b.profile.business_id.as_str()];
    ids.extend(b.accounts.iter().map(|x| x.business_id.as_str()));
    ids.extend(b.categories.iter().map(|x| x.business_id.as_str()));
    ids.extend(b.vendors.iter().map(|x| x.business_id.as_str()));
    ids.extend(b.documents.iter().map(|x| x.business_id.as_str()));
    ids.extend(b.payments.iter().map(|x| x.business_id.as_str()));
    ids.extend(b.drafts.iter().map(|x| x.business_id.as_str()));
    ids.extend(
        b.drafts
            .iter()
            .filter_map(|x| x.import_source.as_ref())
            .map(|s| s.business_id.as_str()),
    );
    ids.extend(b.journals.iter().map(|x| x.business_id.as_str()));
    ids.extend(b.approvals.iter().map(|x| x.business_id.as_str()));
    ids.extend(b.rules.iter().map(|x| x.business_id.as_str()));
    ids.extend(b.audits.iter().map(|x| x.business_id.as_str()));
    ids.extend(b.events.iter().map(|x| x.business_id.as_str()));
    ids
}

/// A small payments CSV: unique or blank references, negative outflows.
fn csv_rows() -> impl Strategy<Value = Vec<(u32, String, i64, bool)>> {
    prop::collection::vec(
        (
            1u32..=28,
            "[A-Za-z][A-Za-z ,]{0,10}",
            1i64..=10_000_000,
            any::<bool>(),
        ),
        1..12,
    )
}

fn csv_text(rows: &[(u32, String, i64, bool)]) -> String {
    let mut text = String::from("date,description,amount,currency,reference\n");
    for (i, (day, description, amount, referenced)) in rows.iter().enumerate() {
        let reference = if *referenced {
            format!("REF-{i}")
        } else {
            String::new()
        };
        let amount = amount_input(minor(-amount), Currency::Usd);
        text.push_str(&format!(
            "2026-10-{day:02},\"{description}\",{amount},USD,{reference}\n"
        ));
    }
    text
}

fn payment_map() -> CsvMapping {
    CsvMapping {
        date: "date".into(),
        description: "description".into(),
        amount: "amount".into(),
        currency: "currency".into(),
        reference: "reference".into(),
        ..CsvMapping::default()
    }
}

#[derive(Debug, Clone)]
enum Op {
    Save(Spec, bool),
    Approve(usize),
    Correct(usize, i64),
    Import(Vec<(u32, String, i64, bool)>),
    Foreign(usize),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (spec(1_000_000), any::<bool>()).prop_map(|(s, foreign)| Op::Save(s, foreign)),
        (0usize..40).prop_map(Op::Approve),
        (0usize..40, 1i64..=1_000_000).prop_map(|(i, a)| Op::Correct(i, a)),
        csv_rows().prop_map(Op::Import),
        (0usize..10).prop_map(Op::Foreign),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    // Any successfully approved expense produces a journal whose debits
    // equal credits.
    #[test]
    fn approved_expenses_produce_balanced_journals(s in spec(MAX_MINOR)) {
        let (b, mut env) = business();
        let (posted, d) = post(&b, &mut env, &s);
        let j = journal_for(&posted, &d.id);
        let debit: i128 = j.lines.iter().map(|l| i128::from(l.debit.get())).sum();
        let credit: i128 = j.lines.iter().map(|l| i128::from(l.credit.get())).sum();
        prop_assert_eq!(debit, credit);
        prop_assert_eq!(debit, i128::from(s.amount));
        prop_assert_eq!(j.currency, s.currency);
        prop_assert!(assert_balanced(&posted, j).is_ok());
    }

    // After `correct`, the original journal plus its reversal net to zero on
    // every account.
    #[test]
    fn corrections_net_to_zero_on_every_account(s in spec(MAX_MINOR), amount in 1..=MAX_MINOR) {
        let (b, mut env) = business();
        let (posted, d) = post(&b, &mut env, &s);
        let original = posted.drafts.iter().find(|x| x.id == d.id).unwrap().clone();
        let mut replacement = original.replacement(env.new_id());
        replacement.amount_minor = minor(amount);
        replacement.journal_id = None;
        let next = correct(&posted, &d.id, &replacement, "Generated correction", &mut env).unwrap();
        let prior = journal_for(&next, &d.id);
        let reversal = next
            .journals
            .iter()
            .find(|j| j.reverses_id.as_ref() == Some(&prior.id))
            .unwrap();
        prop_assert!(net_by_account([prior, reversal]).values().all(|n| *n == 0));
        let status = next.drafts.iter().find(|x| x.id == d.id).unwrap().status();
        prop_assert_eq!(status, ExpenseStatus::Corrected);
    }

    // `totals` never returns a figure that combines journals of different
    // currencies.
    #[test]
    fn totals_never_combine_currencies(specs in prop::collection::vec(spec(1_000_000_000), 1..8)) {
        let (mut b, mut env) = business();
        for s in &specs {
            b = post(&b, &mut env, s).0;
        }
        let got = totals(&b, "2026-10").unwrap();
        let mut want: BTreeMap<Currency, i128> = BTreeMap::new();
        for j in b.journals.iter().filter(|j| j.date.starts_with("2026-10")) {
            for l in j.lines.iter().filter(|l| l.category_id.is_some()) {
                *want.entry(j.currency).or_insert(0) +=
                    i128::from(l.debit.get()) - i128::from(l.credit.get());
            }
        }
        let got: BTreeMap<Currency, i128> =
            got.into_iter().map(|(c, m)| (c, i128::from(m.get()))).collect();
        prop_assert_eq!(got, want);
    }

    // amountInput(parseMoney(s)) == s for every valid canonical input.
    #[test]
    fn amount_input_round_trips_canonical_amounts(n in -MAX_MINOR..=MAX_MINOR) {
        let s = amount_input(minor(n), Currency::Usd);
        prop_assert_eq!(parse_money(&s, Currency::Usd).unwrap(), minor(n));
        prop_assert_eq!(amount_input(parse_money(&s, Currency::Usd).unwrap(), Currency::Usd), s);
    }

    #[test]
    fn canonical_text_round_trips(
        negative in any::<bool>(),
        whole in "0|[1-9][0-9]{0,13}",
        cents in "[0-9]{2}",
    ) {
        let s = format!("{}{whole}.{cents}", if negative { "-" } else { "" });
        let value = format!("{whole}{cents}").parse::<i128>().unwrap();
        prop_assume!(value <= i128::from(MAX_MINOR) && !(negative && value == 0));
        prop_assert_eq!(amount_input(parse_money(&s, Currency::Usd).unwrap(), Currency::Usd), s);
    }

    // parseMoney rejects anything outside MAX_MINOR.
    #[test]
    fn parse_money_rejects_values_beyond_max_minor(
        excess in 1u128..=u128::from(u64::MAX),
        negative in any::<bool>(),
    ) {
        let value = u128::from(MAX_MINOR.unsigned_abs()) + excess;
        let s = format!("{}{}.{:02}", if negative { "-" } else { "" }, value / 100, value % 100);
        prop_assert_eq!(parse_money(&s, Currency::Usd), Err(LedgerError::AmountTooLarge));
    }

    // Importing the same CSV twice creates zero new drafts the second time.
    #[test]
    fn importing_the_same_csv_twice_creates_no_new_drafts(rows in csv_rows(), history in any::<bool>()) {
        let (b, mut env) = fresh();
        let account = b.accounts[0].id.clone();
        let text = csv_text(&rows);
        let parsed = parse_csv(&text).unwrap();
        let options = ImportOptions {
            kind: history.then_some(ImportKind::ExpenseHistory),
            ..ImportOptions::default()
        };
        let preview = |b: &BusinessData, env: &mut SequentialEnv| {
            preview_import(b, &parsed, &payment_map(), &account, SignConvention::Negative, &options, env)
                .unwrap()
        };
        let first = preview(&b, &mut env);
        prop_assert!(first.iter().all(|r| r.error.is_none()), "{:?}", first);
        let once = commit_import(&b, &first, &account, &mut env).unwrap();
        prop_assert!(once.drafts.len() > b.drafts.len());
        let again = preview(&once, &mut env);
        prop_assert!(again.iter().all(|r| r.duplicate));
        let twice = commit_import(&once, &again, &account, &mut env).unwrap();
        prop_assert_eq!(twice.drafts.len(), once.drafts.len());
        prop_assert_eq!(twice.payments.len(), once.payments.len());
        let replayed = commit_import(&once, &first, &account, &mut env).unwrap();
        prop_assert_eq!(replayed.drafts.len(), once.drafts.len());
    }

    // No command ever produces a record whose business id differs from the
    // business it was called on.
    #[test]
    fn commands_never_produce_records_for_another_business(ops in prop::collection::vec(op(), 1..12)) {
        let (businesses, mut env) = seed();
        let (mut b, other) = (with_gbp(businesses[0].clone()), businesses[1].clone());
        let own = b.profile.id.clone();
        for op in ops {
            let result = match op {
                Op::Save(s, foreign) => {
                    let mut d = draft(&b, &mut env, &s);
                    if foreign {
                        d.business_id = other.profile.id.clone();
                    }
                    save_draft(&b, &d, &mut env)
                }
                Op::Approve(i) => {
                    let id = b.drafts.get(i % b.drafts.len()).unwrap().id.clone();
                    approve(&b, &id, &mut env)
                }
                Op::Correct(i, amount) => {
                    let original = b.drafts.get(i % b.drafts.len()).unwrap().clone();
                    let mut replacement = original.replacement(env.new_id());
                    replacement.amount_minor = minor(amount);
                    correct(&b, &original.id, &replacement, "Generated", &mut env)
                }
                Op::Import(rows) => {
                    let parsed = parse_csv(&csv_text(&rows)).unwrap();
                    let account = b.accounts[0].id.clone();
                    preview_import(
                        &b, &parsed, &payment_map(), &account, SignConvention::Negative,
                        &ImportOptions::default(), &mut env,
                    )
                    .and_then(|preview| commit_import(&b, &preview, &account, &mut env))
                }
                Op::Foreign(i) => {
                    let id = other.drafts.get(i % other.drafts.len()).unwrap().id.clone();
                    let foreign = approve(&b, &id, &mut env);
                    prop_assert!(foreign.is_err());
                    foreign
                }
            };
            if let Ok(next) = result {
                b = next;
            }
            prop_assert!(business_ids(&b).iter().all(|id| *id == own));
        }
    }
}
