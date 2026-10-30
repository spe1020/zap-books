//! WebAssembly bindings over `crates/protocol`, for the browser app
//! (`docs/briefs/stage-2-pr4-browser.md`). The browser verifies the chain,
//! opens acceptances and follows membership with exactly the code the
//! service runs; none of it is written again in TypeScript.
//!
//! Member keys stay in the NIP-07 signer extension, so the browser's part of
//! encryption and signing happens there: [`api::build_proposal`] gives the
//! plaintext and tags for the extension to encrypt and sign, and
//! [`api::open_delivery`] reads a delivery the extension decrypted.
//!
//! Each export takes one JSON request as text and answers JSON text, or
//! throws an `Error` with the message. Build with
//! `wasm-pack build crates/protocol-wasm --target web`.
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

pub mod api;

#[cfg(target_arch = "wasm32")]
mod exports {
    use wasm_bindgen::prelude::{wasm_bindgen, JsError};

    use crate::api;

    fn js(answer: api::Answer) -> Result<String, JsError> {
        answer.map_err(|message| JsError::new(&message))
    }

    #[wasm_bindgen(js_name = buildProposal)]
    pub fn build_proposal(request: &str) -> Result<String, JsError> {
        js(api::build_proposal(request))
    }

    #[wasm_bindgen(js_name = checkSignedProposal)]
    pub fn check_signed_proposal(request: &str) -> Result<String, JsError> {
        js(api::check_signed_proposal(request))
    }

    #[wasm_bindgen(js_name = verifyChain)]
    pub fn verify_chain(request: &str) -> Result<String, JsError> {
        js(api::verify_chain(request))
    }

    #[wasm_bindgen(js_name = openAcceptance)]
    pub fn open_acceptance(request: &str) -> Result<String, JsError> {
        js(api::open_acceptance(request))
    }

    #[wasm_bindgen(js_name = followMembership)]
    pub fn follow_membership(request: &str) -> Result<String, JsError> {
        js(api::follow_membership(request))
    }

    #[wasm_bindgen(js_name = applyRecordChanges)]
    pub fn apply_record_changes(request: &str) -> Result<String, JsError> {
        js(api::apply_record_changes(request))
    }

    #[wasm_bindgen(js_name = parseDelivery)]
    pub fn parse_delivery(request: &str) -> Result<String, JsError> {
        js(api::parse_delivery(request))
    }

    #[wasm_bindgen(js_name = openDelivery)]
    pub fn open_delivery(request: &str) -> Result<String, JsError> {
        js(api::open_delivery(request))
    }

    #[wasm_bindgen(js_name = generateRecoveryKeypair)]
    pub fn generate_recovery_keypair() -> Result<String, JsError> {
        js(api::generate_recovery_keypair())
    }
}
