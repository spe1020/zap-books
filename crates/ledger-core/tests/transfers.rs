//! Owner funding merge gates (`docs/briefs/owner-funding.md`, Required
//! tests): transfers, their corrections, opening balances, expenses paid
//! personally, and balances. The TypeScript has the same gates in
//! `src/domain/transfers.test.ts`.

mod common;

use std::path::Path;

use common::minor;
use ledger_core::*;
use proptest::prelude::*;

/// Northstar's accounts and categories with no entries, plus "Loan from
/// owner" (liability) and "Owner contributions" (equity), as recorded by
/// the fixture generator.
fn setup() -> (BusinessData, SequentialEnv) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/parity/states/northstar-funded.json");
    let b: BusinessData = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let env = SequentialEnv::new(9000, "2026-10-15T12:00:00.000Z", "2026-10-15");
    (b, env)
}

fn account(b: &BusinessData, name: &str) -> String {
    b.accounts
        .iter()
        .find(|a| a.name == name)
        .unwrap()
        .id
        .clone()
}

struct Ids {
    bank: String,
    card: String,
    reserve: String,
    euro: String,
    loan: String,
    equity: String,
}

fn ids(b: &BusinessData) -> Ids {
    Ids {
        bank: account(b, "Operating account"),
        card: account(b, "Business card"),
        reserve: account(b, "Reserve account"),
        euro: account(b, "Euro account"),
        loan: account(b, "Loan from owner"),
        equity: account(b, "Owner contributions"),
    }
}

fn balance(b: &BusinessData, account_id: &str, as_of: &str) -> i64 {
    balances(b, as_of, Currency::Usd)
        .unwrap()
        .into_iter()
        .find(|r| r.account_id == account_id)
        .unwrap()
        .balance
        .get()
}

fn transfer(t: TransferType, from: &str, to: &str) -> TransferInput {
    TransferInput {
        date: "2026-03-10".into(),
        amount_minor: minor(50_000),
        currency: Currency::Usd,
        from_account_id: from.into(),
        to_account_id: to.into(),
        transfer_type: t,
        purpose: "Invented purpose".into(),
        reference: None,
        document_ids: Vec::new(),
    }
}

fn with(mut t: TransferInput, amount: i64, date: &str) -> TransferInput {
    t.amount_minor = minor(amount);
    t.date = date.into();
    t
}

fn expense(
    b: &BusinessData,
    env: &mut SequentialEnv,
    account_id: &str,
    amount: i64,
) -> BusinessData {
    let mut d = blank_draft(b, env);
    d.vendor = "Invented Supplies".into();
    d.description = "Invented paper".into();
    d.purpose = "Office use".into();
    d.category_id = b.categories[0].id.clone();
    d.account_id = account_id.into();
    d.amount_minor = minor(amount);
    d.date = "2026-03-12".into();
    d.duplicate_reason = "Separate invented expense".into();
    let saved = save_draft(b, &d, env).unwrap();
    approve(&saved, &d.id, env).unwrap()
}

#[test]
fn a_personally_paid_expense_leaves_the_bank_unchanged_and_raises_the_loan() {
    let (b, mut env) = setup();
    let a = ids(&b);
    let paid = expense(&b, &mut env, &a.loan, 1_234);
    assert_eq!(balance(&paid, &a.bank, "2026-12-31"), 0);
    assert_eq!(balance(&paid, &a.loan, "2026-12-31"), 1_234);
    assert_eq!(
        totals(&paid, "2026-03").unwrap()[&Currency::Usd],
        minor(1_234)
    );
    let contributed = expense(&b, &mut env, &a.equity, 999);
    assert_eq!(balance(&contributed, &a.bank, "2026-12-31"), 0);
    assert_eq!(balance(&contributed, &a.equity, "2026-12-31"), 999);
}

#[test]
fn a_loan_deposit_raises_the_bank_and_the_loan_and_a_repayment_lowers_both() {
    let (b, mut env) = setup();
    let a = ids(&b);
    let lent = record_transfer(
        &b,
        &transfer(TransferType::OwnerLoan, &a.loan, &a.bank),
        &mut env,
    )
    .unwrap();
    assert_eq!(balance(&lent, &a.bank, "2026-12-31"), 50_000);
    assert_eq!(balance(&lent, &a.loan, "2026-12-31"), 50_000);
    let repayment = with(
        transfer(TransferType::LoanRepayment, &a.bank, &a.loan),
        20_000,
        "2026-03-11",
    );
    let repaid = record_transfer(&lent, &repayment, &mut env).unwrap();
    assert_eq!(balance(&repaid, &a.bank, "2026-12-31"), 30_000);
    assert_eq!(balance(&repaid, &a.loan, "2026-12-31"), 30_000);
}

#[test]
fn transfers_never_count_as_spending() {
    let (mut b, mut env) = setup();
    let a = ids(&b);
    for t in [
        transfer(TransferType::OwnerLoan, &a.loan, &a.bank),
        transfer(TransferType::OwnerContribution, &a.equity, &a.card),
        transfer(TransferType::LoanRepayment, &a.bank, &a.loan),
        transfer(TransferType::OwnerDraw, &a.card, &a.equity),
        transfer(TransferType::BetweenAccounts, &a.bank, &a.reserve),
    ] {
        b = record_transfer(&b, &t, &mut env).unwrap();
    }
    let opening = OpeningBalanceInput {
        account_id: a.bank.clone(),
        date: "2026-01-01".into(),
        amount_minor: minor(70_000),
        reference: None,
    };
    b = record_opening_balance(&b, &opening, &mut env).unwrap();
    assert!(totals(&b, "2026").unwrap().is_empty());
    assert!(category_totals(&b, "2026", Currency::Usd)
        .unwrap()
        .is_empty());
}

#[test]
fn every_wrong_account_for_each_transfer_type_is_refused_with_its_message() {
    let (b, mut env) = setup();
    let a = ids(&b);
    let kinds = [
        ("bank", &a.bank),
        ("card", &a.card),
        ("loan", &a.loan),
        ("equity", &a.equity),
    ];
    let rules: [(TransferType, &[&str], &[&str], &str); 5] = [
        (
            TransferType::OwnerLoan,
            &["loan"],
            &["bank", "card"],
            "An owner loan moves money from a loan account into a bank or card account.",
        ),
        (
            TransferType::OwnerContribution,
            &["equity"],
            &["bank", "card"],
            "An owner contribution moves money from an owner equity account into a bank or card account.",
        ),
        (
            TransferType::LoanRepayment,
            &["bank", "card"],
            &["loan"],
            "A loan repayment moves money from a bank or card account into a loan account.",
        ),
        (
            TransferType::OwnerDraw,
            &["bank", "card"],
            &["equity"],
            "An owner draw moves money from a bank or card account into an owner equity account.",
        ),
        (
            TransferType::BetweenAccounts,
            &["bank", "card"],
            &["bank", "card"],
            "A transfer between accounts moves money from one bank or card account to another.",
        ),
    ];
    let mut refused = 0;
    for (t, froms, tos, message) in rules {
        for (from_kind, from) in kinds {
            for (to_kind, to) in kinds {
                if from == to {
                    continue;
                }
                let result = record_transfer(&b, &transfer(t, from, to), &mut env);
                if froms.contains(&from_kind) && tos.contains(&to_kind) {
                    assert!(result.is_ok(), "{t:?} {from_kind} to {to_kind}");
                } else {
                    assert_eq!(
                        result.unwrap_err().to_string(),
                        message,
                        "{from_kind} to {to_kind}"
                    );
                    refused += 1;
                }
            }
        }
    }
    assert!(refused > 30);
}

#[test]
fn correcting_a_transfer_reverses_it_exactly_and_links_the_replacement() {
    let (b, mut env) = setup();
    let a = ids(&b);
    let lent = record_transfer(
        &b,
        &transfer(TransferType::OwnerLoan, &a.loan, &a.bank),
        &mut env,
    )
    .unwrap();
    let original = lent.journals.last().unwrap().to_unchecked();
    let replacement = with(
        transfer(TransferType::OwnerLoan, &a.loan, &a.card),
        45_000,
        "2026-03-10",
    );
    let fixed = correct_transfer(
        &lent,
        &original.id,
        &replacement,
        "Deposited to the card",
        &mut env,
    )
    .unwrap();
    let reversal = fixed
        .journals
        .iter()
        .find(|j| j.reverses_id.as_deref() == Some(original.id.as_str()))
        .unwrap();
    let posted = fixed
        .journals
        .iter()
        .find(|j| j.correction_of.as_deref() == Some(original.id.as_str()))
        .unwrap();
    assert_eq!(reversal.kind, JournalKind::Reversal);
    for (r, o) in reversal.lines.iter().zip(&original.lines) {
        assert_eq!(
            (r.account_id.as_str(), r.debit, r.credit),
            (o.account_id.as_str(), o.credit, o.debit)
        );
    }
    assert_eq!(posted.kind, JournalKind::Transfer);
    // The original and its reversal net to zero on both of its accounts.
    assert_eq!(balance(&fixed, &a.bank, "2026-12-31"), 0);
    // Money paid into a card leaves it in credit: less than nothing owed.
    assert_eq!(balance(&fixed, &a.card, "2026-12-31"), -45_000);
    assert_eq!(balance(&fixed, &a.loan, "2026-12-31"), 45_000);
    let last = fixed.events.last().unwrap();
    assert_eq!(last.event_type, EventType::TransferReversed);
    assert_eq!(
        last.source_ids,
        vec![original.id.clone(), reversal.id.clone(), posted.id.clone()]
    );
    let again = correct_transfer(
        &fixed,
        &original.id,
        &transfer(TransferType::OwnerLoan, &a.loan, &a.bank),
        "Again",
        &mut env,
    );
    assert_eq!(again.unwrap_err(), LedgerError::TransferAlreadyCorrected);
    // An invalid replacement changes nothing: the caller keeps `lent`.
    let wrong = correct_transfer(
        &lent,
        &original.id,
        &transfer(TransferType::OwnerLoan, &a.bank, &a.loan),
        "Wrong way round",
        &mut env,
    );
    assert!(wrong.is_err());
    assert_eq!(lent.journals.len(), 1);
}

#[test]
fn a_transfer_between_currencies_or_to_the_same_account_is_refused() {
    let (b, mut env) = setup();
    let a = ids(&b);
    let across = record_transfer(
        &b,
        &transfer(TransferType::BetweenAccounts, &a.bank, &a.euro),
        &mut env,
    );
    assert_eq!(across.unwrap_err(), LedgerError::TransferCurrency);
    let mut in_euros = transfer(TransferType::BetweenAccounts, &a.reserve, &a.bank);
    in_euros.currency = Currency::Eur;
    assert_eq!(
        record_transfer(&b, &in_euros, &mut env).unwrap_err(),
        LedgerError::TransferCurrency
    );
    let itself = record_transfer(
        &b,
        &transfer(TransferType::BetweenAccounts, &a.bank, &a.bank),
        &mut env,
    );
    assert_eq!(itself.unwrap_err(), LedgerError::TransferSameAccount);
}

#[test]
fn an_opening_balance_dated_after_the_first_entry_is_refused() {
    let (b, mut env) = setup();
    let a = ids(&b);
    let started = expense(&b, &mut env, &a.card, 1_000);
    let late = OpeningBalanceInput {
        account_id: a.bank.clone(),
        date: "2026-03-13".into(),
        amount_minor: minor(11_700),
        reference: None,
    };
    assert_eq!(
        record_opening_balance(&started, &late, &mut env)
            .unwrap_err()
            .to_string(),
        "An opening balance must be dated on or before the first entry, 2026-03-12."
    );
    let on_time = OpeningBalanceInput {
        date: "2026-03-12".into(),
        ..late
    };
    let opened = record_opening_balance(&started, &on_time, &mut env).unwrap();
    assert_eq!(balance(&opened, &a.bank, "2026-12-31"), 11_700);
    assert_eq!(
        record_opening_balance(&opened, &on_time, &mut env).unwrap_err(),
        LedgerError::OpeningBalanceExists
    );
}

#[test]
fn balances_are_right_before_between_and_after_entries_with_reversals() {
    let (mut b, mut env) = setup();
    let a = ids(&b);
    let opening = OpeningBalanceInput {
        account_id: a.bank.clone(),
        date: "2026-01-01".into(),
        amount_minor: minor(100_000),
        reference: None,
    };
    b = record_opening_balance(&b, &opening, &mut env).unwrap();
    let loan = with(
        transfer(TransferType::OwnerLoan, &a.loan, &a.bank),
        50_000,
        "2026-02-01",
    );
    b = record_transfer(&b, &loan, &mut env).unwrap();
    let loan_id = b.journals.last().unwrap().id.clone();
    b = expense(&b, &mut env, &a.card, 2_000);
    let smaller = with(
        transfer(TransferType::OwnerLoan, &a.loan, &a.bank),
        30_000,
        "2026-02-01",
    );
    b = correct_transfer(&b, &loan_id, &smaller, "Smaller loan", &mut env).unwrap();
    assert_eq!(balance(&b, &a.bank, "2025-12-31"), 0);
    assert_eq!(balance(&b, &a.bank, "2026-01-01"), 100_000);
    assert_eq!(balance(&b, &a.loan, "2026-01-31"), 0);
    assert_eq!(balance(&b, &a.bank, "2026-02-01"), 130_000);
    assert_eq!(balance(&b, &a.loan, "2026-02-01"), 30_000);
    assert_eq!(balance(&b, &a.card, "2026-03-11"), 0);
    assert_eq!(balance(&b, &a.card, "2026-03-12"), 2_000);
    assert_eq!(balance(&b, &a.bank, "2027-01-01"), 130_000);
    let euros: Vec<String> = balances(&b, "2026-12-31", Currency::Eur)
        .unwrap()
        .into_iter()
        .map(|r| r.name)
        .collect();
    assert_eq!(euros, vec!["Euro account".to_string()]);
    assert_eq!(
        balances(&b, "2026-02-30", Currency::Usd).unwrap_err(),
        LedgerError::InvalidBalanceDate
    );
}

#[test]
fn a_cards_balance_is_whats_owed_purchases_raise_it_and_payments_lower_it() {
    let (b, mut env) = setup();
    let a = ids(&b);
    let b = expense(&b, &mut env, &a.card, 2_500);
    assert_eq!(balance(&b, &a.card, "2026-12-31"), 2_500);
    let b = expense(&b, &mut env, &a.card, 1_500);
    assert_eq!(balance(&b, &a.card, "2026-12-31"), 4_000);
    let payment = with(
        transfer(TransferType::BetweenAccounts, &a.bank, &a.card),
        3_000,
        "2026-03-20",
    );
    let b = record_transfer(&b, &payment, &mut env).unwrap();
    assert_eq!(balance(&b, &a.card, "2026-12-31"), 1_000);
    assert_eq!(balance(&b, &a.bank, "2026-12-31"), -3_000);
}

#[test]
fn a_cards_opening_balance_is_entered_as_the_amount_owed() {
    let (b, mut env) = setup();
    let a = ids(&b);
    let input = OpeningBalanceInput {
        account_id: a.card.clone(),
        date: "2026-01-01".into(),
        amount_minor: minor(8_000),
        reference: None,
    };
    let owed = record_opening_balance(&b, &input, &mut env).unwrap();
    assert_eq!(balance(&owed, &a.card, "2026-12-31"), 8_000);
    // The card is credited, as a purchase would credit it.
    let line = owed
        .journals
        .last()
        .unwrap()
        .lines
        .iter()
        .find(|l| l.account_id == a.card)
        .unwrap()
        .clone();
    assert_eq!((line.debit, line.credit), (Minor::ZERO, minor(8_000)));
    let bought = expense(&owed, &mut env, &a.card, 500);
    assert_eq!(balance(&bought, &a.card, "2026-12-31"), 8_500);
}

/// A bank account with an opening balance, its journal id and the opening
/// balances account.
fn opened() -> (BusinessData, SequentialEnv, String, String) {
    let (b, mut env) = setup();
    let input = OpeningBalanceInput {
        account_id: ids(&b).bank,
        date: "2026-01-01".into(),
        amount_minor: minor(10_000),
        reference: None,
    };
    let b = record_opening_balance(&b, &input, &mut env).unwrap();
    let journal = b.journals.last().unwrap().id.clone();
    let opening = account(&b, "Opening balances");
    (b, env, journal, opening)
}

fn opening_replacement(from: &str, to: &str, amount: i64, date: &str) -> TransferInput {
    let mut t = with(
        transfer(TransferType::OpeningBalance, from, to),
        amount,
        date,
    );
    t.purpose = "Opening balance".into();
    t
}

#[test]
fn an_opening_balance_correction_may_change_the_amount_its_direction_and_the_date() {
    let (b, mut env, journal, opening) = opened();
    let bank = ids(&b).bank;
    let smaller = opening_replacement(&opening, &bank, 9_000, "2025-12-31");
    let fixed = correct_transfer(&b, &journal, &smaller, "Statement misread", &mut env).unwrap();
    assert_eq!(balance(&fixed, &bank, "2026-12-31"), 9_000);
    let overdrawn = opening_replacement(&bank, &opening, 700, "2026-01-01");
    let fixed = correct_transfer(&b, &journal, &overdrawn, "Overdrawn", &mut env).unwrap();
    assert_eq!(balance(&fixed, &bank, "2026-12-31"), -700);
}

#[test]
fn an_opening_balance_correction_refuses_another_type() {
    let (b, mut env, journal, opening) = opened();
    let mut replacement = opening_replacement(&opening, &ids(&b).bank, 9_000, "2026-01-01");
    replacement.transfer_type = TransferType::BetweenAccounts;
    assert_eq!(
        correct_transfer(&b, &journal, &replacement, "Invented reason", &mut env).unwrap_err(),
        LedgerError::TransferCorrectionKind
    );
}

#[test]
fn an_opening_balance_correction_refuses_another_account() {
    let (b, mut env, journal, opening) = opened();
    let a = ids(&b);
    for (from, to) in [
        (&opening, &a.reserve),
        (&a.card, &opening),
        (&a.loan, &opening),
        // Nor against another equity account.
        (&a.equity, &a.bank),
    ] {
        let replacement = opening_replacement(from, to, 9_000, "2026-01-01");
        assert_eq!(
            correct_transfer(&b, &journal, &replacement, "Invented reason", &mut env).unwrap_err(),
            LedgerError::OpeningBalanceAccountChanged
        );
    }
}

#[test]
fn an_opening_balance_correction_refuses_a_date_after_the_first_entry() {
    let (b, mut env, journal, opening) = opened();
    let a = ids(&b);
    let started = expense(&b, &mut env, &a.card, 1_000); // dated 2026-03-12
    let late = opening_replacement(&opening, &a.bank, 12_300, "2026-03-13");
    assert_eq!(
        correct_transfer(&started, &journal, &late, "Invented reason", &mut env)
            .unwrap_err()
            .to_string(),
        "An opening balance must be dated on or before the first entry, 2026-03-12."
    );
    let on_time = opening_replacement(&opening, &a.bank, 12_300, "2026-03-12");
    assert!(correct_transfer(&started, &journal, &on_time, "Invented reason", &mut env).is_ok());
}

#[derive(Debug, Clone)]
enum Op {
    Transfer {
        t: usize,
        from: usize,
        to: usize,
        amount: i64,
        month: u32,
    },
    Expense {
        account: usize,
        amount: i64,
    },
    Correct {
        pick: usize,
        amount: i64,
    },
    Opening {
        account: usize,
        amount: i64,
    },
}

fn op() -> impl Strategy<Value = Op> {
    let amount = 1i64..=10_000_000;
    prop_oneof![
        (0usize..5, 0usize..5, 0usize..5, amount.clone(), 1u32..=9).prop_map(
            |(t, from, to, amount, month)| Op::Transfer {
                t,
                from,
                to,
                amount,
                month
            }
        ),
        (0usize..5, amount.clone()).prop_map(|(account, amount)| Op::Expense { account, amount }),
        (any::<usize>(), amount.clone()).prop_map(|(pick, amount)| Op::Correct { pick, amount }),
        (0usize..4, -10_000_000i64..=10_000_000)
            .prop_map(|(account, amount)| Op::Opening { account, amount }),
    ]
}

const TYPES: [TransferType; 5] = [
    TransferType::OwnerLoan,
    TransferType::OwnerContribution,
    TransferType::LoanRepayment,
    TransferType::OwnerDraw,
    TransferType::BetweenAccounts,
];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Across any sequence of commands (each valid one applied, each
    /// invalid one refused), total debits equal total credits, and the
    /// balances of all non-expense accounts plus spending reconcile with
    /// the journal.
    #[test]
    fn debits_equal_credits_and_balances_plus_spending_reconcile(ops in prop::collection::vec(op(), 1..30)) {
        let (mut b, mut env) = setup();
        let a = ids(&b);
        let usd = [a.bank.clone(), a.card.clone(), a.reserve.clone(), a.loan.clone(), a.equity.clone()];
        for op in ops {
            let next = match op {
                Op::Transfer { t, from, to, amount, month } => {
                    let input = with(transfer(TYPES[t], &usd[from], &usd[to]), amount, &format!("2026-0{month}-15"));
                    record_transfer(&b, &input, &mut env)
                }
                Op::Expense { account, amount } => Ok(expense(&b, &mut env, &usd[account], amount)),
                Op::Correct { pick, amount } => {
                    let live: Vec<UncheckedJournal> = b
                        .journals
                        .iter()
                        .filter(|j| j.kind == JournalKind::Transfer)
                        .filter(|j| !b.journals.iter().any(|r| r.reverses_id.as_deref() == Some(j.id.as_str())))
                        .map(|j| j.to_unchecked())
                        .collect();
                    if live.is_empty() {
                        continue;
                    }
                    let original = &live[pick % live.len()];
                    let mut replacement = with(
                        transfer(
                            original.transfer_type.unwrap_or(TransferType::BetweenAccounts),
                            &original.lines[1].account_id,
                            &original.lines[0].account_id,
                        ),
                        amount,
                        &original.date,
                    );
                    if replacement.transfer_type == TransferType::OpeningBalance {
                        replacement.purpose = "Opening balance".into();
                    }
                    correct_transfer(&b, &original.id, &replacement, "Generated correction", &mut env)
                }
                Op::Opening { account, amount } => {
                    let input = OpeningBalanceInput {
                        account_id: usd[[0, 1, 2, 3][account]].clone(),
                        date: "2026-01-01".into(),
                        amount_minor: minor(amount),
                        reference: None,
                    };
                    record_opening_balance(&b, &input, &mut env)
                }
            };
            if let Ok(next) = next {
                b = next;
            }
        }
        let (mut debit, mut credit) = (0i64, 0i64);
        for j in &b.journals {
            for l in &j.lines {
                debit += l.debit.get();
                credit += l.credit.get();
            }
        }
        prop_assert_eq!(debit, credit);
        let net: i64 = balances(&b, "2026-12-31", Currency::Usd)
            .unwrap()
            .iter()
            .map(|r| match r.kind {
                AccountKind::Bank => r.balance.get(),
                _ => -r.balance.get(),
            })
            .sum();
        let spending = totals(&b, "2026").unwrap().get(&Currency::Usd).map_or(0, |m| m.get());
        prop_assert_eq!(net + spending, 0);
    }
}
