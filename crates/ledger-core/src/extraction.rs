//! Receipt-reading outcomes: `outcomesFor` and `finalizeExtractions` from
//! `src/domain/extraction.ts`. Approving a draft judges each of its pending
//! readings against what was posted.

use std::collections::BTreeMap;

use crate::types::{
    BusinessData, ExpenseDraft, ExpenseStatus, ExtractedField, ExtractionFieldName,
    ExtractionOutcome, ExtractionRecord, FieldStatus,
};

/// `outcomesFor` for one field: `unclear`, or for a found value `ignored`
/// unless accepted, then `accepted` if the draft kept it and `edited` if not.
/// An absent field has no outcome.
fn outcome<T: PartialEq>(
    record: &ExtractionRecord,
    name: ExtractionFieldName,
    field: &ExtractedField<T>,
    posted: &T,
) -> Option<ExtractionOutcome> {
    match field.status {
        FieldStatus::Unclear => Some(ExtractionOutcome::Unclear),
        FieldStatus::Absent => None,
        FieldStatus::Found if !record.accepted.contains(&name) => Some(ExtractionOutcome::Ignored),
        // TS `===`: a found field with a null value never equals the draft's.
        FieldStatus::Found if field.value.as_ref() == Some(posted) => {
            Some(ExtractionOutcome::Accepted)
        }
        FieldStatus::Found => Some(ExtractionOutcome::Edited),
    }
}

/// `outcomesFor`: the outcome of each applicable field (vendor, date, total,
/// currency, category), judged against the final draft.
pub fn outcomes_for(
    record: &ExtractionRecord,
    draft: &ExpenseDraft,
) -> BTreeMap<ExtractionFieldName, ExtractionOutcome> {
    use ExtractionFieldName as F;
    let f = &record.fields;
    [
        (
            F::Vendor,
            outcome(record, F::Vendor, &f.vendor, &draft.vendor),
        ),
        (F::Date, outcome(record, F::Date, &f.date, &draft.date)),
        (
            F::Total,
            outcome(record, F::Total, &f.total, &draft.amount_minor),
        ),
        (
            F::Currency,
            outcome(record, F::Currency, &f.currency, &draft.currency),
        ),
        (
            F::Category,
            outcome(record, F::Category, &f.category, &draft.category_id),
        ),
    ]
    .into_iter()
    .filter_map(|(name, o)| o.map(|o| (name, o)))
    .collect()
}

/// `finalizeExtractions`: once the draft `draft_id` is posted, sets the
/// outcomes of its pending readings. Anything else is left alone.
pub(crate) fn finalize_extractions(b: &mut BusinessData, draft_id: &str) {
    let Some(draft) = b
        .drafts
        .iter()
        .find(|d| d.id == draft_id && d.business_id == b.profile.id)
        .filter(|d| d.status() == ExpenseStatus::Posted)
        .cloned()
    else {
        return;
    };
    for x in &mut b.extractions {
        if x.draft_id == draft_id && x.outcomes.is_none() {
            x.outcomes = Some(outcomes_for(x, &draft));
        }
    }
}
