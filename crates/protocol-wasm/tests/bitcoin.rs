//! Bitcoin commands through the WASM bindings: a proposal carries its
//! `fiatValue` and `millisats` exactly as the browser sends them, and a
//! malformed one is refused before anything is signed.

use protocol_wasm::api;
use serde_json::{json, Value};

const BUSINESS: &str = "abababababababababababababababababababababababababababababababab";

fn exchange() -> Value {
    json!({
        "type": "RecordTransfer",
        "transfer": {
            "date": "2026-10-20",
            "amountMinor": 250000,
            "currency": "BTC",
            "fromAccountId": "bank",
            "toAccountId": "wallet",
            "transferType": "exchange",
            "purpose": "Invented purpose",
            "fiatValue": {
                "currency": "USD",
                "amountMinor": 15000,
                "rate": "60000.00",
                "source": "exchange",
                "at": "2026-10-20T09:00Z"
            },
            "millisats": 250000400
        }
    })
}

fn build(command: Value) -> api::Answer {
    api::build_proposal(
        &json!({ "business": BUSINESS, "baseSeq": 3, "command": command }).to_string(),
    )
}

#[test]
fn a_proposal_carries_the_fiat_value_and_millisats() {
    let parts: Value = serde_json::from_str(&build(exchange()).unwrap()).unwrap();
    let payload: Value = serde_json::from_str(parts["plaintext"].as_str().unwrap()).unwrap();
    assert_eq!(payload["command"], exchange());
}

#[test]
fn a_fiat_value_that_cant_be_held_is_refused_before_signing() {
    for (field, bad) in [
        ("rate", json!(60000)),
        ("source", json!("guess")),
        ("amountMinor", json!(1.5)),
    ] {
        let mut command = exchange();
        command["transfer"]["fiatValue"][field] = bad;
        assert_eq!(
            build(command).unwrap_err(),
            "The request is not valid for this function.",
            "{field}"
        );
    }
}
