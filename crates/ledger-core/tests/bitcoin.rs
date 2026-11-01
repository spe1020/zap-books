//! Bitcoin merge gates (decision 0005, phase 1). The TypeScript has the same
//! gates in `src/domain/bitcoin.test.ts`.

mod common;

use std::path::Path;

use common::minor;
use ledger_core::*;
use proptest::prelude::*;

const BANK: &str = "northstar-checking";

/// Northstar with no entries, plus "Lightning wallet" and "Cold storage",
/// as recorded by the fixture generator.
fn setup() -> (BusinessData, SequentialEnv) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/parity/states/northstar-wallets.json");
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

/// `sats` at 60,000 a bitcoin, in cents.
fn valued(sats: i64) -> FiatValue {
    FiatValue {
        currency: Currency::Usd,
        amount_minor: minor(((sats as i128 * 60_000 * 100 + 50_000_000) / 100_000_000) as i64),
        rate: "60000.00".into(),
        source: FiatSource::Manual,
        at: "2027-01-15".into(),
    }
}

fn balance(b: &BusinessData, currency: Currency, account_id: &str) -> i64 {
    balances(b, "2027-12-31", currency)
        .unwrap()
        .into_iter()
        .find(|r| r.account_id == account_id)
        .unwrap()
        .balance
        .get()
}

fn expense(
    b: &BusinessData,
    env: &mut SequentialEnv,
    currency: Currency,
    amount: i64,
    fiat_value: Option<FiatValue>,
) -> Result<BusinessData> {
    let mut d = blank_draft(b, env);
    d.vendor = "Invented Relay Co".into();
    d.description = "Relay subscription".into();
    d.purpose = "Client deliverable".into();
    d.category_id = b.categories[0].id.clone();
    d.date = "2027-01-15".into();
    d.currency = currency;
    d.amount_minor = minor(amount);
    d.account_id = if currency == Currency::Btc {
        account(b, "Lightning wallet")
    } else {
        b.accounts
            .iter()
            .find(|a| a.kind == AccountKind::Bank && a.currency == currency)
            .unwrap()
            .id
            .clone()
    };
    d.fiat_value = fiat_value;
    let saved = save_draft(b, &d, env)?;
    approve(&saved, &d.id, env)
}

/// `impliedRate`, for the tests: the rate to 12 places, rounded half up.
fn implied_rate(sats: i64, cents: i64) -> String {
    let scaled = (cents as u128 * 10u128.pow(18) * 2 + sats as u128) / (2 * sats as u128);
    let digits = format!("{scaled:013}");
    let (whole, fraction) = digits.split_at(digits.len() - 12);
    format!("{whole}.{fraction}")
}

fn exchange(b: &BusinessData, sats: i64, cents: i64, buy: bool) -> TransferInput {
    let wallet = account(b, "Lightning wallet");
    let (from, to) = if buy {
        (BANK.to_string(), wallet)
    } else {
        (wallet, BANK.to_string())
    };
    TransferInput {
        date: "2027-01-10".into(),
        amount_minor: minor(sats),
        currency: Currency::Btc,
        from_account_id: from,
        to_account_id: to,
        transfer_type: TransferType::Exchange,
        purpose: "Invented purpose".into(),
        reference: None,
        document_ids: Vec::new(),
        fiat_value: Some(FiatValue {
            currency: Currency::Usd,
            amount_minor: minor(cents),
            rate: implied_rate(sats, cents),
            source: FiatSource::Exchange,
            at: "2027-01-10T09:00Z".into(),
        }),
        millisats: None,
    }
}

fn between_wallets(b: &BusinessData, sats: i64) -> TransferInput {
    TransferInput {
        date: "2027-01-10".into(),
        amount_minor: minor(sats),
        currency: Currency::Btc,
        from_account_id: account(b, "Cold storage"),
        to_account_id: account(b, "Lightning wallet"),
        transfer_type: TransferType::BetweenAccounts,
        purpose: "Invented purpose".into(),
        reference: None,
        document_ids: Vec::new(),
        fiat_value: Some(valued(sats)),
        millisats: None,
    }
}

#[test]
fn sats_parse_and_display_exactly_at_every_precision() {
    for places in 0..=8u32 {
        for k in 0..200i64 {
            let whole = (k * 104_729 + i64::from(places) * 7) % 21_000_001;
            let fraction = (k * 7_919) % 10i64.pow(places);
            let text = if places == 0 {
                whole.to_string()
            } else {
                format!("{whole}.{fraction:0width$}", width = places as usize)
            };
            let sats = whole * 100_000_000 + fraction * 10i64.pow(8 - places);
            assert_eq!(parse_money(&text, Currency::Btc), Ok(minor(sats)), "{text}");
            let shown = amount_input(minor(sats), Currency::Btc);
            assert_eq!(parse_money(&shown, Currency::Btc), Ok(minor(sats)));
            assert_eq!(parse_sats(&sats.to_string()), Ok(minor(sats)));
        }
    }
    assert_eq!(parse_money("0.00000001", Currency::Btc), Ok(minor(1)));
    assert_eq!(
        parse_money("0.000000001", Currency::Btc)
            .unwrap_err()
            .to_string(),
        "Use an amount with up to eight decimal places, such as 0.00012345."
    );
    assert_eq!(amount_input(minor(1), Currency::Btc), "0.00000001");
    assert_eq!(
        amount_input(minor(-123_456_789), Currency::Btc),
        "-1.23456789"
    );
}

#[test]
fn the_largest_valid_btc_amount_fits_the_range_limit() {
    let all = 21_000_000 * 100_000_000;
    assert!(all <= MAX_MINOR);
    assert_eq!(parse_money("21000000", Currency::Btc), Ok(minor(all)));
    assert_eq!(
        parse_money("90071992.54740991", Currency::Btc),
        Ok(minor(MAX_MINOR))
    );
    assert_eq!(
        amount_input(minor(MAX_MINOR), Currency::Btc),
        "90071992.54740991"
    );
    assert_eq!(
        parse_money("90071992.54740992", Currency::Btc),
        Err(LedgerError::AmountTooLarge)
    );
    let (b, mut env) = setup();
    let posted = record_transfer(&b, &between_wallets(&b, all), &mut env).unwrap();
    assert_eq!(
        balance(&posted, Currency::Btc, &account(&b, "Lightning wallet")),
        all
    );
}

#[test]
fn a_btc_journal_without_a_fiat_value_or_with_one_its_rate_doesnt_give_is_refused() {
    let (b, mut env) = setup();
    assert_eq!(
        expense(&b, &mut env, Currency::Btc, 21_000, None)
            .unwrap_err()
            .to_string(),
        "A bitcoin entry needs its fiat value at the time of the transaction."
    );
    let mut off = valued(21_000);
    off.amount_minor = minor(1_262);
    assert_eq!(
        expense(&b, &mut env, Currency::Btc, 21_000, Some(off))
            .unwrap_err()
            .to_string(),
        "The fiat value must be the sats times the rate, within one minor unit."
    );
    let posted = expense(&b, &mut env, Currency::Btc, 21_000, Some(valued(21_000))).unwrap();
    let journal = posted.journals.last().unwrap().to_unchecked();
    assert_eq!(journal.fiat_value, Some(valued(21_000)));
    let mut bare = journal.clone();
    bare.fiat_value = None;
    assert_eq!(
        JournalEntry::new(&posted, bare).unwrap_err(),
        LedgerError::InvalidJournalFiatValue
    );
    let mut wrong = journal;
    wrong.fiat_value.as_mut().unwrap().amount_minor = minor(9);
    assert_eq!(
        JournalEntry::new(&posted, wrong).unwrap_err(),
        LedgerError::InvalidJournalFiatValue
    );
    // Dollars refuse one.
    assert_eq!(
        expense(&b, &mut env, Currency::Usd, 4_200, Some(valued(4_200)))
            .unwrap_err()
            .to_string(),
        "Only a bitcoin entry records a fiat value."
    );
    // A transfer and an opening balance need one too.
    let mut unvalued = between_wallets(&b, 100_000);
    unvalued.fiat_value = None;
    assert!(record_transfer(&b, &unvalued, &mut env).is_err());
    let opening = OpeningBalanceInput {
        account_id: account(&b, "Cold storage"),
        date: "2027-01-01".into(),
        amount_minor: minor(100_000),
        reference: None,
        fiat_value: None,
    };
    assert!(record_opening_balance(&b, &opening, &mut env).is_err());
}

#[test]
fn within_one_minor_unit_means_at_most_one_cent_either_way() {
    for (cents, ok) in [
        (6000, true),
        (6001, true),
        (5999, true),
        (6002, false),
        (5998, false),
    ] {
        let mut v = valued(100_000);
        v.amount_minor = minor(cents);
        assert_eq!(fiat_value_matches(minor(100_000), &v), ok, "{cents}");
    }
}

#[test]
fn an_exchange_balances_on_both_sides_and_never_counts_as_spending() {
    let (b, mut env) = setup();
    let wallet = account(&b, "Lightning wallet");
    let bought = record_transfer(&b, &exchange(&b, 250_000, 15_000, true), &mut env).unwrap();
    assert_eq!(balance(&bought, Currency::Usd, BANK), -15_000);
    assert_eq!(balance(&bought, Currency::Btc, &wallet), 250_000);
    let sold = record_transfer(&bought, &exchange(&b, 50_000, 3_100, false), &mut env).unwrap();
    assert_eq!(balance(&sold, Currency::Usd, BANK), -11_900);
    assert_eq!(balance(&sold, Currency::Btc, &wallet), 200_000);
    for period in ["", "2027-01"] {
        assert!(totals(&sold, period).unwrap().is_empty());
        assert!(recorded_fiat_spending(&sold, period).unwrap().is_empty());
        assert!(category_totals(&sold, period, Currency::Btc)
            .unwrap()
            .is_empty());
        assert!(category_totals(&sold, period, Currency::Usd)
            .unwrap()
            .is_empty());
    }
    let original = sold
        .journals
        .iter()
        .find(|j| j.transfer_type == Some(TransferType::Exchange))
        .unwrap()
        .id
        .clone();
    let fixed = correct_transfer(
        &sold,
        &original,
        &exchange(&b, 200_000, 12_000, true),
        "Invented reason",
        &mut env,
    )
    .unwrap();
    assert_eq!(balance(&fixed, Currency::Usd, BANK), -8_900);
    assert_eq!(balance(&fixed, Currency::Btc, &wallet), 150_000);
}

#[test]
fn millisats_round_to_the_nearest_sat_and_are_kept() {
    let rounded: Vec<i64> = [1, 499, 500, 999, 1000, 1499, 1500, 2500]
        .into_iter()
        .map(|m| sats_from_millisats(minor(m)).get())
        .collect();
    assert_eq!(rounded, [0, 0, 1, 1, 1, 1, 2, 3]);
    assert_eq!(
        sats_from_millisats(minor(MAX_MINOR)),
        minor(9_007_199_254_741)
    );
    let (b, mut env) = setup();
    let mut input = between_wallets(&b, 100_000);
    input.millisats = Some(minor(100_000_499));
    let posted = record_transfer(&b, &input, &mut env).unwrap();
    assert_eq!(
        posted.journals.last().unwrap().millisats,
        Some(minor(100_000_499))
    );
    input.millisats = Some(minor(100_000_500));
    assert_eq!(
        record_transfer(&b, &input, &mut env)
            .unwrap_err()
            .to_string(),
        "The amount in sats must be the millisats rounded to the nearest sat."
    );
}

#[test]
fn existing_currencies_are_unchanged() {
    /// The two-decimal `parseMoney` and `amountInput` before precision.
    fn legacy_input(n: i64) -> String {
        let sign = if n < 0 { "-" } else { "" };
        let abs = n.unsigned_abs();
        format!("{sign}{}.{:02}", abs / 100, abs % 100)
    }
    for currency in Currency::FIAT {
        assert_eq!(currency.precision(), 2);
        for n in [0, 1, -1, 99, 100, -405, 123_456, MAX_MINOR, -MAX_MINOR] {
            assert_eq!(amount_input(minor(n), currency), legacy_input(n));
            assert_eq!(parse_money(&legacy_input(n), currency), Ok(minor(n)), "{n}");
        }
        for bad in ["1.001", "1e4", "", "1,000", "0.00000001"] {
            assert_eq!(
                parse_money(bad, currency).unwrap_err().to_string(),
                "Use an amount with up to two decimal places, such as 42.50."
            );
        }
    }
}

proptest! {
    /// Random dollar expenses, bitcoin expenses and exchanges: every report
    /// keeps sats and dollars apart.
    #[test]
    fn no_report_ever_adds_sats_to_dollars(
        steps in prop::collection::vec((0u8..3, 1i64..5_000_000, 1i64..500_000), 1..8)
    ) {
        let (mut b, mut env) = setup();
        let wallet = account(&b, "Lightning wallet");
        let (mut usd_spent, mut sats_spent, mut value_spent) = (0, 0, 0);
        let (mut usd_held, mut sats_held) = (0, 0);
        for (kind, sats, cents) in steps {
            match kind {
                0 => {
                    b = expense(&b, &mut env, Currency::Usd, cents, None).unwrap();
                    usd_spent += cents;
                    usd_held -= cents;
                }
                1 => {
                    let value = valued(sats);
                    value_spent += value.amount_minor.get();
                    b = expense(&b, &mut env, Currency::Btc, sats, Some(value)).unwrap();
                    sats_spent += sats;
                    sats_held -= sats;
                }
                _ => {
                    b = record_transfer(&b, &exchange(&b, sats, cents, true), &mut env).unwrap();
                    usd_held -= cents;
                    sats_held += sats;
                }
            }
        }
        let spent = totals(&b, "").unwrap();
        prop_assert_eq!(spent.get(&Currency::Usd).map_or(0, |m| m.get()), usd_spent);
        prop_assert_eq!(spent.get(&Currency::Btc).map_or(0, |m| m.get()), sats_spent);
        let value = recorded_fiat_spending(&b, "").unwrap();
        prop_assert_eq!(value.get(&Currency::Usd).map_or(0, |m| m.get()), value_spent);
        prop_assert!(value.keys().all(|c| c.is_fiat()));
        prop_assert_eq!(balance(&b, Currency::Usd, BANK), usd_held);
        prop_assert_eq!(balance(&b, Currency::Btc, &wallet), sats_held);
        for currency in [Currency::Usd, Currency::Btc] {
            let rows = balances(&b, "2027-12-31", currency).unwrap();
            let same = rows.iter().all(|r| {
                b.accounts.iter().find(|a| a.id == r.account_id).unwrap().currency == currency
            });
            prop_assert!(same);
        }
        let sum = |c| -> i64 {
            category_totals(&b, "", c).unwrap().iter().map(|r| r.amount.get()).sum()
        };
        prop_assert_eq!(sum(Currency::Usd), usd_spent);
        prop_assert_eq!(sum(Currency::Btc), sats_spent);
    }
}
